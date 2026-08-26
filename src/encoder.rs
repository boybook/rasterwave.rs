use std::collections::VecDeque;

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
const ENHANCED_PREAMBLE_SECONDS: f64 = 0.800;
const CW_RAMP_SECONDS: f64 = 0.003;

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

/// Optional station identification appended to an SSTV raster.
#[derive(Clone, Debug, PartialEq)]
pub enum SstvStationId {
    /// Do not append station identification.
    None,
    /// Append a QSSTV/DL3YAP-compatible 6-bit FSK identifier.
    Fsk {
        /// Amateur-radio callsign to transmit.
        callsign: String,
    },
    /// Append an audible Morse identifier.
    Cw {
        /// Amateur-radio callsign to transmit.
        callsign: String,
        /// Morse speed in words per minute.
        wpm: u16,
        /// CW audio tone frequency.
        tone_hz: f32,
    },
}

/// Optional tones and silence surrounding a standard SSTV raster.
#[derive(Clone, Debug, PartialEq)]
pub struct SstvTransmissionEnvelope {
    /// Emit the QSSTV-compatible eight-tone calibration preamble before VIS.
    pub enhanced_preamble: bool,
    /// Station identification emitted after the raster.
    pub station_id: SstvStationId,
    /// Silence inserted between the raster and station identification.
    pub post_image_gap_seconds: f64,
    /// Silence emitted after all station identification.
    pub end_guard_seconds: f64,
}

impl Default for SstvTransmissionEnvelope {
    fn default() -> Self {
        Self {
            enhanced_preamble: false,
            station_id: SstvStationId::None,
            post_image_gap_seconds: 0.0,
            end_guard_seconds: 0.0,
        }
    }
}

/// Current portion of an SSTV transmission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderStage {
    /// Optional enhanced calibration tones.
    Preamble,
    /// Standard calibration leader and VIS word.
    Vis,
    /// Mode-specific image raster.
    Raster,
    /// Post-raster station identification and its leading gap.
    StationId,
    /// Final keyed-transmitter silence.
    Guard,
    /// No samples remain.
    Finished,
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
    /// Current transmission portion.
    pub stage: EncoderStage,
    /// First sample belonging to the mode-specific raster.
    pub raster_start_sample: u64,
    /// Exclusive end sample of the mode-specific raster.
    pub raster_end_sample: u64,
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
    frequency_hz: Option<f64>,
    total_samples: usize,
    remaining_samples: usize,
    ramp_samples: usize,
}

