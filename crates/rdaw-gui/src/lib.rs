//! egui front-end for the DAW. Splits into:
//!
//! - [`app`] — the [`app::DawApp`] state and top-level layout/dispatch.
//! - [`transport`] — the play/pause/stop/zoom bar.
//! - [`timeline`] — the clip timeline widget (draw + scrub).
//! - [`demo`] — a stand-in audio source and demo arrangement, isolated here so
//!   it's easy to delete once real file loading lands.
//!
//! The audio side is unchanged — this only *drives* the existing engine and
//! *reads back* its play position.

pub mod app;
pub mod demo;
pub mod timeline;
pub mod transport;

/// Sample rate we synthesize the demo source at. The live device may differ;
/// the timeline resamples. We render the device at this rate for the demo so
/// frame↔pixel math lines up with what you hear.
pub const SR: f64 = 44_100.0;
