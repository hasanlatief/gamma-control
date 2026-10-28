use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::icc;
use crate::kscreen::{self, ColorState, Output};
use crate::profiles::{self, Profiles, Settings};

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
        let settings = self.settings;
        let name = output.name.clone();
        let handle = std::thread::spawn(move || {
            std::fs::create_dir_all(&dir)?;
            icc::write_profile(path.as_ref(), &settings)?;
            kscreen::set_color(&name, &ColorState { source: "ICC".into(), icc_path: path })?;
            kscreen::list_outputs()
        });
        self.pending = Some(Pending {
            output: output.name,
            previous: output.color,
            previous_settings: self.applied,
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
                ui.add_enabled_ui(!locked, |ui| {
                    egui::ComboBox::from_label("Output")
                        .selected_text(self.outputs.get(self.selected).map_or("none", |o| &o.name))
                        .show_index(ui, &mut self.selected, self.outputs.len(), |i| &self.outputs[i].name);

                    ui.separator();
                    ui.spacing_mut().slider_width = 300.0;
                    let s = &mut self.settings;
                    ui.add(egui::Slider::new(&mut s.brightness, icc::BRIGHTNESS_RANGE).text("Brightness").fixed_decimals(2).step_by(0.01));
                    ui.add(egui::Slider::new(&mut s.gamma, icc::GAMMA_RANGE).text("Gamma").fixed_decimals(2).step_by(0.01));
                    ui.add(egui::Slider::new(&mut s.contrast, icc::CONTRAST_RANGE).text("Contrast").fixed_decimals(2).step_by(0.01));
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
                        egui::ComboBox::from_id_salt("profile")
                            .selected_text("Load profile")
                            .show_ui(ui, |ui| {
                                for (name, settings) in &self.profiles {
                                    if ui.selectable_label(*name == self.profile_name, name).clicked() {
                                        self.profile_name = name.clone();
                                        self.settings = *settings;
                                    }
                                }
                            });
                        ui.add(egui::TextEdit::singleline(&mut self.profile_name).hint_text("Profile name").desired_width(140.0));
                        let named = !self.profile_name.trim().is_empty();
                        if ui.add_enabled(named, egui::Button::new("Save")).clicked() {
                            self.profiles.insert(self.profile_name.trim().to_owned(), self.settings);
                            self.status = profiles::save(&self.profiles).err().map(|e| format!("{e:#}")).unwrap_or_default();
                        }
                        if ui.add_enabled(self.profiles.contains_key(&self.profile_name), egui::Button::new("Delete")).clicked() {
                            self.profiles.remove(&self.profile_name);
                            self.status = profiles::save(&self.profiles).err().map(|e| format!("{e:#}")).unwrap_or_default();
                        }
                    });
                });

                // Laid out even when empty so the startup size leaves room for it.
                ui.separator();
                ui.label(&self.status);
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
                    self.applied = self.settings;
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
