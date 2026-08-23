use crate::color::rgb_to_yuv;
use crate::oscillator::Oscillator;
use crate::vis::with_even_parity;
use crate::{Channel, Error, Result, Rgb, RgbImage, ScanLayout, SstvMode};

const LEADER_SECONDS: f64 = 0.300;
const BREAK_SECONDS: f64 = 0.010;
const VIS_BIT_SECONDS: f64 = 0.030;
const SYNC_HZ: f64 = 1200.0;
const PORCH_HZ: f64 = 1500.0;
const PIXEL_LOW_HZ: f64 = 1500.0;
const PIXEL_BANDWIDTH_HZ: f64 = 800.0;

/// Options controlling an SSTV transmission.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncodeOptions {
    /// Peak PCM amplitude in `0.0..=1.0`.
    pub amplitude: f32,
    /// Shift every generated tone by this amount.
    ///
    /// This is useful for calibrated test equipment. Normal transmissions use
    /// zero; RF dial placement belongs to the radio rather than this codec.
    pub tone_offset_hz: f32,
    /// Emit the standard leader, break, VIS start/data/parity and stop tones.
    pub include_vis_header: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            amplitude: 0.5,
            tone_offset_hz: 0.0,
            include_vis_header: true,
        }
    }
}

/// Snapshot returned by [`SstvEncoder::progress`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderProgress {
    /// Samples emitted so far.
    pub samples_emitted: u64,
    /// Estimated complete transmission size.
    pub estimated_total_samples: u64,
    /// Current image row, when image scanning has begun.
    pub current_row: Option<u32>,
    /// Whether the encoder has reached the end of the image.
    pub finished: bool,
}

#[derive(Clone, Copy, Debug)]
enum HeaderStage {
    Leader1,
    Break,
    Leader2,
    VisStart,
    VisBit(u8),
    VisStop,
    Done,
}

#[derive(Clone, Copy, Debug)]
enum ScanSource {
    Channel { channel: Channel, row_offset: u8 },
    PdChroma(Channel),
}

#[derive(Clone, Copy, Debug)]
enum Stage {
    Tone { frequency_hz: f64, seconds: f64 },
    Scan { source: ScanSource, seconds: f64 },
}

#[derive(Clone, Copy, Debug)]
struct ActiveSegment {
    frequency_hz: f64,
    remaining_samples: usize,
}

/// Incremental, phase-continuous SSTV encoder.
///
/// The encoder owns its immutable image backing and has no global state. It
/// can be moved to a worker thread and filled with any output chunk size.
#[derive(Debug)]
pub struct SstvEncoder {
    image: RgbImage,
    mode: SstvMode,
    sample_rate: u32,
    options: EncodeOptions,
    oscillator: Oscillator,
    header_stage: HeaderStage,
    body_prefix_pending: bool,
    radio_line: u32,
    body_stage: usize,
    scan_pixel: u32,
    active: Option<ActiveSegment>,
    exact_sample_deadline: f64,
    scheduled_samples: u64,
    scan_stage_total_samples: u64,
    emitted: u64,
    estimated_total: u64,
    finished: bool,
}

impl SstvEncoder {
    /// Construct a streaming encoder.
    pub fn new(
        image: RgbImage,
        mode: SstvMode,
        sample_rate: u32,
        options: EncodeOptions,
    ) -> Result<Self> {
        validate_sample_rate(sample_rate)?;
        if !(0.0..=1.0).contains(&options.amplitude) {
            return Err(Error::InvalidConfiguration(
                "amplitude must be in the range 0.0..=1.0",
            ));
        }
        if !options.tone_offset_hz.is_finite() || options.tone_offset_hz.abs() > 500.0 {
            return Err(Error::InvalidConfiguration(
                "tone_offset_hz must be finite and within +/-500 Hz",
            ));
        }

        let spec = mode.spec();
        if image.width() != spec.width || image.height() != spec.height {
            return Err(Error::ImageDimensions {
                mode,
                expected_width: spec.width,
                expected_height: spec.height,
                actual_width: image.width(),
                actual_height: image.height(),
            });
        }

        let header_seconds = if options.include_vis_header {
            LEADER_SECONDS * 2.0 + BREAK_SECONDS + VIS_BIT_SECONDS * 10.0
        } else {
            0.0
        };
        let body_seconds =
            spec.line_seconds * f64::from(spec.height) / f64::from(spec.rows_per_line);
        let body_seconds = body_seconds
            + if matches!(spec.layout, ScanLayout::Scottie { .. }) {
                spec.sync_seconds
            } else {
                0.0
            };
        let estimated_total =
            ((header_seconds + body_seconds) * f64::from(sample_rate)).round() as u64;

        Ok(Self {
            image,
            mode,
            sample_rate,
            options,
            oscillator: Oscillator::default(),
            header_stage: if options.include_vis_header {
                HeaderStage::Leader1
            } else {
                HeaderStage::Done
            },
            body_prefix_pending: matches!(spec.layout, ScanLayout::Scottie { .. }),
            radio_line: 0,
            body_stage: 0,
            scan_pixel: 0,
            active: None,
            exact_sample_deadline: 0.0,
            scheduled_samples: 0,
            scan_stage_total_samples: 0,
            emitted: 0,
            estimated_total,
            finished: false,
        })
    }

