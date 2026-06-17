//! The application state ([`DawApp`]) and top-level layout. The actual widgets
//! live in [`crate::transport`] and [`crate::timeline`]; this module owns the
//! state they read/write and wires them into the egui frame.

use std::sync::Arc;

use eframe::egui;
use rdaw_core::{Project, Waveform};
use rdaw_engine::{Command, Engine};

use crate::{demo, timeline, transport, SR};

pub struct DawApp {
    /// The arrangement document — source of truth on the UI side.
    pub project: Project,
    /// Decoded sources, indexed the same as `project.sources`. Held here so the
    /// next step (rebuild-on-edit) can re-`build_graph` after a clip change.
    #[allow(dead_code)]
    pub sources: Vec<Arc<Waveform>>,
    /// The running audio engine, or `None` if the output device failed to open.
    pub engine: Option<Engine>,
    /// Last error surfaced to the user (device open, etc.).
    pub error: Option<String>,
    /// Whether we *think* we're playing — drives the button label and repaint.
    /// The engine is authoritative for position; this is just UI intent.
    pub playing: bool,
    /// Horizontal zoom: pixels per second of timeline.
    pub px_per_sec: f32,
}

impl DawApp {
    pub fn new() -> Self {
        let (project, sources) = demo::project();

        // Start the engine. If there's no output device (headless, etc.) we
        // still show the UI and report the error rather than crashing.
        let (engine, error) = match Engine::new(project.build_graph(2, SR, &sources)) {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(format!("audio engine: {e}"))),
        };

        Self {
            project,
            sources,
            engine,
            error,
            playing: false,
            px_per_sec: 120.0,
        }
    }

    /// Current play-head position in seconds, polled from the engine.
    pub fn playhead_secs(&self) -> f64 {
        match &self.engine {
            Some(e) => e.playhead() as f64 / SR,
            None => 0.0,
        }
    }

    pub fn send(&mut self, cmd: Command) {
        if let Some(e) = &mut self.engine {
            e.send(cmd);
        }
    }

    /// Start or pause-in-place, mirroring the transport button. Bound to Space.
    pub fn toggle_play_pause(&mut self) {
        if self.playing {
            self.send(Command::Pause);
            self.playing = false;
        } else {
            self.send(Command::Play);
            self.playing = true;
        }
    }
}

impl Default for DawApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for DawApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Space toggles play/pause. Consume it so a focused button doesn't also
        // get activated by the same press (which would toggle twice).
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            self.toggle_play_pause();
        }

        egui::Panel::top("transport").show_inside(ui, |ui| {
            transport::bar(self, ui);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
                ui.label("(UI is shown; playback is disabled without an output device)");
            }
            timeline::draw(self, ui);
        });

        // Keep the play head animating while we believe playback is running.
        if self.playing {
            ctx.request_repaint();
        }
    }
}
