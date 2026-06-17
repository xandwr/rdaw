//! The transport bar: play/pause, stop, position readout, and zoom.

use eframe::egui;
use rdaw_engine::Command;

use crate::app::DawApp;

/// Draw the transport bar into `ui`, reading and mutating `app` state.
pub fn bar(app: &mut DawApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let play_label = if app.playing { "⏸ Pause" } else { "▶ Play" };
        if ui.button(play_label).clicked() {
            app.toggle_play_pause();
        }
        if ui.button("⏹ Stop").clicked() {
            app.send(Command::Stop);
            app.playing = false;
        }

        ui.separator();
        let pos = app.playhead_secs();
        ui.monospace(format!("{:02}:{:06.3}", (pos / 60.0) as u32, pos % 60.0));

        ui.separator();
        ui.label("zoom");
        ui.add(egui::Slider::new(&mut app.px_per_sec, 20.0..=400.0).suffix(" px/s"));
    });
}
