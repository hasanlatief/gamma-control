mod app;
mod icc;
mod kscreen;
mod profiles;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        // Start smaller than the UI: separators fill the available width, so a
        // larger first frame would inflate the size the app measures and fits to.
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([100.0, 100.0]),
        ..Default::default()
    };
    eframe::run_native("Gamma Control", options, Box::new(|_| Ok(Box::new(app::App::new()))))
}