#[derive(Clone, Copy, Debug)]
struct QueuedSegment {
    frequency_hz: Option<f64>,
    seconds: f64,
    ramp_seconds: f64,
    stage: EncoderStage,
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
    preamble: VecDeque<QueuedSegment>,
    trailer: VecDeque<QueuedSegment>,
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
    raster_start_sample: u64,
    raster_end_sample: u64,
    stage: EncoderStage,
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
        Self::new_with_envelope(
            image,
            mode,
            sample_rate,
            options,
            SstvTransmissionEnvelope::default(),
        )
    }

    /// Construct a streaming encoder with an optional transmission envelope.
    pub fn new_with_envelope(
        image: RgbImage,
        mode: SstvMode,
        sample_rate: u32,
        options: EncodeOptions,
        envelope: SstvTransmissionEnvelope,
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
        validate_envelope(&options, &envelope)?;

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
        let preamble = build_preamble(&envelope);
        let trailer = build_trailer(&envelope);
        let preamble_seconds = preamble.iter().map(|segment| segment.seconds).sum::<f64>();
        let trailer_seconds = trailer.iter().map(|segment| segment.seconds).sum::<f64>();
        let raster_start_sample =
            ((preamble_seconds + header_seconds) * f64::from(sample_rate)).round() as u64;
        let raster_end_sample = ((preamble_seconds + header_seconds + body_seconds)
            * f64::from(sample_rate))
        .round() as u64;
        let estimated_total = ((preamble_seconds + header_seconds + body_seconds + trailer_seconds)
            * f64::from(sample_rate))
        .round() as u64;
        let stage = if !preamble.is_empty() {
            EncoderStage::Preamble
        } else if options.include_vis_header {
            EncoderStage::Vis
        } else {
            EncoderStage::Raster
        };

        Ok(Self {
            image,
            mode,
            sample_rate,
            options,
            oscillator: Oscillator::default(),
            preamble,
            trailer,
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
            raster_start_sample,
            raster_end_sample,
            stage,
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
            let destination = &mut output[written..written + count];
            if let Some(frequency_hz) = active.frequency_hz {
                if active.ramp_samples > 0 {
                    self.oscillator.fill_ramped(
                        destination,
                        frequency_hz + f64::from(self.options.tone_offset_hz),
                        self.sample_rate,
                        self.options.amplitude,
                        active.total_samples - active.remaining_samples..active.total_samples,
                        active.ramp_samples,
                    );
                } else {
                    self.oscillator.fill(
                        destination,
                        frequency_hz + f64::from(self.options.tone_offset_hz),
                        self.sample_rate,
                        self.options.amplitude,
                    );
                }
            } else {
                destination.fill(0.0);
            }
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
            stage: self.stage,
            raster_start_sample: self.raster_start_sample,
            raster_end_sample: self.raster_end_sample,
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
        if let Some(segment) = self.preamble.pop_front() {
            return Some(self.queued_segment(segment));
        }
        if let Some(segment) = self.next_header_segment() {
            self.stage = EncoderStage::Vis;
            return Some(segment);
        }
        if let Some(segment) = self.next_body_segment() {
            self.stage = EncoderStage::Raster;
            return Some(segment);
        }
        if let Some(segment) = self.trailer.pop_front() {
            return Some(self.queued_segment(segment));
        }
        self.stage = EncoderStage::Finished;
        None
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
                    frequency_hz: Some(pixel_frequency(value)),
                    total_samples: end.saturating_sub(start) as usize,
                    remaining_samples: end.saturating_sub(start) as usize,
                    ramp_samples: 0,
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
        let total_samples = self.schedule_duration(seconds) as usize;
        ActiveSegment {
            frequency_hz: Some(frequency_hz),
            total_samples,
            remaining_samples: total_samples,
            ramp_samples: 0,
        }
    }

    fn queued_segment(&mut self, segment: QueuedSegment) -> ActiveSegment {
        self.stage = segment.stage;
        let total_samples = self.schedule_duration(segment.seconds) as usize;
        ActiveSegment {
            frequency_hz: segment.frequency_hz,
            total_samples,
            remaining_samples: total_samples,
            ramp_samples: (segment.ramp_seconds * f64::from(self.sample_rate)).round() as usize,
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

fn validate_envelope(options: &EncodeOptions, envelope: &SstvTransmissionEnvelope) -> Result<()> {
    if envelope.enhanced_preamble && !options.include_vis_header {
        return Err(Error::InvalidConfiguration(
            "enhanced_preamble requires include_vis_header",
        ));
    }
    for (value, name) in [
        (envelope.post_image_gap_seconds, "post_image_gap_seconds"),
        (envelope.end_guard_seconds, "end_guard_seconds"),
    ] {
        if !value.is_finite() || !(0.0..=5.0).contains(&value) {
            return Err(Error::InvalidConfiguration(match name {
                "post_image_gap_seconds" => {
                    "post_image_gap_seconds must be finite and within 0..=5 seconds"
                }
                _ => "end_guard_seconds must be finite and within 0..=5 seconds",
            }));
        }
    }
    match &envelope.station_id {
        SstvStationId::None => Ok(()),
        SstvStationId::Fsk { callsign } => validate_callsign(callsign),
        SstvStationId::Cw {
            callsign,
            wpm,
            tone_hz,
        } => {
            validate_callsign(callsign)?;
            if !(5..=60).contains(wpm) {
                return Err(Error::InvalidConfiguration("CW WPM must be within 5..=60"));
            }
            if !tone_hz.is_finite() || !(400.0..=2300.0).contains(tone_hz) {
                return Err(Error::InvalidConfiguration(
                    "CW tone_hz must be finite and within 400..=2300 Hz",
                ));
            }
            Ok(())
        }
    }
}

fn validate_callsign(callsign: &str) -> Result<()> {
    if callsign.is_empty()
        || callsign.len() > 16
        || !callsign
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'/')
    {
        return Err(Error::InvalidConfiguration(
            "station callsign must contain 1..=16 uppercase A-Z, 0-9, or / characters",
        ));
    }
    Ok(())
}

fn build_preamble(envelope: &SstvTransmissionEnvelope) -> VecDeque<QueuedSegment> {
    if !envelope.enhanced_preamble {
        return VecDeque::new();
    }
    [
        1900.0, 1500.0, 1900.0, 1500.0, 2300.0, 1500.0, 2300.0, 1500.0,
    ]
    .into_iter()
    .map(|frequency_hz| QueuedSegment {
        frequency_hz: Some(frequency_hz),
        seconds: ENHANCED_PREAMBLE_SECONDS / 8.0,
        ramp_seconds: 0.0,
        stage: EncoderStage::Preamble,
    })
    .collect()
}

fn build_trailer(envelope: &SstvTransmissionEnvelope) -> VecDeque<QueuedSegment> {
    let mut segments = VecDeque::new();
    if !matches!(envelope.station_id, SstvStationId::None) {
        push_silence(
            &mut segments,
            envelope.post_image_gap_seconds,
            EncoderStage::StationId,
        );
        match &envelope.station_id {
            SstvStationId::None => {}
            SstvStationId::Fsk { callsign } => push_fsk_id(&mut segments, callsign),
            SstvStationId::Cw {
                callsign,
                wpm,
                tone_hz,
            } => push_cw_id(&mut segments, callsign, *wpm, f64::from(*tone_hz)),
        }
    }
    push_silence(
        &mut segments,
        envelope.end_guard_seconds,
        EncoderStage::Guard,
    );
    segments
}

fn push_fsk_id(segments: &mut VecDeque<QueuedSegment>, callsign: &str) {
    push_tone(segments, 1500.0, 0.300, EncoderStage::StationId, false);
    push_tone(segments, 2100.0, 0.100, EncoderStage::StationId, false);
    push_tone(segments, 1900.0, 0.022, EncoderStage::StationId, false);
    push_fsk_character(segments, 0x2a);
    let mut checksum = 0_u8;
    for byte in callsign.bytes() {
        let value = byte - 0x20;
        checksum ^= value;
        push_fsk_character(segments, value);
    }
    push_fsk_character(segments, 0x01);
    push_fsk_character(segments, checksum & 0x3f);
    push_tone(segments, 1900.0, 0.100, EncoderStage::StationId, false);
}

fn push_fsk_character(segments: &mut VecDeque<QueuedSegment>, value: u8) {
    for bit in 0..6 {
        let frequency_hz = if value & (1 << bit) != 0 {
            1900.0
        } else {
            2100.0
        };
        push_tone(
            segments,
            frequency_hz,
            0.022,
            EncoderStage::StationId,
            false,
        );
    }
}

fn push_cw_id(segments: &mut VecDeque<QueuedSegment>, callsign: &str, wpm: u16, tone_hz: f64) {
    let dot_seconds = 1.2 / f64::from(wpm);
    for (character_index, character) in callsign.chars().enumerate() {
        if character_index > 0 {
            push_silence(segments, dot_seconds * 3.0, EncoderStage::StationId);
        }
        let code = callsign_morse(character);
        for (symbol_index, symbol) in code.bytes().enumerate() {
            if symbol_index > 0 {
                push_silence(segments, dot_seconds, EncoderStage::StationId);
            }
            push_tone(
                segments,
                tone_hz,
                if symbol == b'-' {
                    dot_seconds * 3.0
                } else {
                    dot_seconds
                },
                EncoderStage::StationId,
                true,
            );
        }
    }
}

fn callsign_morse(character: char) -> &'static str {
    match character {
        'A' => ".-",
        'B' => "-...",
        'C' => "-.-.",
        'D' => "-..",
        'E' => ".",
        'F' => "..-.",
        'G' => "--.",
        'H' => "....",
        'I' => "..",
        'J' => ".---",
        'K' => "-.-",
        'L' => ".-..",
        'M' => "--",
        'N' => "-.",
        'O' => "---",
        'P' => ".--.",
        'Q' => "--.-",
        'R' => ".-.",
        'S' => "...",
        'T' => "-",
        'U' => "..-",
        'V' => "...-",
        'W' => ".--",
        'X' => "-..-",
        'Y' => "-.--",
        'Z' => "--..",
        '0' => "-----",
        '1' => ".----",
        '2' => "..---",
        '3' => "...--",
        '4' => "....-",
        '5' => ".....",
        '6' => "-....",
        '7' => "--...",
        '8' => "---..",
        '9' => "----.",
        '/' => "-..-.",
        _ => unreachable!("callsign validation rejects unsupported characters"),
    }
}

fn push_tone(
    segments: &mut VecDeque<QueuedSegment>,
    frequency_hz: f64,
    seconds: f64,
    stage: EncoderStage,
    ramped: bool,
) {
    segments.push_back(QueuedSegment {
        frequency_hz: Some(frequency_hz),
        seconds,
        ramp_seconds: if ramped { CW_RAMP_SECONDS } else { 0.0 },
        stage,
    });
}

fn push_silence(segments: &mut VecDeque<QueuedSegment>, seconds: f64, stage: EncoderStage) {
    if seconds > 0.0 {
        segments.push_back(QueuedSegment {
            frequency_hz: None,
            seconds,
            ramp_seconds: 0.0,
            stage,
        });
    }
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

    #[test]
    fn fsk_id_uses_dl3yap_marker_and_callsign_checksum() {
        let envelope = SstvTransmissionEnvelope {
            station_id: SstvStationId::Fsk {
                callsign: "A".to_owned(),
            },
            ..SstvTransmissionEnvelope::default()
        };
        let segments = build_trailer(&envelope).into_iter().collect::<Vec<_>>();
        assert_eq!(segments[0].frequency_hz, Some(1500.0));
        assert_eq!(segments[1].frequency_hz, Some(2100.0));
        assert_eq!(segments[2].frequency_hz, Some(1900.0));
        let bits = |offset: usize| {
            segments[offset..offset + 6]
                .iter()
                .map(|segment| segment.frequency_hz == Some(1900.0))
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(3), [false, true, false, true, false, true]);
        assert_eq!(bits(9), [true, false, false, false, false, true]);
        assert_eq!(bits(21), [true, false, false, false, false, true]);
    }

    #[test]
    fn cw_id_uses_paris_timing_and_ramped_tones() {
        let envelope = SstvTransmissionEnvelope {
            station_id: SstvStationId::Cw {
                callsign: "ET".to_owned(),
                wpm: 20,
                tone_hz: 800.0,
            },
            ..SstvTransmissionEnvelope::default()
        };
        let segments = build_trailer(&envelope).into_iter().collect::<Vec<_>>();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].seconds, 0.060);
        assert_eq!(segments[0].ramp_seconds, 0.003);
        assert_eq!(segments[1].frequency_hz, None);
        assert_eq!(segments[1].seconds, 0.180);
        assert!((segments[2].seconds - 0.180).abs() < f64::EPSILON);
    }
}
