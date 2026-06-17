//! The timeline widget: draws the ruler, one lane per track with its clips, the
//! scrub interaction, and the play head.

use eframe::egui;
use rdaw_engine::Command;

use crate::app::DawApp;
use crate::SR;

const RULER_H: f32 = 20.0;
const LANE_H: f32 = 64.0;
const LANE_GAP: f32 = 4.0;

/// Draw the timeline into `ui`, reading and mutating `app` state.
pub fn draw(app: &mut DawApp, ui: &mut egui::Ui) {
    let track_count = app.project.tracks.len().max(1);
    let desired_h = RULER_H + track_count as f32 * (LANE_H + LANE_GAP);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), desired_h),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter().with_clip_rect(rect);

    let px_per_sec = app.px_per_sec;
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
    for (t, track) in app.project.tracks.iter().enumerate() {
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
            let start_f = app.project.frames_at(clip.start, SR);
            let len_f = app.project.frames_at(clip.len, SR);
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
        let (origin, current) = ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos()));
        if let (Some(origin), Some(current)) = (origin, current)
            && ruler_band.contains(origin)
        {
            let frame = (x_to_secs(current.x) * SR) as u64;
            app.send(Command::Seek { frame });
        }
    }

    // Play head.
    let head_x = secs_to_x(app.playhead_secs());
    painter.line_segment(
        [
            egui::pos2(head_x, rect.top()),
            egui::pos2(head_x, rect.bottom()),
        ],
        egui::Stroke::new(1.5, egui::Color32::from_rgb(230, 80, 80)),
    );
}
