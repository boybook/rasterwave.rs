//! Rasterwave RS is a library-first codec for analog radio images.
//!
//! The crate provides incremental SSTV and radiofax encoders and decoders.
//! Stateful codec instances own all mutable data: there is no process-global
//! codec state, which makes separate instances safe to move across threads.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod color;
mod decoder;
mod encoder;
mod error;
mod image;
mod mode;
mod oscillator;
mod vis;

pub mod fax;
pub mod metrics;

pub use decoder::{
    AbortReason, DecodeEvent, DecodeEventRef, DecodeSink, DecoderConfig, DetectionSource,
    LineCompleteness, ProcessReport, SstvDecoder, SyncState,
};
pub use encoder::{EncodeOptions, EncoderProgress, SstvEncoder};
pub use error::{Error, Result};
pub use image::{GrayImage, Rgb, RgbImage};
pub use mode::{Channel, ColorLayout, ModeSpec, ModeStatus, SSTV_MODES, ScanLayout, SstvMode};

/// Encode one complete SSTV image into mono `f32` PCM.
///
/// This convenience API is built on the same incremental encoder used by
/// [`SstvEncoder::read_samples`].
pub fn encode_sstv(image: RgbImage, mode: SstvMode, sample_rate: u32) -> Result<Vec<f32>> {
    let mut encoder = SstvEncoder::new(image, mode, sample_rate, EncodeOptions::default())?;
    let mut pcm = Vec::with_capacity(encoder.estimated_sample_count());
    let mut chunk = vec![0.0_f32; 4096];
    while !encoder.is_finished() {
        let written = encoder.read_samples(&mut chunk);
        pcm.extend_from_slice(&chunk[..written]);
    }
    Ok(pcm)
}

/// Decode a complete mono PCM recording while preserving streaming semantics.
///
/// Events are returned in the same order that an incremental caller would
/// observe them. In particular, scan-line events precede image completion.
pub fn decode_sstv(samples: &[f32], sample_rate: u32) -> Result<Vec<DecodeEvent>> {
    let mut decoder = SstvDecoder::new(sample_rate, DecoderConfig::default())?;
    let mut events = Vec::new();
    decoder.process_into(samples, &mut events)?;
    decoder.finish_into(&mut events)?;
    Ok(events)
}