    /// Fill as much of `output` as possible and return the number written.
    ///
    /// A return value smaller than `output.len()` means the transmission ended.
    pub fn read_samples(&mut self, output: &mut [f32]) -> usize {
        if self.finished || output.is_empty() {
            return 0;
        }

        let mut written = 0;
        while written < output.len() {
            if self.active.is_none() {
                self.active = self.next_segment();
                if self.active.is_none() {
                    self.finished = true;
                    break;
                }
            }

            let active = self.active.as_mut().expect("active segment was created");
            let count = active.remaining_samples.min(output.len() - written);
            self.oscillator.fill(
                &mut output[written..written + count],
                active.frequency_hz + f64::from(self.options.tone_offset_hz),
                self.sample_rate,
                self.options.amplitude,
            );
            active.remaining_samples -= count;
            written += count;
            self.emitted += count as u64;
            if active.remaining_samples == 0 {
                self.active = None;
            }
        }
        written
    }

    /// Whether every transmission sample has been emitted.
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    /// Estimated complete sample count.
    ///
    /// Rounding at individual timing boundaries can differ by a few samples
    /// from this duration-derived estimate.
    pub fn estimated_sample_count(&self) -> usize {
        usize::try_from(self.estimated_total).unwrap_or(usize::MAX)
    }

    /// Return a cheap progress snapshot.
    pub fn progress(&self) -> EncoderProgress {
        let spec = self.mode.spec();
        EncoderProgress {
            samples_emitted: self.emitted,
            estimated_total_samples: self.estimated_total,
            current_row: (!self.header_pending() && self.radio_line < self.radio_line_count())
                .then_some(self.radio_line * u32::from(spec.rows_per_line)),
            finished: self.finished,
        }
    }

    fn header_pending(&self) -> bool {
        !matches!(self.header_stage, HeaderStage::Done)
    }

    fn radio_line_count(&self) -> u32 {
        let spec = self.mode.spec();
        spec.height / u32::from(spec.rows_per_line)
    }

    fn next_segment(&mut self) -> Option<ActiveSegment> {
        if let Some(segment) = self.next_header_segment() {
            return Some(segment);
        }
        self.next_body_segment()
    }

    fn next_header_segment(&mut self) -> Option<ActiveSegment> {
        let current = self.header_stage;
        let (frequency_hz, seconds, next) = match current {
            HeaderStage::Leader1 => (1900.0, LEADER_SECONDS, HeaderStage::Break),
            HeaderStage::Break => (1200.0, BREAK_SECONDS, HeaderStage::Leader2),
            HeaderStage::Leader2 => (1900.0, LEADER_SECONDS, HeaderStage::VisStart),
            HeaderStage::VisStart => (1200.0, VIS_BIT_SECONDS, HeaderStage::VisBit(0)),
            HeaderStage::VisBit(bit) => {
                let vis = with_even_parity(self.mode.spec().vis_code);
                let frequency = if (vis >> bit) & 1 == 1 {
                    1100.0
                } else {
                    1300.0
                };
                let next = if bit == 7 {
                    HeaderStage::VisStop
                } else {
                    HeaderStage::VisBit(bit + 1)
                };
                (frequency, VIS_BIT_SECONDS, next)
            }
            HeaderStage::VisStop => (1200.0, VIS_BIT_SECONDS, HeaderStage::Done),
            HeaderStage::Done => return None,
        };
        self.header_stage = next;
        Some(self.timed_segment(frequency_hz, seconds))
    }

