//! Minimal egui front-end for the DAW. This is the MVP scaffold: a transport
//! bar (play/pause/stop), a timeline that draws each clip as a rectangle, and a
//! play head that animates by polling [`Engine::playhead`] every frame.
//!
//! The audio side is unchanged — this only *drives* the existing engine and
//! *reads back* its play position. Editing (drag/drop, move) is the next step.

use std::f64::consts::TAU;
use std::sync::Arc;

use eframe::egui;
use rdaw_core::{ClipData, Project, Track, Waveform};
use rdaw_engine::{Command, Engine};

/// Sample rate we synthesize the demo source at. The live device may differ;
/// the timeline resamples. We render the device at this rate for the demo so
/// frame↔pixel math lines up with what you hear.
const SR: f64 = 44_100.0;

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

/// A mono sine burst — a stand-in audio source until file loading is wired in.
fn sine_burst(freq_hz: f64, seconds: f64, amp: f32) -> Waveform {
    let frames = (seconds * SR) as usize;
    let inc = TAU * freq_hz / SR;
    let data = (0..frames)
        .map(|i| ((i as f64 * inc).sin() as f32) * amp)
        .collect();
    Waveform::from_planar(1, data).with_sample_rate(SR)
}

struct DawApp {
    /// The arrangement document — source of truth on the UI side.
    project: Project,
    /// Decoded sources, indexed the same as `project.sources`. Held here so the
    /// next step (rebuild-on-edit) can re-`build_graph` after a clip change.
    #[allow(dead_code)]
    sources: Vec<Arc<Waveform>>,
    /// The running audio engine, or `None` if the output device failed to open.
    engine: Option<Engine>,
    /// Last error surfaced to the user (device open, etc.).
    error: Option<String>,
    /// Whether we *think* we're playing — drives the button label and repaint.
    /// The engine is authoritative for position; this is just UI intent.
    playing: bool,
    /// Horizontal zoom: pixels per second of timeline.
    px_per_sec: f32,
}

impl DawApp {
    fn new() -> Self {
        // Build a demo arrangement: one track, two copies of a 440 Hz burst
        // placed at different times, so the timeline has something to draw.
        let burst = Arc::new(sine_burst(440.0, 0.6, 0.4));
        let clip_len = burst.frames() as u64;

        let mut project = Project::new(120.0);
        project.master_gain = 0.8;
        let src = project.add_source("<synth-440hz>");
        project.add_track(
            Track::new("lead")
                .with_clip(ClipData::new(src, 0u64, clip_len))
                .with_clip(ClipData::new(src, clip_len * 2, clip_len)),
        );

        let sources = vec![burst];

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
    fn playhead_secs(&self) -> f64 {
        match &self.engine {
            Some(e) => e.playhead() as f64 / SR,
            None => 0.0,
        }
    }

    fn send(&mut self, cmd: Command) {
        if let Some(e) = &mut self.engine {
            e.send(cmd);
        }
    }

    /// Start or pause-in-place, mirroring the transport button. Bound to Space.
    fn toggle_play_pause(&mut self) {
        if self.playing {
            self.send(Command::Pause);
            self.playing = false;
        } else {
            self.send(Command::Play);
            self.playing = true;
        }
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

        // --- Transport bar -------------------------------------------------
        egui::Panel::top("transport").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let play_label = if self.playing {
                    "⏸ Pause"
                } else {
                    "▶ Play"
                };
                if ui.button(play_label).clicked() {
                    self.toggle_play_pause();
                }
                if ui.button("⏹ Stop").clicked() {
                    self.send(Command::Stop);
                    self.playing = false;
                }

                ui.separator();
                let pos = self.playhead_secs();
                ui.monospace(format!("{:02}:{:06.3}", (pos / 60.0) as u32, pos % 60.0));

                ui.separator();
                ui.label("zoom");
                ui.add(egui::Slider::new(&mut self.px_per_sec, 20.0..=400.0).suffix(" px/s"));
            });
        });

        // --- Timeline ------------------------------------------------------
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
                ui.label("(UI is shown; playback is disabled without an output device)");
            }
            self.draw_timeline(ui);
        });

        // Keep the play head animating while we believe playback is running.
        if self.playing {
            ctx.request_repaint();
        }
    }
}

