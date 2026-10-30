use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::icc;
use crate::kscreen::{self, ColorState, Output};
use crate::profiles::{self, Profiles, Settings, Tab};

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);

struct Pending {
    output: String,
    previous: ColorState,
    previous_settings: Settings,
    deadline: Instant,
    focused: bool,
}

enum Job {
    Apply,
    Revert(Settings),
}

pub struct App {
    outputs: Vec<Output>,
    selected: usize,
    settings: Settings,
    applied: Settings,
    profiles: Profiles,
    profile_name: String,
    pending: Option<Pending>,
    job: Option<(Job, JoinHandle<anyhow::Result<Vec<Output>>>)>,
    status: String,
    fitted: bool,
    /// Curve point index and value under the pointer last frame, to fill gaps in fast strokes.
    last_draw: Option<(usize, f32)>,
    // Sizes measured last frame. egui lays out in one pass, so growing a widget
    // to fill the window needs to know how much space its neighbours take.
    tabs_width: f32,
    slider_extra: f32,
    profile_extra: f32,
    bottom_height: f32,
}

impl App {
    pub fn new() -> Self {
        let (outputs, status) = match kscreen::list_outputs() {
            Ok(o) => (o, String::new()),
            Err(e) => (Vec::new(), format!("{e:#}")),
        };
        Self {
            outputs,
            selected: 0,
            settings: Settings::default(),
            applied: Settings::default(),
            profiles: profiles::load(),
            profile_name: String::new(),
            pending: None,
            job: None,
            status,
            fitted: false,
            last_draw: None,
            tabs_width: 0.0,
            slider_extra: 0.0,
            profile_extra: 0.0,
            bottom_height: 0.0,
        }
    }

    fn apply(&mut self) {
        let Some(output) = self.outputs.get(self.selected).cloned() else { return };
        let dir = dirs::data_dir().unwrap_or_default().join("icc");
        // Alternate file names so KWin always sees a path change.
        let path = ["a", "b"]
            .map(|s| dir.join(format!("gamma_control_{}_{s}.icc", output.name)).display().to_string())
            .into_iter()
            .find(|p| *p != output.color.icc_path)
            .unwrap();
        let curve = self.settings.curve.clone();
        let name = output.name.clone();
        let handle = std::thread::spawn(move || {
            std::fs::create_dir_all(&dir)?;
            icc::write_profile(path.as_ref(), &curve)?;
            kscreen::set_color(&name, &ColorState { source: "ICC".into(), icc_path: path })?;
            kscreen::list_outputs()
        });
        self.pending = Some(Pending {
            output: output.name,
            previous: output.color,
            previous_settings: self.applied.clone(),
            deadline: Instant::now() + CONFIRM_TIMEOUT,
            focused: false,
        });
        self.job = Some((Job::Apply, handle));
    }

    fn revert(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let handle = std::thread::spawn(move || {
            kscreen::set_color(&p.output, &p.previous)?;
            kscreen::list_outputs()
        });
        self.job = Some((Job::Revert(p.previous_settings), handle));
    }