    fn next_body_segment(&mut self) -> Option<ActiveSegment> {
        let spec = self.mode.spec();
        if self.body_prefix_pending {
            self.body_prefix_pending = false;
            return Some(self.timed_segment(SYNC_HZ, spec.sync_seconds));
        }
        if self.radio_line >= self.radio_line_count() {
            return None;
        }

        let stage = body_stage(spec, self.body_stage, self.radio_line)?;
        match stage {
            Stage::Tone {
                frequency_hz,
                seconds,
            } => {
                self.advance_body_stage();
                Some(self.timed_segment(frequency_hz, seconds))
            }
            Stage::Scan { source, seconds } => {
                let width = spec.width;
                let pixel = self.scan_pixel;
                let value = self.scan_value(source, self.radio_line, pixel);
                if pixel == 0 {
                    self.scan_stage_total_samples = self.schedule_duration(seconds);
                }
                let total = self.scan_stage_total_samples;
                let start = u64::from(pixel) * total / u64::from(width);
                let end = u64::from(pixel + 1) * total / u64::from(width);
                self.scan_pixel += 1;
                if self.scan_pixel >= width {
                    self.scan_pixel = 0;
                    self.scan_stage_total_samples = 0;
                    self.advance_body_stage();
                }
                Some(ActiveSegment {
                    frequency_hz: pixel_frequency(value),
                    remaining_samples: end.saturating_sub(start) as usize,
                })
            }
        }
    }

    fn advance_body_stage(&mut self) {
        self.body_stage += 1;
        if body_stage(self.mode.spec(), self.body_stage, self.radio_line).is_none() {
            self.body_stage = 0;
            self.radio_line += 1;
        }
    }

    fn timed_segment(&mut self, frequency_hz: f64, seconds: f64) -> ActiveSegment {
        ActiveSegment {
            frequency_hz,
            remaining_samples: self.schedule_duration(seconds) as usize,
        }
    }

    fn schedule_duration(&mut self, seconds: f64) -> u64 {
        self.exact_sample_deadline += seconds * f64::from(self.sample_rate);
        let deadline = self.exact_sample_deadline.round() as u64;
        let duration = deadline.saturating_sub(self.scheduled_samples);
        self.scheduled_samples = deadline;
        duration
    }

    fn scan_value(&self, source: ScanSource, radio_line: u32, x: u32) -> u8 {
        let spec = self.mode.spec();
        let first_row = radio_line * u32::from(spec.rows_per_line);
        match source {
            ScanSource::Channel {
                channel,
                row_offset,
            } => {
                let row = (first_row + u32::from(row_offset)).min(spec.height - 1);
                let first = self.pixel(row, x);
                match channel {
                    Channel::Red => first.r,
                    Channel::Green => first.g,
                    Channel::Blue => first.b,
                    Channel::Luma => rgb_to_yuv(first).y,
                    Channel::ChromaBlue => {
                        let other_row = paired_row(row, spec.height);
                        average_u8(rgb_to_yuv(first).u, rgb_to_yuv(self.pixel(other_row, x)).u)
                    }
                    Channel::ChromaRed => {
                        let other_row = paired_row(row, spec.height);
                        average_u8(rgb_to_yuv(first).v, rgb_to_yuv(self.pixel(other_row, x)).v)
                    }
                }
            }
            ScanSource::PdChroma(channel) => {
                let first = self.pixel(first_row, x);
                let second_row = (first_row + 1).min(spec.height - 1);
                let a = rgb_to_yuv(first);
                let b = rgb_to_yuv(self.pixel(second_row, x));
                match channel {
                    Channel::ChromaBlue => average_u8(a.u, b.u),
                    Channel::ChromaRed => average_u8(a.v, b.v),
                    _ => unreachable!("PD chroma source is always a chroma channel"),
                }
            }
        }
    }

    #[inline]
    fn pixel(&self, row: u32, x: u32) -> Rgb {
        self.image.pixels()[row as usize * self.image.width() as usize + x as usize]
    }
}

