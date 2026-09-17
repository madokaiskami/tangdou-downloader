mod gui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([600.0, 590.0])
            .with_min_inner_size([480.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Tangdou Downloader",
        options,
        Box::new(|creation_context| Ok(Box::new(gui::TangdouApp::new(creation_context)))),
    )
}