    fn poll_job(&mut self) {
        if !self.job.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            return;
        }
        let (job, handle) = self.job.take().unwrap();
        match handle.join().unwrap() {
            Ok(outputs) => {
                self.outputs = outputs;
                self.status.clear();
                match job {
                    Job::Apply => {
                        if let Some(p) = &mut self.pending {
                            p.deadline = Instant::now() + CONFIRM_TIMEOUT;
                        }
                    }
                    Job::Revert(s) => {
                        self.settings = s;
                        self.status = "Reverted to previous settings.".into();
                    }
                }
            }
            Err(e) => {
                self.status = format!("{e:#}");
                if matches!(job, Job::Apply) {
                    self.revert();
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_job();
        let busy = self.job.is_some();
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        } else if let Some(p) = &self.pending {
            if Instant::now() >= p.deadline {
                self.revert();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }

        let content_size = egui::CentralPanel::default_margins().show(ui, |ui| {
            let rect = ui.scope(|ui| {
                let locked = busy || self.pending.is_some();
                let bottom_top = ui.add_enabled_ui(!locked, |ui| {
                    egui::ComboBox::from_label("Output")
                        .selected_text(self.outputs.get(self.selected).map_or("none", |o| &o.name))
                        .show_index(ui, &mut self.selected, self.outputs.len(), |i| &self.outputs[i].name);

                    ui.separator();
                    let s = &mut self.settings;
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - self.tabs_width) / 2.0).max(0.0));
                        self.tabs_width = ui
                            .horizontal(|ui| {
                                ui.selectable_value(&mut s.tab, Tab::Sliders, "Brightness / Gamma / Contrast");
                                ui.selectable_value(&mut s.tab, Tab::Levels, "Black / White point");
                                ui.selectable_value(&mut s.tab, Tab::Draw, "Draw");
                            })
                            .response
                            .rect
                            .width();
                    });
                    // Until the startup fit settles, use minimum sizes: filling the window
                    // while measuring it feeds any layout error back into the size, growing it forever.
                    let grow = self.fitted;
                    let slider_width = if grow { (ui.available_width() - self.slider_extra).max(300.0) } else { 300.0 };
                    ui.spacing_mut().slider_width = slider_width;
                    // While sizing the window at startup, reserve the tallest tab's height
                    // (three slider rows); afterwards the curve takes whatever the sliders leave.
                    let rows = 3.0 * ui.spacing().interact_size.y + 2.0 * ui.spacing().item_spacing.y;
                    let sliders = ui.vertical(|ui| {
                        if !grow {
                            ui.set_min_height(rows);
                        }
                        match s.tab {
                        Tab::Sliders => {
                            let sl = &mut s.sliders;
                            ui.add(egui::Slider::new(&mut sl.brightness, icc::BRIGHTNESS_RANGE).text("Brightness").fixed_decimals(2).step_by(0.01));
                            ui.add(egui::Slider::new(&mut sl.gamma, icc::GAMMA_RANGE).text("Gamma").fixed_decimals(2).step_by(0.01));
                            ui.add(egui::Slider::new(&mut sl.contrast, icc::CONTRAST_RANGE).text("Contrast").fixed_decimals(2).step_by(0.01));
                            s.curve = icc::sliders_curve(sl);
                        }
                        Tab::Levels => {
                            let l = &mut s.levels;
                            ui.add(egui::Slider::new(&mut l.black, 0.0..=l.white - 0.01).text("Black point").fixed_decimals(2).step_by(0.01));
                            ui.add(egui::Slider::new(&mut l.white, l.black + 0.01..=1.0).text("White point").fixed_decimals(2).step_by(0.01));
                            s.curve = icc::levels_curve(l);
                        }
                        Tab::Draw => {}
                        }
                    });
                    if s.tab != Tab::Draw {
                        self.slider_extra = sliders.response.rect.width() - slider_width;
                    }

                    // The curve fills the space above the bottom rows, square, and sits right on top of them.
                    let gap = ui.spacing().item_spacing.y;
                    let free = egui::vec2(ui.available_width(), ui.available_height() - gap - self.bottom_height);
                    let size = if grow { free.min_elem().max(300.0) } else { 300.0 };
                    if grow {
                        ui.add_space((free.y - size).max(0.0));
                    }

                    let (response, painter) = ui.vertical_centered(|ui| ui.allocate_painter(egui::vec2(size, size), egui::Sense::drag())).inner;
                    let rect = response.rect;
                    let to_screen = |x: f32, y: f32| egui::pos2(rect.left() + x * rect.width(), rect.bottom() - y * rect.height());
                    painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
                    painter.line_segment([to_screen(0.0, 0.0), to_screen(1.0, 1.0)], ui.visuals().widgets.noninteractive.bg_stroke);
                    let line = (0..=128).map(|i| i as f32 / 128.0).map(|x| to_screen(x, icc::eval(&s.curve, x))).collect();
                    painter.add(egui::Shape::line(line, ui.visuals().widgets.active.fg_stroke));
                    if s.tab == Tab::Draw
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        let curve = &mut s.curve;
                        let n = (icc::LEVELS - 1) as f32;
                        let i = (((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * n).round() as usize;
                        let y = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
                        let (pi, py) = self.last_draw.unwrap_or((i, y));
                        for j in pi.min(i)..=pi.max(i) {
                            let t = if i == pi { 1.0 } else { (j as f32 - pi as f32) / (i as f32 - pi as f32) };
                            curve[j] = py + (y - py) * t;
                        }
                        self.last_draw = Some((i, y));
                    } else {
                        self.last_draw = None;
                    }
                    let bottom_top = ui.cursor().top();
                    ui.horizontal(|ui| {
                        if ui.button("Apply").clicked() {
                            self.apply();
                        }
                        if ui.button("Reset to default").clicked() {
                            self.settings = Settings::default();
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        let left = ui.cursor().left();
                        let row_width = ui.available_width();
                        egui::ComboBox::from_id_salt("profile")
                            .selected_text("Load profile")
                            .show_ui(ui, |ui| {
                                for (name, settings) in &self.profiles {
                                    if ui.selectable_label(*name == self.profile_name, name).clicked() {
                                        self.profile_name = name.clone();
                                        self.settings = settings.clone();
                                    }
                                }
                            });
                        let name_width = if grow { (row_width - self.profile_extra).max(140.0) } else { 140.0 };
                        let name = ui.add(egui::TextEdit::singleline(&mut self.profile_name).hint_text("Profile name").desired_width(name_width));
                        let named = !self.profile_name.trim().is_empty();
                        if ui.add_enabled(named, egui::Button::new("Save")).clicked() {
                            self.profiles.insert(self.profile_name.trim().to_owned(), self.settings.clone());
                            self.status = profiles::save(&self.profiles).err().map(|e| format!("{e:#}")).unwrap_or_default();
                        }
                        let delete = ui.add_enabled(self.profiles.contains_key(&self.profile_name), egui::Button::new("Delete"));
                        if delete.clicked() {
                            self.profiles.remove(&self.profile_name);
                            self.status = profiles::save(&self.profiles).err().map(|e| format!("{e:#}")).unwrap_or_default();
                        }
                        self.profile_extra = delete.rect.right() - left - name.rect.width();
                    });
                    bottom_top
                });

                // Laid out even when empty so the startup size leaves room for it.
                ui.separator();
                ui.label(&self.status);
                self.bottom_height = ui.min_rect().bottom() - bottom_top.inner;
            })
            .response
            .rect;

            // The panel margin sits on both sides of the content.
            rect.max.to_vec2() + rect.min.to_vec2()
        });