fn body_stage(spec: &crate::ModeSpec, index: usize, radio_line: u32) -> Option<Stage> {
    let tone = |frequency_hz, seconds| Stage::Tone {
        frequency_hz,
        seconds,
    };
    let scan = |source, seconds| Stage::Scan { source, seconds };
    match spec.layout {
        ScanLayout::Monochrome { scan_seconds } => match index {
            0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
            1 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Luma,
                    row_offset: 0,
                },
                scan_seconds,
            )),
            _ => None,
        },
        ScanLayout::Martin { channel_seconds } => match index {
            0 => Some(tone(SYNC_HZ, 0.004_862)),
            1 => Some(tone(PORCH_HZ, 0.000_572)),
            2 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Green,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            3 => Some(tone(PORCH_HZ, 0.000_572)),
            4 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Blue,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            5 => Some(tone(PORCH_HZ, 0.000_572)),
            6 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Red,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            7 => Some(tone(PORCH_HZ, 0.000_572)),
            _ => None,
        },
        ScanLayout::Scottie { channel_seconds } => match index {
            0 => Some(tone(PORCH_HZ, 0.0015)),
            1 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Green,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            2 => Some(tone(PORCH_HZ, 0.0015)),
            3 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Blue,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            4 => Some(tone(SYNC_HZ, 0.009)),
            5 => Some(tone(PORCH_HZ, 0.0015)),
            6 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Red,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            _ => None,
        },
        ScanLayout::Robot {
            luma_seconds,
            chroma_seconds,
            alternating_chroma,
            separator_seconds,
            chroma_porch_seconds,
        } => {
            let first_chroma = if alternating_chroma && radio_line % 2 == 1 {
                Channel::ChromaBlue
            } else {
                Channel::ChromaRed
            };
            let chroma_id_hz = if first_chroma == Channel::ChromaRed {
                1500.0
            } else {
                2300.0
            };
            let luma = || {
                scan(
                    ScanSource::Channel {
                        channel: Channel::Luma,
                        row_offset: 0,
                    },
                    luma_seconds,
                )
            };
            let chroma = |channel| {
                scan(
                    ScanSource::Channel {
                        channel,
                        row_offset: 0,
                    },
                    chroma_seconds,
                )
            };
            if alternating_chroma && spec.porch_seconds == 0.0 {
                match index {
                    0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
                    1 => Some(luma()),
                    2 => Some(tone(chroma_id_hz, separator_seconds)),
                    3 => Some(chroma(first_chroma)),
                    _ => None,
                }
            } else if alternating_chroma {
                match index {
                    0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
                    1 => Some(tone(PORCH_HZ, spec.porch_seconds)),
                    2 => Some(luma()),
                    3 => Some(tone(chroma_id_hz, separator_seconds)),
                    4 => Some(tone(1900.0, chroma_porch_seconds)),
                    5 => Some(chroma(first_chroma)),
                    _ => None,
                }
            } else {
                match index {
                    0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
                    1 => Some(tone(PORCH_HZ, spec.porch_seconds)),
                    2 => Some(luma()),
                    3 => Some(tone(1500.0, separator_seconds)),
                    4 => Some(tone(1900.0, chroma_porch_seconds)),
                    5 => Some(chroma(Channel::ChromaRed)),
                    6 => Some(tone(2300.0, separator_seconds)),
                    7 => Some(tone(1500.0, chroma_porch_seconds)),
                    8 => Some(chroma(Channel::ChromaBlue)),
                    _ => None,
                }
            }
        }
        ScanLayout::Pd { channel_seconds } => match index {
            0 => Some(tone(SYNC_HZ, 0.020)),
            1 => Some(tone(PORCH_HZ, 0.002_080)),
            2 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Luma,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            3 => Some(scan(
                ScanSource::PdChroma(Channel::ChromaRed),
                channel_seconds,
            )),
            4 => Some(scan(
                ScanSource::PdChroma(Channel::ChromaBlue),
                channel_seconds,
            )),
            5 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Luma,
                    row_offset: 1,
                },
                channel_seconds,
            )),
            _ => None,
        },
        ScanLayout::Wraase {
            channel_seconds,
            outer_channel_scale,
        } => match index {
            0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
            1 if spec.porch_seconds > 0.0 => Some(tone(PORCH_HZ, spec.porch_seconds)),
            1 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Red,
                    row_offset: 0,
                },
                channel_seconds * outer_channel_scale,
            )),
            2 if spec.porch_seconds > 0.0 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Red,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            2 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Green,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            3 if spec.porch_seconds > 0.0 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Green,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            3 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Blue,
                    row_offset: 0,
                },
                channel_seconds * outer_channel_scale,
            )),
            4 if spec.porch_seconds > 0.0 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Blue,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            _ => None,
        },
        ScanLayout::Pasokon { channel_seconds } => match index {
            0 => Some(tone(SYNC_HZ, spec.sync_seconds)),
            1 => Some(tone(PORCH_HZ, spec.porch_seconds)),
            2 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Red,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            3 => Some(tone(PORCH_HZ, spec.porch_seconds)),
            4 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Green,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            5 => Some(tone(PORCH_HZ, spec.porch_seconds)),
            6 => Some(scan(
                ScanSource::Channel {
                    channel: Channel::Blue,
                    row_offset: 0,
                },
                channel_seconds,
            )),
            7 => Some(tone(PORCH_HZ, spec.porch_seconds)),
            _ => None,
        },
    }
}

