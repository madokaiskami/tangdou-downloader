#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod gui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([680.0, 780.0])
            .with_min_inner_size([540.0, 580.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Tangdou Downloader",
        options,
        Box::new(|creation_context| Ok(Box::new(gui::TangdouApp::new(creation_context)))),
    )
}
