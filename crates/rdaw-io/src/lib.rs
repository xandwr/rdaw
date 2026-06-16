//! Minimal, dependency-free WAV I/O — just enough to bounce the engine's
//! output to disk and to load samples back as [`Waveform`]s.
//!
//! This crate is deliberately tiny and hand-rolled: a from-scratch DAW shouldn't
//! need a third-party codec to round-trip uncompressed PCM. We write 32-bit
//! float (lossless for our internal `f32` pipeline) and read both 32-bit float
//! and 16-bit PCM, which covers our own output and most simple source files.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use rdaw_core::{Sample, Waveform};

/// A decoded WAV: the audio plus the sample rate it was recorded at (the engine
/// needs the rate to play it back at the right pitch / resample later).
pub struct LoadedWav {
    pub waveform: Arc<Waveform>,
    pub sample_rate: u32,
}

/// Write interleaved `f32` samples (`[L, R, L, R, ...]`) as a 32-bit float WAV.
pub fn write_wav(
    path: impl AsRef<std::path::Path>,
    interleaved: &[Sample],
    channels: u16,
    sample_rate: u32,
) -> Result<()> {
    const BITS: u16 = 32;
    const FORMAT_IEEE_FLOAT: u16 = 3;

    let block_align = channels * (BITS / 8);
    let byte_rate = sample_rate * block_align as u32;
    let data_bytes = (interleaved.len() * 4) as u32;

    let mut buf = Vec::with_capacity(44 + data_bytes as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&FORMAT_IEEE_FLOAT.to_le_bytes());
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&BITS.to_le_bytes());

    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in interleaved {
        buf.extend_from_slice(&s.to_le_bytes());
    }

    std::fs::write(path.as_ref(), buf)
        .with_context(|| format!("writing WAV to {}", path.as_ref().display()))
}

/// Read a 32-bit float or 16-bit PCM WAV into a [`Waveform`].
pub fn read_wav(path: impl AsRef<std::path::Path>) -> Result<LoadedWav> {
    let bytes = std::fs::read(path.as_ref())
        .with_context(|| format!("reading WAV from {}", path.as_ref().display()))?;

    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }

    let u16_le = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_le = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);

    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut data: Option<&[u8]> = None;

    // Walk the chunk list. Each chunk is an 8-byte header (id + u32 size) then a
    // body padded to an even length.
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_le(&bytes, pos + 4) as usize;
        let body = pos + 8;
        let end = (body + size).min(bytes.len());
        match id {
            b"fmt " if size >= 16 => {
                fmt = Some((
                    u16_le(&bytes, body),
                    u16_le(&bytes, body + 2),
                    u32_le(&bytes, body + 4),
                    u16_le(&bytes, body + 14),
                ));
            }
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        pos = body + size + (size & 1); // skip body + pad byte
    }

    let (format, channels, sample_rate, bits) = fmt.ok_or_else(|| anyhow!("missing fmt chunk"))?;
    let data = data.ok_or_else(|| anyhow!("missing data chunk"))?;
    if channels == 0 {
        bail!("WAV reports zero channels");
    }

    let interleaved: Vec<Sample> = match (format, bits) {
        (3, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (1, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            .collect(),
        _ => bail!("unsupported WAV format tag {format} / {bits}-bit (need float32 or pcm16)"),
    };

    Ok(LoadedWav {
        waveform: Arc::new(
            Waveform::from_interleaved(channels as usize, &interleaved)
                .with_sample_rate(sample_rate as f64),
        ),
        sample_rate,
    })
}
