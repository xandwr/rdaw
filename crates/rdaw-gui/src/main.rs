//! Entry point for the egui front-end. All the real logic lives in the library
//! ([`rdaw_gui`]); this just opens a native window and hands it [`DawApp`].

use eframe::egui;
use rdaw_gui::app::DawApp;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rdaw",
        native_options,
        Box::new(|_cc| Ok(Box::new(DawApp::new()))),
    )
}