        // Fit the window to the UI once at startup. Requests sent before the
        // window is configured are dropped, so repeat until it sticks.
        if !self.fitted {
            let size = content_size.inner;
            if (ui.ctx().viewport_rect().size() - size).length() < 1.0 {
                self.fitted = true;
                // The user may grow the window but not shrink it below what the UI needs.
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
            } else {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        }

        if let Some(p) = &self.pending
            && !busy
        {
            let left = p.deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            let first_frame = !p.focused;
            let builder = egui::ViewportBuilder::default()
                .with_title("Keep these settings?")
                .with_inner_size([260.0, 70.0])
                .with_resizable(false);
            let keep = ui.ctx().show_viewport_immediate(egui::ViewportId::from_hash_of("confirm"), builder, |ui, _| {
                // KWin may open the popup behind the main window.
                if first_frame {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                egui::CentralPanel::default_margins()
                    .show(ui, |ui| {
                        ui.label(format!("Reverting in {left} s."));
                        ui.horizontal(|ui| {
                            if ui.button("Keep").clicked() {
                                Some(true)
                            } else if ui.button("Revert").clicked() || ui.input(|i| i.viewport().close_requested()) {
                                Some(false)
                            } else {
                                None
                            }
                        })
                        .inner
                    })
                    .inner
            });
            match keep {
                Some(true) => {
                    self.applied = self.settings.clone();
                    self.pending = None;
                    self.status = "Settings kept.".into();
                }
                Some(false) => self.revert(),
                None => self.pending.as_mut().unwrap().focused = true,
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(p) = self.pending.take() {
            let _ = kscreen::set_color(&p.output, &p.previous);
        }
    }
}