impl DawApp {
    fn draw_timeline(&mut self, ui: &mut egui::Ui) {
        const RULER_H: f32 = 20.0;
        const LANE_H: f32 = 64.0;
        const LANE_GAP: f32 = 4.0;

        let track_count = self.project.tracks.len().max(1);
        let desired_h = RULER_H + track_count as f32 * (LANE_H + LANE_GAP);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), desired_h),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter().with_clip_rect(rect);

        let px_per_sec = self.px_per_sec;
        let secs_to_x = |s: f64| rect.left() + (s as f32) * px_per_sec;
        let x_to_secs = |x: f32| ((x - rect.left()) / px_per_sec).max(0.0) as f64;

        // Background.
        painter.rect_filled(rect, 0.0, egui::Color32::from_gray(24));

        // Ruler: a gridline + label every second.
        let ruler = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), RULER_H));
        painter.rect_filled(ruler, 0.0, egui::Color32::from_gray(36));
        let max_secs = (rect.width() / px_per_sec).ceil() as i32;
        for s in 0..=max_secs {
            let x = secs_to_x(s as f64);
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0, egui::Color32::from_gray(48)),
            );
            painter.text(
                egui::pos2(x + 3.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{s}s"),
                egui::FontId::monospace(10.0),
                egui::Color32::from_gray(150),
            );
        }

        // Clips, one lane per track.
        for (t, track) in self.project.tracks.iter().enumerate() {
            let lane_top = rect.top() + RULER_H + t as f32 * (LANE_H + LANE_GAP);
            let lane = egui::Rect::from_min_size(
                egui::pos2(rect.left(), lane_top),
                egui::vec2(rect.width(), LANE_H),
            );
            painter.rect_filled(lane, 2.0, egui::Color32::from_gray(30));
            painter.text(
                lane.left_top() + egui::vec2(4.0, 2.0),
                egui::Align2::LEFT_TOP,
                &track.name,
                egui::FontId::proportional(11.0),
                egui::Color32::from_gray(120),
            );

            for clip in &track.clips {
                // Resolve musical/frame positions to seconds for drawing.
                let start_f = self.project.frames_at(clip.start, SR);
                let len_f = self.project.frames_at(clip.len, SR);
                let x0 = secs_to_x(start_f as f64 / SR);
                let x1 = secs_to_x((start_f + len_f) as f64 / SR);
                let clip_rect = egui::Rect::from_min_max(
                    egui::pos2(x0, lane_top + 14.0),
                    egui::pos2(x1, lane_top + LANE_H - 4.0),
                );
                painter.rect_filled(clip_rect, 3.0, egui::Color32::from_rgb(60, 110, 160));
                painter.rect_stroke(
                    clip_rect,
                    3.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 180, 230)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Click/drag scrubs the play head — but only when the gesture *starts*
        // in the ruler band. Pressing in the clip area does nothing (that space
        // is reserved for selecting/dragging clips). We gate on `press_origin`
        // (latched where the drag began) so a scrub started on the ruler keeps
        // tracking even if the pointer strays down into the lanes, and seek to
        // `interact_pos` (the live pointer) so dragging actually moves the head.
        if response.is_pointer_button_down_on() || response.dragged() {
            let ruler_band = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), RULER_H));
            let (origin, current) =
                ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos()));
            if let (Some(origin), Some(current)) = (origin, current) {
                if ruler_band.contains(origin) {
                    let frame = (x_to_secs(current.x) * SR) as u64;
                    self.send(Command::Seek { frame });
                }
            }
        }

        // Play head.
        let head_x = secs_to_x(self.playhead_secs());
        painter.line_segment(
            [
                egui::pos2(head_x, rect.top()),
                egui::pos2(head_x, rect.bottom()),
            ],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 80, 80)),
        );
    }
}