#[inline]
fn pixel_frequency(value: u8) -> f64 {
    PIXEL_LOW_HZ + f64::from(value) * PIXEL_BANDWIDTH_HZ / 255.0
}

#[inline]
fn average_u8(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b)) / 2) as u8
}

fn paired_row(row: u32, height: u32) -> u32 {
    if row % 2 == 0 {
        (row + 1).min(height - 1)
    } else {
        row - 1
    }
}

fn validate_sample_rate(sample_rate: u32) -> Result<()> {
    if !(8_000..=384_000).contains(&sample_rate) {
        return Err(Error::InvalidSampleRate(sample_rate));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_and_image_types_are_thread_safe() {
        static_assertions::assert_impl_all!(SstvEncoder: Send, Sync);
        static_assertions::assert_impl_all!(RgbImage: Send, Sync);
        static_assertions::assert_impl_all!(EncodeOptions: Send, Sync, Copy);
    }

    #[test]
    fn parity_is_even() {
        for vis in 0..=127 {
            assert_eq!(with_even_parity(vis).count_ones() % 2, 0);
        }
    }

    #[test]
    fn streaming_output_is_chunk_size_independent() {
        let spec = SstvMode::Robot8Bw.spec();
        let image = RgbImage::filled(spec.width, spec.height, Rgb::new(100, 100, 100));
        let mut small = SstvEncoder::new(
            image.clone(),
            SstvMode::Robot8Bw,
            12_000,
            EncodeOptions::default(),
        )
        .unwrap();
        let mut large =
            SstvEncoder::new(image, SstvMode::Robot8Bw, 12_000, EncodeOptions::default()).unwrap();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut one = [0.0_f32; 37];
        let mut two = [0.0_f32; 4096];
        while !small.is_finished() {
            let n = small.read_samples(&mut one);
            a.extend_from_slice(&one[..n]);
        }
        while !large.is_finished() {
            let n = large.read_samples(&mut two);
            b.extend_from_slice(&two[..n]);
        }
        assert_eq!(a, b);
    }

    #[test]
    fn cumulative_timeline_stays_within_one_sample() {
        for mode in [
            SstvMode::Robot8Bw,
            SstvMode::Robot36,
            SstvMode::Martin2,
            SstvMode::Scottie2,
            SstvMode::Pd50,
            SstvMode::Pasokon3,
        ] {
            for sample_rate in [8_000, 44_100] {
                let spec = mode.spec();
                let image = RgbImage::filled(spec.width, spec.height, Rgb::new(64, 128, 192));
                let mut encoder =
                    SstvEncoder::new(image, mode, sample_rate, EncodeOptions::default()).unwrap();
                let estimate = encoder.estimated_sample_count() as i64;
                let mut chunk = [0.0_f32; 8192];
                while !encoder.is_finished() {
                    encoder.read_samples(&mut chunk);
                }
                let actual = encoder.progress().samples_emitted as i64;
                assert!(
                    (actual - estimate).abs() <= 1,
                    "{mode:?} at {sample_rate} Hz: actual={actual}, estimate={estimate}"
                );
            }
        }
    }
}
