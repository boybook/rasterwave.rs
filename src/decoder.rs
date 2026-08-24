use std::collections::VecDeque;

use crate::color::{Yuv, yuv_to_rgb};
use crate::vis::{VIS_BIT_SECONDS, has_even_parity};
use crate::{ColorLayout, Error, Result, Rgb, SSTV_MODES, ScanLayout, SstvMode};

pub(crate) const WORK_SAMPLE_RATE: u32 = 12_000;
const HEADER_CHECK_STRIDE: u64 = 24;
const HEADER_HISTORY_SECONDS: usize = 8;
const MAX_SYNC_PULSES: usize = 12;
const SYNC_GAP_TOLERANCE_SAMPLES: u64 = WORK_SAMPLE_RATE as u64 * 3 / 2_000;
const VIS_ALIGNMENT_TIMEOUT_SAMPLES: u64 = WORK_SAMPLE_RATE as u64 / 5;
const VIS_CONFIRM_TIMEOUT_SAMPLES: u64 = WORK_SAMPLE_RATE as u64 * 7;
const MAX_MISSED_SYNC_LINES: u64 = 8;
const SYNC_LOSS_LINE_PERIODS: f64 = 8.5;
const SIGNAL_LOSS_SECONDS: f64 = 0.5;
const MAX_EOF_FILTER_DELAY_SAMPLES: usize = 16;

/// How a decoder selected an SSTV mode.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DetectionSource {
    /// A valid VIS word selected the mode.
    Vis {
        /// Seven-bit VIS code.
        code: u8,
    },
    /// Repeated sync pulse width and period selected the best candidate.
    SyncTiming {
        /// More than one mode shared the observed line timing.
        ambiguous: bool,
        /// Number of timing-compatible modes.
        candidate_count: u8,
    },
    /// The caller locked the decoder to one mode.
    Manual,
}

/// Current synchronization state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Looking for a VIS header or stable sync train.
    Searching,
    /// Reading VIS data bits.
    ReadingVis,
    /// VIS passed parity and is being checked against line sync timing.
    Confirming,
    /// A mode and line clock are active.
    Locked,
    /// The decoder was finalized with [`SstvDecoder::finish`].
    Finished,
}

/// Whether a scan line may be revised when later chroma arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCompleteness {
    /// The line is useful for immediate display but may be revised.
    Provisional,
    /// All components needed by the mode are present.
    Final,
}

/// Why an in-progress image ended without normal completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AbortReason {
    /// The caller declared an input discontinuity.
    InputDiscontinuity,
    /// The input ended before every row arrived.
    EndOfInput,
    /// The decoder was reset.
    Reset,
    /// Synchronization could not be maintained.
    SyncLost,
}

/// Runtime decoder policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecoderConfig {
    /// Start decoding the configured manual mode with the first PCM sample.
    /// Sync pulses remain available for clock and phase correction, but they
    /// are not required to begin or continue emitting image rows.
    pub immediate_decode: bool,
    /// Detect standard calibration and VIS headers.
    pub detect_vis: bool,
    /// Infer mode candidates from repeated sync timing when VIS is absent.
    pub detect_sync_timing: bool,
    /// Optional caller-selected mode. VIS and timing detection remain useful
    /// for offset tracking, but this mode wins when a sync train begins.
    pub manual_mode: Option<SstvMode>,
    /// Reject header detection below this exponentially averaged PCM level.
    pub minimum_signal_level: f32,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            immediate_decode: false,
            detect_vis: true,
            detect_sync_timing: true,
            manual_mode: None,
            minimum_signal_level: 0.002,
        }
    }
}

/// Borrowed decoder event used on the allocation-free hot path.
#[derive(Debug)]
#[non_exhaustive]
pub enum DecodeEventRef<'a> {
    /// Sync timing narrowed the signal to one or more modes.
    ModeCandidate {
        /// Compatible modes, ordered by match score.
        candidates: &'a [SstvMode],
        /// Confidence in the timing lock.
        confidence: f32,
    },
    /// A new image started.
    ImageStarted {
        /// Monotonic decoder-local image identifier.
        image_id: u64,
        /// Selected SSTV mode.
        mode: SstvMode,
        /// Detection path.
        detection: DetectionSource,
        /// Estimated audio-frequency offset.
        frequency_offset_hz: f32,
        /// Image width.
        width: u32,
        /// Image height.
        height: u32,
    },
    /// One displayable RGB line is ready.
    LineReady {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Mode being decoded.
        mode: SstvMode,
        /// Zero-based image row.
        line_index: u32,
        /// Monotonic revision for this row.
        revision: u32,
        /// Whether later input may revise this row.
        completeness: LineCompleteness,
        /// RGB pixels, valid until the callback returns.
        pixels: &'a [Rgb],
    },
    /// Every row in an image completed.
    ImageCompleted {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Completed mode.
        mode: SstvMode,
        /// Number of final rows emitted.
        lines: u32,
    },
    /// An incomplete image ended.
    ImageAborted {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Active mode.
        mode: SstvMode,
        /// Last decoded row, if any.
        last_line: Option<u32>,
        /// Termination reason.
        reason: AbortReason,
    },
    /// A header-like signal could not be accepted.
    SignalRejected {
        /// Stable machine-readable reason.
        reason: &'static str,
    },
}

/// Owned form of [`DecodeEventRef`] for queues, language bindings and storage.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DecodeEvent {
    /// Sync timing narrowed the signal to candidates.
    ModeCandidate {
        /// Compatible modes.
        candidates: Vec<SstvMode>,
        /// Timing confidence.
        confidence: f32,
    },
    /// A new image started.
    ImageStarted {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Selected mode.
        mode: SstvMode,
        /// Detection path.
        detection: DetectionSource,
        /// Estimated frequency offset.
        frequency_offset_hz: f32,
        /// Image width.
        width: u32,
        /// Image height.
        height: u32,
    },
    /// One RGB row is ready.
    LineReady {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Active mode.
        mode: SstvMode,
        /// Row index.
        line_index: u32,
        /// Row revision.
        revision: u32,
        /// Completion state.
        completeness: LineCompleteness,
        /// Owned RGB pixels.
        pixels: Vec<Rgb>,
    },
    /// Every row completed.
    ImageCompleted {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Completed mode.
        mode: SstvMode,
        /// Number of final rows.
        lines: u32,
    },
    /// An incomplete image ended.
    ImageAborted {
        /// Decoder-local image identifier.
        image_id: u64,
        /// Active mode.
        mode: SstvMode,
        /// Last decoded row.
        last_line: Option<u32>,
        /// Termination reason.
        reason: AbortReason,
    },
    /// A header-like signal was rejected.
    SignalRejected {
        /// Stable reason.
        reason: &'static str,
    },
}

impl DecodeEventRef<'_> {
    /// Copy a borrowed event into an owned value.
    pub fn to_owned(&self) -> DecodeEvent {
        match self {
            Self::ModeCandidate {
                candidates,
                confidence,
            } => DecodeEvent::ModeCandidate {
                candidates: candidates.to_vec(),
                confidence: *confidence,
            },
            Self::ImageStarted {
                image_id,
                mode,
                detection,
                frequency_offset_hz,
                width,
                height,
            } => DecodeEvent::ImageStarted {
                image_id: *image_id,
                mode: *mode,
                detection: *detection,
                frequency_offset_hz: *frequency_offset_hz,
                width: *width,
                height: *height,
            },
            Self::LineReady {
                image_id,
                mode,
                line_index,
                revision,
                completeness,
                pixels,
            } => DecodeEvent::LineReady {
                image_id: *image_id,
                mode: *mode,
                line_index: *line_index,
                revision: *revision,
                completeness: *completeness,
                pixels: pixels.to_vec(),
            },
            Self::ImageCompleted {
                image_id,
                mode,
                lines,
            } => DecodeEvent::ImageCompleted {
                image_id: *image_id,
                mode: *mode,
                lines: *lines,
            },
            Self::ImageAborted {
                image_id,
                mode,
                last_line,
                reason,
            } => DecodeEvent::ImageAborted {
                image_id: *image_id,
                mode: *mode,
                last_line: *last_line,
                reason: *reason,
            },
            Self::SignalRejected { reason } => DecodeEvent::SignalRejected { reason },
        }
    }
}

/// Consumer for borrowed streaming decode events.
pub trait DecodeSink {
    /// Handle one event synchronously.
    fn on_event(&mut self, event: DecodeEventRef<'_>);
}

impl<F> DecodeSink for F
where
    F: for<'a> FnMut(DecodeEventRef<'a>),
{
    fn on_event(&mut self, event: DecodeEventRef<'_>) {
        self(event);
    }
}

/// Work performed by one [`SstvDecoder::push_f32`] call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessReport {
    /// Caller-rate samples accepted.
    pub input_samples: usize,
    /// Internal 12 kHz samples processed.
    pub working_samples: usize,
    /// Events delivered to the sink.
    pub events_emitted: usize,
}

#[derive(Debug)]
enum DecoderState {
    Searching,
    ReadingVis(VisState),
    Confirming(PendingVisState),
    Receiving(Box<ReceiveState>),
}

#[derive(Debug)]
struct VisState {
    frequency_offset_hz: f32,
    started_at: u64,
    aligning: bool,
    alignment_samples: usize,
    alignment_sum: f64,
    bit_index: u8,
    samples_in_bit: usize,
    frequency_sum: f64,
    bits: u8,
}

#[derive(Debug)]
struct PendingVisState {
    mode: SstvMode,
    detection: DetectionSource,
    frequency_offset_hz: f32,
    body_start: u64,
    started_at: u64,
    pulses: VecDeque<SyncPulse>,
}

#[derive(Clone, Copy, Debug, Default)]
struct SyncPulse {
    start: u64,
    end: u64,
    duration_samples: u32,
    mean_frequency_hz: f32,
}

#[derive(Debug, Default)]
struct SyncDetector {
    active_start: Option<u64>,
    above_threshold_since: Option<u64>,
    frequency_sum: f64,
    frequency_count: u32,
    pulses: VecDeque<SyncPulse>,
}

impl SyncDetector {
    fn process(&mut self, frequency_hz: f32, sample: u64, offset_hz: f32) -> Option<SyncPulse> {
        let threshold = 1380.0 + offset_hz.clamp(-250.0, 250.0);
        if frequency_hz <= threshold {
            if self.active_start.is_none() {
                self.active_start = Some(sample);
                self.frequency_sum = 0.0;
                self.frequency_count = 0;
            }
            self.above_threshold_since = None;
            self.frequency_sum += f64::from(frequency_hz);
            self.frequency_count += 1;
            return None;
        }

        let start = self.active_start?;
        let above_since = *self.above_threshold_since.get_or_insert(sample);
        if sample.saturating_sub(above_since) < SYNC_GAP_TOLERANCE_SAMPLES {
            return None;
        }
        self.active_start = None;
        self.above_threshold_since = None;
        let duration = above_since.saturating_sub(start) as u32;
        let mean = if self.frequency_count == 0 {
            0.0
        } else {
            (self.frequency_sum / f64::from(self.frequency_count)) as f32
        };
        self.frequency_sum = 0.0;
        self.frequency_count = 0;
        let min = samples(0.003) as u32;
        let max = samples(0.025) as u32;
        if !(min..=max).contains(&duration) {
            return None;
        }
        let pulse = SyncPulse {
            start,
            end: above_since,
            duration_samples: duration,
            mean_frequency_hz: mean,
        };
        self.pulses.push_back(pulse);
        while self.pulses.len() > MAX_SYNC_PULSES {
            self.pulses.pop_front();
        }
        Some(pulse)
    }

    fn clear(&mut self) {
        self.active_start = None;
        self.above_threshold_since = None;
        self.frequency_sum = 0.0;
        self.frequency_count = 0;
        self.pulses.clear();
    }

    fn candidates(
        &self,
        scratch: &mut Vec<SstvMode>,
        scored: &mut Vec<(f64, SstvMode)>,
    ) -> Option<(f32, f64, f32, [SyncPulse; 6])> {
        if self.pulses.len() < 6 {
            return None;
        }
        let mut recent = [SyncPulse::default(); 6];
        for (target, pulse) in recent
            .iter_mut()
            .zip(self.pulses.iter().skip(self.pulses.len() - 6))
        {
            *target = *pulse;
        }
        let mut intervals = [0.0_f64; 5];
        for index in 0..5 {
            intervals[index] = recent[index].start.abs_diff(recent[index + 1].start) as f64;
        }
        let mean_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
        let variance = intervals
            .iter()
            .map(|value| (value - mean_interval).powi(2))
            .sum::<f64>()
            / intervals.len() as f64;
        let stddev = variance.sqrt();
        if stddev > samples(0.000_75) as f64 {
            return None;
        }
        let mean_pulse = recent
            .iter()
            .map(|pulse| f64::from(pulse.duration_samples))
            .sum::<f64>()
            / recent.len() as f64;
        let pulse_variance = recent
            .iter()
            .map(|pulse| (f64::from(pulse.duration_samples) - mean_pulse).powi(2))
            .sum::<f64>()
            / recent.len() as f64;
        let pulse_stddev = pulse_variance.sqrt();
        if pulse_stddev > samples(0.001) as f64 {
            return None;
        }
        let mean_offset = recent
            .iter()
            .map(|pulse| pulse.mean_frequency_hz - 1200.0)
            .sum::<f32>()
            / recent.len() as f32;
        if mean_offset.abs() > 250.0 {
            return None;
        }

        scratch.clear();
        scored.clear();
        let pulse_tolerance = samples(0.0015) as f64;
        for spec in SSTV_MODES {
            let expected_pulse = spec.sync_seconds * f64::from(WORK_SAMPLE_RATE);
            let expected_interval = spec.line_seconds * f64::from(WORK_SAMPLE_RATE);
            let pulse_error = (mean_pulse - expected_pulse).abs();
            let interval_error = (mean_interval - expected_interval).abs();
            let interval_tolerance = (expected_interval * 0.010).max(samples(0.0015) as f64);
            if pulse_error <= pulse_tolerance && interval_error <= interval_tolerance {
                let normalized_error =
                    interval_error / interval_tolerance + pulse_error / pulse_tolerance;
                scored.push((normalized_error, spec.mode));
            }
        }
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        scratch.extend(scored.iter().map(|(_, mode)| *mode));
        if scratch.is_empty() {
            return None;
        }
        let interval_stability = 1.0 - stddev / samples(0.000_75) as f64;
        let pulse_stability = 1.0 - pulse_stddev / samples(0.001) as f64;
        let match_quality = 1.0 - scored[0].0 / 2.0;
        let confidence = interval_stability
            .min(pulse_stability)
            .min(match_quality)
            .clamp(0.0, 1.0) as f32;
        Some((confidence, mean_interval, mean_offset, recent))
    }
}

#[derive(Debug)]
struct ReceiveState {
    image_id: u64,
    mode: SstvMode,
    frequency_offset_hz: f32,
    line_period_samples: f64,
    timeline_start: Option<u64>,
    next_line_deadline: f64,
    scheduled_line_samples: u64,
    line_buffer: Vec<f32>,
    started: bool,
    radio_line: u32,
    last_sync_start: Option<u64>,
    sync_lines_confirmed: u32,
    clock_indices: [u64; 8],
    clock_starts: [u64; 8],
    clock_count: usize,
    clock_next_index: u64,
    y_plane: Vec<u8>,
    u_plane: Vec<u8>,
    v_plane: Vec<u8>,
    rgb_plane: Vec<Rgb>,
    revisions: Vec<u32>,
    last_emitted_row: Option<u32>,
    line_scratch: Vec<Rgb>,
    line_decode_scratch: Vec<f32>,
    channel_scratch: Vec<u8>,
    channel_scratch2: Vec<u8>,
}

impl ReceiveState {
    fn new(image_id: u64, mode: SstvMode, frequency_offset_hz: f32) -> Self {
        let spec = mode.spec();
        let pixel_count = spec.width as usize * spec.height as usize;
        Self {
            image_id,
            mode,
            frequency_offset_hz,
            line_period_samples: spec.line_seconds * f64::from(WORK_SAMPLE_RATE),
            timeline_start: None,
            next_line_deadline: spec.line_seconds * f64::from(WORK_SAMPLE_RATE),
            scheduled_line_samples: 0,
            line_buffer: Vec::with_capacity(
                (spec.line_seconds * 1.2 * f64::from(WORK_SAMPLE_RATE)) as usize,
            ),
            started: !matches!(spec.layout, ScanLayout::Scottie { .. }),
            radio_line: 0,
            last_sync_start: None,
            sync_lines_confirmed: 0,
            clock_indices: [0; 8],
            clock_starts: [0; 8],
            clock_count: 0,
            clock_next_index: 0,
            y_plane: vec![0; pixel_count],
            u_plane: vec![128; pixel_count],
            v_plane: vec![128; pixel_count],
            rgb_plane: vec![Rgb::default(); pixel_count],
            revisions: vec![0; spec.height as usize],
            last_emitted_row: None,
            line_scratch: vec![Rgb::default(); spec.width as usize],
            line_decode_scratch: Vec::with_capacity(
                (spec.line_seconds * 1.1 * f64::from(WORK_SAMPLE_RATE)) as usize,
            ),
            channel_scratch: vec![0; spec.width as usize],
            channel_scratch2: vec![0; spec.width as usize],
        }
    }

    fn on_sync(&mut self, pulse: SyncPulse, history: &FrequencyHistory) {
        let spec = self.mode.spec();
        let expected_pulse = spec.sync_seconds * f64::from(WORK_SAMPLE_RATE);
        if (f64::from(pulse.duration_samples) - expected_pulse).abs() > samples(0.0015) as f64 {
            return;
        }
        if let Some(previous) = self.last_sync_start {
            let measured = pulse.start.saturating_sub(previous) as f64;
            let steps = (measured / self.line_period_samples).round() as u64;
            if !(1..=MAX_MISSED_SYNC_LINES).contains(&steps) {
                return;
            }
            let expected = self.line_period_samples * steps as f64;
            let tolerance = (expected * 0.01).max(samples(0.003) as f64);
            if (measured - expected).abs() > tolerance {
                return;
            }
            self.clock_next_index = self.clock_next_index.saturating_add(steps);
            self.sync_lines_confirmed = self
                .radio_line_count()
                .min(self.clock_next_index.min(u64::from(u32::MAX)) as u32);
        } else {
            self.clock_next_index = 0;
            self.sync_lines_confirmed = 0;
        }
        self.frequency_offset_hz =
            self.frequency_offset_hz * 0.95 + (pulse.mean_frequency_hz - 1200.0) * 0.05;
        self.last_sync_start = Some(pulse.start);
        self.record_clock_point(self.clock_next_index, pulse.start);

        if !self.started {
            let start = if matches!(spec.layout, ScanLayout::Scottie { .. }) {
                pulse.end
            } else {
                pulse.start
            };
            self.line_buffer.clear();
            history.copy_from_absolute(start, &mut self.line_buffer);
            self.timeline_start = Some(start);
            self.started = true;
        }
    }

    fn record_clock_point(&mut self, index: u64, start: u64) {
        if self.clock_count < self.clock_indices.len() {
            self.clock_indices[self.clock_count] = index;
            self.clock_starts[self.clock_count] = start;
            self.clock_count += 1;
        } else {
            self.clock_indices.copy_within(1.., 0);
            self.clock_starts.copy_within(1.., 0);
            let last = self.clock_indices.len() - 1;
            self.clock_indices[last] = index;
            self.clock_starts[last] = start;
        }
        if self.clock_count < 4 {
            return;
        }

        let x0 = self.clock_indices[0] as f64;
        let y0 = self.clock_starts[0] as f64;
        let count = self.clock_count as f64;
        let mean_x = self.clock_indices[..self.clock_count]
            .iter()
            .map(|value| *value as f64 - x0)
            .sum::<f64>()
            / count;
        let mean_y = self.clock_starts[..self.clock_count]
            .iter()
            .map(|value| *value as f64 - y0)
            .sum::<f64>()
            / count;
        let mut covariance = 0.0;
        let mut variance = 0.0;
        for point in 0..self.clock_count {
            let x = self.clock_indices[point] as f64 - x0 - mean_x;
            let y = self.clock_starts[point] as f64 - y0 - mean_y;
            covariance += x * y;
            variance += x * x;
        }
        if variance <= f64::EPSILON {
            return;
        }
        let slope = covariance / variance;
        let nominal = self.mode.spec().line_seconds * f64::from(WORK_SAMPLE_RATE);
        if !(nominal * 0.998..=nominal * 1.002).contains(&slope) {
            return;
        }
        let intercept = mean_y - slope * mean_x;
        let residual_rms = (self.clock_indices[..self.clock_count]
            .iter()
            .zip(&self.clock_starts[..self.clock_count])
            .map(|(x, y)| {
                let predicted = intercept + slope * (*x as f64 - x0);
                (*y as f64 - y0 - predicted).powi(2)
            })
            .sum::<f64>()
            / count)
            .sqrt();
        if residual_rms <= samples(0.0015) as f64 {
            self.line_period_samples = slope;
            if let Some(timeline_start) = self.timeline_start {
                let sync_zero = y0 + intercept - slope * x0;
                let phase_seconds = match self.mode.spec().layout {
                    ScanLayout::Scottie { channel_seconds } => {
                        2.0 * (self.mode.spec().porch_seconds + channel_seconds)
                    }
                    _ => 0.0,
                };
                let nominal = self.mode.spec().line_seconds * f64::from(WORK_SAMPLE_RATE);
                let phase_samples = phase_seconds * f64::from(WORK_SAMPLE_RATE) * slope / nominal;
                let line_zero = sync_zero - phase_samples;
                let next_end =
                    line_zero + f64::from(self.radio_line + 1) * slope - timeline_start as f64;
                let minimum = self.scheduled_line_samples as f64 + slope * 0.5;
                self.next_line_deadline = next_end.max(minimum);
            }
        }
    }

    fn radio_line_count(&self) -> u32 {
        let spec = self.mode.spec();
        spec.height / u32::from(spec.rows_per_line)
    }

    fn sync_is_lost(&self, now: u64) -> bool {
        self.started
            && self.last_sync_start.is_some_and(|last| {
                now.saturating_sub(last) as f64 > self.line_period_samples * SYNC_LOSS_LINE_PERIODS
            })
    }
}

#[derive(Debug)]
pub(crate) struct LinearResampler {
    input_rate: u32,
    source_index: u64,
    next_output_position: f64,
    previous: f32,
    initialized: bool,
}

impl LinearResampler {
    pub(crate) fn new(input_rate: u32) -> Self {
        Self {
            input_rate,
            source_index: 0,
            next_output_position: 0.0,
            previous: 0.0,
            initialized: false,
        }
    }

    pub(crate) fn push(&mut self, input: f32, mut output: impl FnMut(f32)) -> usize {
        if !self.initialized {
            self.initialized = true;
            self.previous = input;
            self.source_index = 0;
            output(input);
            self.next_output_position = f64::from(self.input_rate) / f64::from(WORK_SAMPLE_RATE);
            return 1;
        }
        self.source_index += 1;
        let current_position = self.source_index as f64;
        let previous_position = current_position - 1.0;
        let step = f64::from(self.input_rate) / f64::from(WORK_SAMPLE_RATE);
        let mut count = 0;
        while self.next_output_position <= current_position {
            let fraction = (self.next_output_position - previous_position).clamp(0.0, 1.0);
            output(self.previous + (input - self.previous) * fraction as f32);
            self.next_output_position += step;
            count += 1;
        }
        self.previous = input;
        count
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.input_rate);
    }
}

#[derive(Debug)]
pub(crate) struct FrequencyDemodulator {
    center_hz: f32,
    oscillator_sin: f64,
    oscillator_cos: f64,
    oscillator_sin_step: f64,
    oscillator_cos_step: f64,
    oscillator_until_normalize: u16,
    i_stage1: f64,
    q_stage1: f64,
    i_stage2: f64,
    q_stage2: f64,
    previous_i: f64,
    previous_q: f64,
    has_previous_vector: bool,
    current_frequency: f32,
    amplitude_ema: f32,
    carrier_level_ema: f32,
}

impl Default for FrequencyDemodulator {
    fn default() -> Self {
        Self::new(1900.0)
    }
}

impl FrequencyDemodulator {
    pub(crate) fn new(center_hz: f32) -> Self {
        let step = std::f64::consts::TAU * f64::from(center_hz) / f64::from(WORK_SAMPLE_RATE);
        let (oscillator_sin_step, oscillator_cos_step) = step.sin_cos();
        Self {
            center_hz,
            oscillator_sin: 0.0,
            oscillator_cos: 1.0,
            oscillator_sin_step,
            oscillator_cos_step,
            oscillator_until_normalize: 4096,
            i_stage1: 0.0,
            q_stage1: 0.0,
            i_stage2: 0.0,
            q_stage2: 0.0,
            previous_i: 0.0,
            previous_q: 0.0,
            has_previous_vector: false,
            current_frequency: center_hz,
            amplitude_ema: 0.0,
            carrier_level_ema: 0.0,
        }
    }

    pub(crate) fn process(&mut self, sample: f32, _index: u64) -> f32 {
        self.amplitude_ema += (sample.abs() - self.amplitude_ema) * 0.001;
        let sample = f64::from(sample);
        let mixed_i = sample * self.oscillator_cos;
        let mixed_q = -sample * self.oscillator_sin;
        let next_sin = self.oscillator_sin * self.oscillator_cos_step
            + self.oscillator_cos * self.oscillator_sin_step;
        let next_cos = self.oscillator_cos * self.oscillator_cos_step
            - self.oscillator_sin * self.oscillator_sin_step;
        self.oscillator_sin = next_sin;
        self.oscillator_cos = next_cos;
        self.oscillator_until_normalize -= 1;
        if self.oscillator_until_normalize == 0 {
            let magnitude = self.oscillator_sin.hypot(self.oscillator_cos);
            if magnitude > f64::EPSILON {
                self.oscillator_sin /= magnitude;
                self.oscillator_cos /= magnitude;
            }
            self.oscillator_until_normalize = 4096;
        }

        const ALPHA: f64 = 0.42;
        self.i_stage1 += ALPHA * (mixed_i - self.i_stage1);
        self.q_stage1 += ALPHA * (mixed_q - self.q_stage1);
        self.i_stage2 += ALPHA * (self.i_stage1 - self.i_stage2);
        self.q_stage2 += ALPHA * (self.q_stage1 - self.q_stage2);

        let magnitude = self.i_stage2.hypot(self.q_stage2);
        self.carrier_level_ema += ((2.0 * magnitude) as f32 - self.carrier_level_ema) * 0.02;
        if self.has_previous_vector && magnitude > 0.0005 {
            let cross = self.previous_i * self.q_stage2 - self.previous_q * self.i_stage2;
            let dot = self.previous_i * self.i_stage2 + self.previous_q * self.q_stage2;
            let delta = cross.atan2(dot);
            let measured = f64::from(self.center_hz)
                + delta * f64::from(WORK_SAMPLE_RATE) / std::f64::consts::TAU;
            if (50.0..f64::from(WORK_SAMPLE_RATE) * 0.5 - 50.0).contains(&measured) {
                self.current_frequency += (measured as f32 - self.current_frequency) * 0.22;
            }
        }
        self.previous_i = self.i_stage2;
        self.previous_q = self.q_stage2;
        self.has_previous_vector = magnitude > 0.0005;
        self.current_frequency
    }

    pub(crate) const fn carrier_level(&self) -> f32 {
        self.carrier_level_ema
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.center_hz);
    }
}

#[derive(Debug)]
struct FrequencyHistory {
    data: Vec<f32>,
    head: usize,
    len: usize,
    first_absolute: u64,
}

impl FrequencyHistory {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity.max(1)],
            head: 0,
            len: 0,
            first_absolute: 0,
        }
    }

    fn push(&mut self, value: f32, absolute: u64) {
        if self.len < self.data.len() {
            let index = (self.head + self.len) % self.data.len();
            self.data[index] = value;
            if self.len == 0 {
                self.first_absolute = absolute;
            }
            self.len += 1;
        } else {
            self.data[self.head] = value;
            self.head = (self.head + 1) % self.data.len();
            self.first_absolute = self.first_absolute.saturating_add(1);
        }
    }

    fn mean_from_end(&self, end_offset: usize, length: usize) -> Option<f32> {
        if length == 0 || end_offset + length > self.len {
            return None;
        }
        let start = self.len - end_offset - length;
        let mut sum = 0.0_f64;
        for logical in start..start + length {
            sum += f64::from(self.data[(self.head + logical) % self.data.len()]);
        }
        Some((sum / length as f64) as f32)
    }

    fn copy_from_absolute(&self, start: u64, output: &mut Vec<f32>) {
        if self.len == 0 {
            return;
        }
        let logical_start = start.saturating_sub(self.first_absolute) as usize;
        if logical_start >= self.len {
            return;
        }
        output.reserve(self.len - logical_start);
        for logical in logical_start..self.len {
            output.push(self.data[(self.head + logical) % self.data.len()]);
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.first_absolute = 0;
    }
}

/// Online SSTV decoder with bounded acquisition and line buffers.
#[derive(Debug)]
pub struct SstvDecoder {
    input_sample_rate: u32,
    config: DecoderConfig,
    resampler: LinearResampler,
    demodulator: FrequencyDemodulator,
    history: FrequencyHistory,
    sync_detector: SyncDetector,
    candidate_scratch: Vec<SstvMode>,
    candidate_score_scratch: Vec<(f64, SstvMode)>,
    state: DecoderState,
    working_sample: u64,
    next_image_id: u64,
    signal_level_ema: f32,
    last_signal_sample: u64,
    finished: bool,
}

impl SstvDecoder {
    /// Construct an online decoder for the caller's PCM sample rate.
    pub fn new(input_sample_rate: u32, config: DecoderConfig) -> Result<Self> {
        if !(8_000..=384_000).contains(&input_sample_rate) {
            return Err(Error::InvalidSampleRate(input_sample_rate));
        }
        if !config.minimum_signal_level.is_finite() || config.minimum_signal_level < 0.0 {
            return Err(Error::InvalidConfiguration(
                "minimum_signal_level must be finite and non-negative",
            ));
        }
        if config.immediate_decode && config.manual_mode.is_none() {
            return Err(Error::InvalidConfiguration(
                "immediate_decode requires manual_mode",
            ));
        }
        Ok(Self {
            input_sample_rate,
            config,
            resampler: LinearResampler::new(input_sample_rate),
            demodulator: FrequencyDemodulator::default(),
            history: FrequencyHistory::new(WORK_SAMPLE_RATE as usize * HEADER_HISTORY_SECONDS),
            sync_detector: SyncDetector::default(),
            candidate_scratch: Vec::with_capacity(8),
            candidate_score_scratch: Vec::with_capacity(8),
            state: DecoderState::Searching,
            working_sample: 0,
            next_image_id: 1,
            signal_level_ema: 0.0,
            last_signal_sample: 0,
            finished: false,
        })
    }

    /// Push arbitrary-sized mono `f32` PCM into the streaming state machine.
    pub fn push_f32(&mut self, input: &[f32], sink: &mut impl DecodeSink) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(Error::NonFiniteSample);
        }
        if input.is_empty() {
            return Ok(ProcessReport::default());
        }
        let mut report = ProcessReport {
            input_samples: input.len(),
            ..ProcessReport::default()
        };
        for &sample in input {
            // Collect at most a handful of working-rate outputs per source sample.
            let mut resampled = [0.0_f32; 4];
            let mut count = 0;
            self.resampler.push(sample, |value| {
                if count < resampled.len() {
                    resampled[count] = value;
                    count += 1;
                }
            });
            for value in &resampled[..count] {
                report.working_samples += 1;
                report.events_emitted += self.process_working_sample(*value, sink);
            }
        }
        Ok(report)
    }

    /// Convert signed 16-bit PCM and feed it through [`Self::push_f32`].
    pub fn push_i16(&mut self, input: &[i16], sink: &mut impl DecodeSink) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut report = ProcessReport {
            input_samples: input.len(),
            ..ProcessReport::default()
        };
        let mut converted = [0.0_f32; 1024];
        for chunk in input.chunks(converted.len()) {
            for (target, source) in converted.iter_mut().zip(chunk) {
                *target = f32::from(*source) / 32768.0;
            }
            let chunk_report = self.push_f32(&converted[..chunk.len()], sink)?;
            report.working_samples += chunk_report.working_samples;
            report.events_emitted += chunk_report.events_emitted;
        }
        Ok(report)
    }

    /// Convenience collector for callers that need owned events.
    pub fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<DecodeEvent>,
    ) -> Result<ProcessReport> {
        let mut sink = |event: DecodeEventRef<'_>| output.push(event.to_owned());
        self.push_f32(input, &mut sink)
    }

    /// Finalize the stream and report an incomplete active image.
    pub fn finish(&mut self, sink: &mut impl DecodeSink) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut report = ProcessReport::default();
        let state = std::mem::replace(&mut self.state, DecoderState::Searching);
        if let DecoderState::Receiving(mut receive) = state {
            let allow_unconfirmed_final =
                self.signal_level_ema >= self.config.minimum_signal_level.max(0.0005);
            report.events_emitted +=
                decode_ready_lines(&mut receive, allow_unconfirmed_final, true, sink);
            if receive.radio_line >= receive.radio_line_count() {
                sink.on_event(DecodeEventRef::ImageCompleted {
                    image_id: receive.image_id,
                    mode: receive.mode,
                    lines: receive.mode.spec().height,
                });
            } else {
                sink.on_event(DecodeEventRef::ImageAborted {
                    image_id: receive.image_id,
                    mode: receive.mode,
                    last_line: receive.last_emitted_row,
                    reason: AbortReason::EndOfInput,
                });
            }
            report.events_emitted += 1;
        }
        self.finished = true;
        Ok(report)
    }

    /// Owned-event convenience form of [`Self::finish`].
    pub fn finish_into(&mut self, output: &mut Vec<DecodeEvent>) -> Result<ProcessReport> {
        let mut sink = |event: DecodeEventRef<'_>| output.push(event.to_owned());
        self.finish(&mut sink)
    }

    /// Explicitly break time continuity after dropped PCM.
    pub fn mark_discontinuity(
        &mut self,
        _dropped_input_samples: u64,
        sink: &mut impl DecodeSink,
    ) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut report = ProcessReport::default();
        if let DecoderState::Receiving(receive) = &self.state {
            sink.on_event(DecodeEventRef::ImageAborted {
                image_id: receive.image_id,
                mode: receive.mode,
                last_line: receive.last_emitted_row,
                reason: AbortReason::InputDiscontinuity,
            });
            report.events_emitted = 1;
        }
        self.reset_runtime();
        Ok(report)
    }

    /// Reset all acquisition state and allow input after finalization.
    ///
    /// This convenience form silently discards an active image. Use
    /// [`Self::reset_with_sink`] when consumers need an explicit reset event.
    pub fn reset(&mut self) {
        self.finished = false;
        self.reset_runtime();
    }

    /// Reset acquisition and report an active image as explicitly aborted.
    pub fn reset_with_sink(&mut self, sink: &mut impl DecodeSink) -> ProcessReport {
        let mut report = ProcessReport::default();
        if let DecoderState::Receiving(receive) = &self.state {
            sink.on_event(DecodeEventRef::ImageAborted {
                image_id: receive.image_id,
                mode: receive.mode,
                last_line: receive.last_emitted_row,
                reason: AbortReason::Reset,
            });
            report.events_emitted = 1;
        }
        self.finished = false;
        self.reset_runtime();
        report
    }

    /// Current synchronization state.
    pub fn sync_state(&self) -> SyncState {
        if self.finished {
            SyncState::Finished
        } else {
            match self.state {
                DecoderState::Searching => SyncState::Searching,
                DecoderState::ReadingVis(_) => SyncState::ReadingVis,
                DecoderState::Confirming(_) => SyncState::Confirming,
                DecoderState::Receiving(_) => SyncState::Locked,
            }
        }
    }

    /// Caller PCM sample rate.
    pub const fn input_sample_rate(&self) -> u32 {
        self.input_sample_rate
    }

    fn reset_runtime(&mut self) {
        self.resampler.reset();
        self.demodulator.reset();
        self.history.clear();
        self.sync_detector.clear();
        self.candidate_scratch.clear();
        self.candidate_score_scratch.clear();
        self.state = DecoderState::Searching;
        self.working_sample = 0;
        self.signal_level_ema = 0.0;
        self.last_signal_sample = 0;
    }

    fn process_working_sample(&mut self, sample: f32, sink: &mut impl DecodeSink) -> usize {
        let absolute = self.working_sample;
        self.signal_level_ema += (sample.abs() - self.signal_level_ema) * 0.04;
        if self.signal_level_ema >= self.config.minimum_signal_level {
            self.last_signal_sample = absolute;
        }
        let frequency = self.demodulator.process(sample, absolute);
        self.history.push(frequency, absolute);
        let offset = match &self.state {
            DecoderState::ReadingVis(vis) => vis.frequency_offset_hz,
            DecoderState::Confirming(pending) => pending.frequency_offset_hz,
            DecoderState::Receiving(receive) => receive.frequency_offset_hz,
            DecoderState::Searching => 0.0,
        };
        let pulse = self.sync_detector.process(frequency, absolute, offset);
        let mut events = 0;
        let mut confirmed_vis: Option<(SstvMode, DetectionSource, f32, [SyncPulse; 3], f64)> = None;
        let mut sync_lost: Option<(u64, SstvMode, Option<u32>)> = None;

        if self.config.immediate_decode && matches!(self.state, DecoderState::Searching) {
            let mode = self
                .config
                .manual_mode
                .expect("immediate_decode manual mode validated at construction");
            events += self.begin_image(mode, DetectionSource::Manual, 0.0, sink);
            if let DecoderState::Receiving(receive) = &mut self.state {
                receive.started = true;
                receive.timeline_start = Some(absolute);
                receive.sync_lines_confirmed = receive.radio_line_count();
            }
        }

        match &mut self.state {
            DecoderState::Searching => {
                if self.config.detect_vis
                    && absolute % HEADER_CHECK_STRIDE == 0
                    && self.demodulator.amplitude_ema >= self.config.minimum_signal_level
                {
                    if let Some(header_offset) = detect_header(&self.history) {
                        self.state = DecoderState::ReadingVis(VisState {
                            frequency_offset_hz: header_offset,
                            started_at: absolute,
                            aligning: true,
                            alignment_samples: 0,
                            alignment_sum: 0.0,
                            bit_index: 0,
                            samples_in_bit: 0,
                            frequency_sum: 0.0,
                            bits: 0,
                        });
                    }
                }

                if matches!(self.state, DecoderState::Searching)
                    && (self.config.detect_sync_timing || self.config.manual_mode.is_some())
                    && pulse.is_some()
                {
                    events += self.try_sync_timing_lock(sink);
                }
            }
            DecoderState::ReadingVis(vis) => {
                if vis.aligning
                    && absolute.saturating_sub(vis.started_at) > VIS_ALIGNMENT_TIMEOUT_SAMPLES
                {
                    sink.on_event(DecodeEventRef::SignalRejected {
                        reason: "vis-alignment-timeout",
                    });
                    events += 1;
                    self.state = DecoderState::Searching;
                    self.sync_detector.clear();
                    self.working_sample += 1;
                    return events;
                }
                if vis.aligning {
                    let threshold = 1200.0 + vis.frequency_offset_hz;
                    if (frequency - threshold).abs() >= 45.0 {
                        vis.alignment_samples += 1;
                        vis.alignment_sum += f64::from(frequency);
                        if vis.alignment_samples >= 4 {
                            vis.aligning = false;
                            vis.samples_in_bit = vis.alignment_samples;
                            vis.frequency_sum = vis.alignment_sum;
                        }
                    } else {
                        vis.alignment_samples = 0;
                        vis.alignment_sum = 0.0;
                    }
                    self.working_sample += 1;
                    return events;
                }
                vis.frequency_sum += f64::from(frequency);
                vis.samples_in_bit += 1;
                if vis.samples_in_bit >= samples(VIS_BIT_SECONDS) {
                    let average = (vis.frequency_sum / vis.samples_in_bit as f64) as f32;
                    let threshold = 1200.0 + vis.frequency_offset_hz;
                    if vis.bit_index < 8 {
                        if average < threshold {
                            vis.bits |= 1 << vis.bit_index;
                        }
                        vis.bit_index += 1;
                        vis.samples_in_bit = 0;
                        vis.frequency_sum = 0.0;
                    } else {
                        let bits = vis.bits;
                        let frequency_offset_hz = vis.frequency_offset_hz;
                        let valid_stop = (average - threshold).abs() <= 100.0;
                        if !has_even_parity(bits) || !valid_stop {
                            sink.on_event(DecodeEventRef::SignalRejected {
                                reason: "invalid-vis",
                            });
                            events += 1;
                            self.state = DecoderState::Searching;
                        } else {
                            let observed = SstvMode::from_vis(bits & 0x7f);
                            if let Some(mode) = observed {
                                let candidate = [mode];
                                sink.on_event(DecodeEventRef::ModeCandidate {
                                    candidates: &candidate,
                                    confidence: 1.0,
                                });
                                events += 1;
                            }
                            if let Some(mode) = self.config.manual_mode.or(observed) {
                                let detection = if self.config.manual_mode.is_some() {
                                    DetectionSource::Manual
                                } else {
                                    DetectionSource::Vis { code: bits & 0x7f }
                                };
                                self.sync_detector.clear();
                                self.state = DecoderState::Confirming(PendingVisState {
                                    mode,
                                    detection,
                                    frequency_offset_hz,
                                    body_start: absolute.saturating_add(1),
                                    started_at: absolute,
                                    pulses: VecDeque::with_capacity(5),
                                });
                            } else {
                                sink.on_event(DecodeEventRef::SignalRejected {
                                    reason: "unsupported-vis",
                                });
                                events += 1;
                                self.state = DecoderState::Searching;
                            }
                        }
                    }
                }
            }
            DecoderState::Confirming(pending) => {
                if absolute.saturating_sub(pending.started_at) > VIS_CONFIRM_TIMEOUT_SAMPLES {
                    sink.on_event(DecodeEventRef::SignalRejected {
                        reason: "vis-sync-timeout",
                    });
                    events += 1;
                    self.state = DecoderState::Searching;
                    self.sync_detector.clear();
                    self.working_sample += 1;
                    return events;
                }
                if let Some(pulse) = pulse {
                    let expected_pulse =
                        pending.mode.spec().sync_seconds * f64::from(WORK_SAMPLE_RATE);
                    if (f64::from(pulse.duration_samples) - expected_pulse).abs()
                        <= samples(0.003) as f64
                    {
                        pending.pulses.push_back(pulse);
                        while pending.pulses.len() > 5 {
                            pending.pulses.pop_front();
                        }
                        if let Some((pulses, period)) = confirm_vis_timing(
                            pending.mode,
                            pending.body_start,
                            &mut pending.pulses,
                        ) {
                            confirmed_vis = Some((
                                pending.mode,
                                pending.detection,
                                pending.frequency_offset_hz,
                                pulses,
                                period,
                            ));
                        }
                    }
                }
            }
            DecoderState::Receiving(receive) => {
                if receive.started {
                    receive.line_buffer.push(frequency);
                }
                if let Some(pulse) = pulse {
                    receive.on_sync(pulse, &self.history);
                    if self.config.immediate_decode {
                        receive.sync_lines_confirmed = receive.radio_line_count();
                    }
                }
                let allow_unconfirmed_final =
                    self.signal_level_ema >= self.config.minimum_signal_level.max(0.0005);
                events += decode_ready_lines(receive, allow_unconfirmed_final, false, sink);
                if receive.radio_line >= receive.radio_line_count() {
                    sink.on_event(DecodeEventRef::ImageCompleted {
                        image_id: receive.image_id,
                        mode: receive.mode,
                        lines: receive.mode.spec().height,
                    });
                    events += 1;
                    self.state = DecoderState::Searching;
                } else if !self.config.immediate_decode
                    && (receive.sync_is_lost(absolute)
                        || (self.config.minimum_signal_level > 0.0
                            && absolute.saturating_sub(self.last_signal_sample)
                                > samples(SIGNAL_LOSS_SECONDS) as u64))
                {
                    sync_lost = Some((receive.image_id, receive.mode, receive.last_emitted_row));
                }
            }
        }

        if let Some((image_id, mode, last_line)) = sync_lost {
            sink.on_event(DecodeEventRef::ImageAborted {
                image_id,
                mode,
                last_line,
                reason: AbortReason::SyncLost,
            });
            events += 1;
            self.state = DecoderState::Searching;
            self.sync_detector.clear();
        }

        if let Some((mode, detection, frequency_offset_hz, pulses, period)) = confirmed_vis {
            events += self.begin_image(mode, detection, frequency_offset_hz, sink);
            if let DecoderState::Receiving(receive) = &mut self.state {
                receive.line_period_samples = period;
                receive.next_line_deadline = period;
                receive.last_sync_start = Some(pulses[2].start);
                receive.line_buffer.clear();
                let start = match mode.spec().layout {
                    ScanLayout::Scottie { channel_seconds } => {
                        let before_sync = 2.0 * (mode.spec().porch_seconds + channel_seconds);
                        let nominal_period = mode.spec().line_seconds * f64::from(WORK_SAMPLE_RATE);
                        let clock_scale = period / nominal_period;
                        pulses[0].start.saturating_sub(
                            (before_sync * f64::from(WORK_SAMPLE_RATE) * clock_scale).round()
                                as u64,
                        )
                    }
                    _ => pulses[0].start,
                };
                self.history
                    .copy_from_absolute(start, &mut receive.line_buffer);
                receive.timeline_start = Some(start);
                receive.started = true;
                receive.clock_next_index = 2;
                receive.sync_lines_confirmed = 2.min(receive.radio_line_count());
                for (index, pulse) in pulses.iter().enumerate() {
                    receive.record_clock_point(index as u64, pulse.start);
                }
                events += decode_ready_lines(receive, false, false, sink);
            }
        }

        self.working_sample += 1;
        events
    }

    fn try_sync_timing_lock(&mut self, sink: &mut impl DecodeSink) -> usize {
        let Some((confidence, mean_interval, offset, pulses)) = self.sync_detector.candidates(
            &mut self.candidate_scratch,
            &mut self.candidate_score_scratch,
        ) else {
            return 0;
        };
        let mut events = 0;
        if self.config.detect_sync_timing {
            sink.on_event(DecodeEventRef::ModeCandidate {
                candidates: &self.candidate_scratch,
                confidence,
            });
            events += 1;
        }
        if !self.config.detect_sync_timing && self.config.manual_mode.is_none() {
            return events;
        }
        let candidate_count = self.candidate_scratch.len().min(u8::MAX as usize) as u8;
        if self.config.manual_mode.is_none() && (candidate_count != 1 || confidence < 0.5) {
            return events;
        }
        let mode = self.config.manual_mode.unwrap_or(self.candidate_scratch[0]);
        if !self.candidate_scratch.contains(&mode) {
            return events;
        }
        let detection = if self.config.manual_mode.is_some() {
            DetectionSource::Manual
        } else {
            DetectionSource::SyncTiming {
                ambiguous: candidate_count > 1,
                candidate_count,
            }
        };
        events += self.begin_image(mode, detection, offset, sink);
        if let DecoderState::Receiving(receive) = &mut self.state {
            receive.line_period_samples = mean_interval;
            receive.next_line_deadline = mean_interval;
            let start = match mode.spec().layout {
                ScanLayout::Scottie { channel_seconds } => {
                    let before_sync = 2.0 * (mode.spec().porch_seconds + channel_seconds);
                    let nominal_period = mode.spec().line_seconds * f64::from(WORK_SAMPLE_RATE);
                    let clock_scale = mean_interval / nominal_period;
                    pulses[0].start.saturating_sub(
                        (before_sync * f64::from(WORK_SAMPLE_RATE) * clock_scale).round() as u64,
                    )
                }
                _ => pulses[0].start,
            };
            receive.line_buffer.clear();
            self.history
                .copy_from_absolute(start, &mut receive.line_buffer);
            receive.timeline_start = Some(start);
            receive.started = true;
            receive.last_sync_start = Some(pulses[5].start);
            receive.clock_next_index = 5;
            receive.sync_lines_confirmed = 5.min(receive.radio_line_count());
            for (index, pulse) in pulses.iter().enumerate() {
                receive.record_clock_point(index as u64, pulse.start);
            }
            events += decode_ready_lines(receive, false, false, sink);
        }
        events
    }

    fn begin_image(
        &mut self,
        mode: SstvMode,
        detection: DetectionSource,
        frequency_offset_hz: f32,
        sink: &mut impl DecodeSink,
    ) -> usize {
        let image_id = self.next_image_id;
        self.next_image_id = self.next_image_id.wrapping_add(1).max(1);
        let spec = mode.spec();
        sink.on_event(DecodeEventRef::ImageStarted {
            image_id,
            mode,
            detection,
            frequency_offset_hz,
            width: spec.width,
            height: spec.height,
        });
        // VIS stop and a sync-first mode can both be 1200 Hz. Reset the pulse
        // detector at the protocol boundary so they cannot merge into one
        // overlong low-frequency pulse.
        self.sync_detector.clear();
        self.state = DecoderState::Receiving(Box::new(ReceiveState::new(
            image_id,
            mode,
            frequency_offset_hz,
        )));
        1
    }
}

fn confirm_vis_timing(
    mode: SstvMode,
    body_start: u64,
    pulses: &mut VecDeque<SyncPulse>,
) -> Option<([SyncPulse; 3], f64)> {
    let expected = mode.spec().line_seconds * f64::from(WORK_SAMPLE_RATE);
    let tolerance = (expected * 0.03).max(samples(0.003) as f64);
    for window in pulses.make_contiguous().windows(3) {
        let first_period = window[1].start.saturating_sub(window[0].start) as f64;
        let second_period = window[2].start.saturating_sub(window[1].start) as f64;
        if (first_period - expected).abs() <= tolerance
            && (second_period - expected).abs() <= tolerance
        {
            let first_sync_offset = match mode.spec().layout {
                ScanLayout::Scottie { channel_seconds } => {
                    mode.spec().sync_seconds + 2.0 * (mode.spec().porch_seconds + channel_seconds)
                }
                _ => 0.0,
            } * f64::from(WORK_SAMPLE_RATE);
            let expected_first = body_start.saturating_add(first_sync_offset.round() as u64);
            let phase_tolerance = (expected * 0.02).max(samples(0.005) as f64);
            if window[0].start.abs_diff(expected_first) as f64 > phase_tolerance {
                continue;
            }
            return Some((
                [window[0], window[1], window[2]],
                (first_period + second_period) * 0.5,
            ));
        }
    }
    None
}

fn detect_header(history: &FrequencyHistory) -> Option<f32> {
    let trim = samples(0.004);
    let start = history.mean_from_end(trim, samples(0.022))?;
    let leader2 = history.mean_from_end(samples(0.030 + 0.025), samples(0.250))?;
    let break_tone = history.mean_from_end(samples(0.030 + 0.300 + 0.002), samples(0.006))?;
    let leader1 = history.mean_from_end(samples(0.030 + 0.300 + 0.010 + 0.025), samples(0.250))?;
    if !(1600.0..=2200.0).contains(&leader1) || (leader1 - leader2).abs() > 45.0 {
        return None;
    }
    let leader = (leader1 + leader2) * 0.5;
    if (break_tone - (leader - 700.0)).abs() > 80.0 || (start - (leader - 700.0)).abs() > 80.0 {
        return None;
    }
    Some(leader - 1900.0)
}

fn decode_ready_lines(
    receive: &mut ReceiveState,
    allow_unconfirmed_final: bool,
    at_end_of_input: bool,
    sink: &mut impl DecodeSink,
) -> usize {
    let mut events = 0;
    while receive.started && receive.radio_line < receive.radio_line_count() {
        let confirmed = receive.radio_line < receive.sync_lines_confirmed;
        let unconfirmed_final = allow_unconfirmed_final
            && receive.radio_line + 1 == receive.radio_line_count()
            && receive.radio_line == receive.sync_lines_confirmed;
        if !confirmed && !unconfirmed_final {
            break;
        }
        let deadline = receive
            .next_line_deadline
            .round()
            .max(receive.scheduled_line_samples as f64) as u64;
        let mut line_len = deadline.saturating_sub(receive.scheduled_line_samples) as usize;
        if receive.line_buffer.len() < line_len {
            let shortfall = line_len - receive.line_buffer.len();
            if at_end_of_input && unconfirmed_final && shortfall <= MAX_EOF_FILTER_DELAY_SAMPLES {
                line_len = receive.line_buffer.len();
            } else {
                break;
            }
        }
        let mut line = std::mem::take(&mut receive.line_decode_scratch);
        line.clear();
        line.extend_from_slice(&receive.line_buffer[..line_len]);
        events += decode_radio_line(receive, &line, sink);
        receive.line_decode_scratch = line;
        let remaining = receive.line_buffer.len() - line_len;
        receive.line_buffer.copy_within(line_len.., 0);
        receive.line_buffer.truncate(remaining);
        receive.scheduled_line_samples = deadline;
        receive.next_line_deadline += receive.line_period_samples;
        receive.radio_line += 1;
    }
    events
}

fn decode_radio_line(
    receive: &mut ReceiveState,
    line: &[f32],
    sink: &mut impl DecodeSink,
) -> usize {
    let spec = receive.mode.spec();
    let row = receive.radio_line * u32::from(spec.rows_per_line);
    match spec.layout {
        ScanLayout::Monochrome { scan_seconds } => {
            decode_channel(
                line,
                spec.line_seconds,
                spec.sync_seconds,
                scan_seconds,
                spec.width,
                receive.frequency_offset_hz,
                &mut receive.y_plane[row_offset(spec.width, row)..row_offset(spec.width, row + 1)],
            );
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            1
        }
        ScanLayout::Martin { channel_seconds } => {
            let mut offset = spec.sync_seconds + spec.porch_seconds;
            decode_rgb_channels(
                receive,
                line,
                row,
                &mut offset,
                channel_seconds,
                [ChannelIndex::Green, ChannelIndex::Blue, ChannelIndex::Red],
            );
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            1
        }
        ScanLayout::Scottie { channel_seconds } => {
            let mut offset = spec.porch_seconds;
            decode_rgb_channels(
                receive,
                line,
                row,
                &mut offset,
                channel_seconds,
                [ChannelIndex::Green, ChannelIndex::Blue, ChannelIndex::Red],
            );
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            1
        }
        ScanLayout::Robot { .. } => decode_robot_line(receive, line, row, sink),
        ScanLayout::Pd { channel_seconds } => {
            decode_pd_line(receive, line, row, channel_seconds);
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            emit_rgb_line(receive, row + 1, LineCompleteness::Final, sink);
            2
        }
        ScanLayout::Wraase {
            channel_seconds,
            outer_channel_scale,
        } => {
            let mut offset = spec.sync_seconds + spec.porch_seconds;
            decode_rgb_variable(
                receive,
                line,
                row,
                &mut offset,
                [
                    (ChannelIndex::Red, channel_seconds * outer_channel_scale),
                    (ChannelIndex::Green, channel_seconds),
                    (ChannelIndex::Blue, channel_seconds * outer_channel_scale),
                ],
            );
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            1
        }
        ScanLayout::Pasokon { channel_seconds } => {
            let mut offset = spec.sync_seconds + spec.porch_seconds;
            decode_rgb_channels(
                receive,
                line,
                row,
                &mut offset,
                channel_seconds,
                [ChannelIndex::Red, ChannelIndex::Green, ChannelIndex::Blue],
            );
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            1
        }
    }
}

#[derive(Clone, Copy)]
enum ChannelIndex {
    Red,
    Green,
    Blue,
}

fn decode_rgb_channels(
    receive: &mut ReceiveState,
    line: &[f32],
    row: u32,
    offset: &mut f64,
    channel_seconds: f64,
    channels: [ChannelIndex; 3],
) {
    let separator = receive.mode.spec().porch_seconds;
    for channel in channels {
        decode_rgb_channel(receive, line, row, *offset, channel_seconds, channel);
        *offset += channel_seconds + separator;
        if matches!(receive.mode.spec().layout, ScanLayout::Scottie { .. })
            && matches!(channel, ChannelIndex::Blue)
        {
            *offset += receive.mode.spec().sync_seconds;
        }
    }
}

fn decode_rgb_variable(
    receive: &mut ReceiveState,
    line: &[f32],
    row: u32,
    offset: &mut f64,
    channels: [(ChannelIndex, f64); 3],
) {
    for (channel, seconds) in channels {
        decode_rgb_channel(receive, line, row, *offset, seconds, channel);
        *offset += seconds;
    }
}

fn decode_rgb_channel(
    receive: &mut ReceiveState,
    line: &[f32],
    row: u32,
    start_seconds: f64,
    seconds: f64,
    channel: ChannelIndex,
) {
    let width = receive.mode.spec().width;
    let mut values = std::mem::take(&mut receive.channel_scratch);
    values.resize(width as usize, 0);
    decode_channel(
        line,
        receive.mode.spec().line_seconds,
        start_seconds,
        seconds,
        width,
        receive.frequency_offset_hz,
        &mut values,
    );
    let start = row_offset(width, row);
    for (index, value) in values.iter().copied().enumerate() {
        let target = &mut receive.line_scratch[index];
        match channel {
            ChannelIndex::Red => target.r = value,
            ChannelIndex::Green => target.g = value,
            ChannelIndex::Blue => target.b = value,
        }
    }
    // Preserve partial channel updates; the third channel completes the row.
    for (index, pixel) in receive.line_scratch.iter().enumerate() {
        receive.rgb_plane[start + index] = *pixel;
    }
    receive.channel_scratch = values;
}

fn decode_robot_line(
    receive: &mut ReceiveState,
    line: &[f32],
    row: u32,
    sink: &mut impl DecodeSink,
) -> usize {
    let spec = receive.mode.spec();
    let ScanLayout::Robot {
        luma_seconds,
        chroma_seconds,
        alternating_chroma: alternating,
        separator_seconds,
        chroma_porch_seconds,
    } = spec.layout
    else {
        unreachable!("decode_robot_line is called only for Robot layouts");
    };
    let width = spec.width;
    let start = row_offset(width, row);
    decode_channel(
        line,
        spec.line_seconds,
        spec.sync_seconds + spec.porch_seconds,
        luma_seconds,
        width,
        receive.frequency_offset_hz,
        &mut receive.y_plane[start..start + width as usize],
    );
    let chroma_start = spec.sync_seconds
        + spec.porch_seconds
        + luma_seconds
        + separator_seconds
        + chroma_porch_seconds;
    if alternating {
        if row % 2 == 0 {
            decode_channel(
                line,
                spec.line_seconds,
                chroma_start,
                chroma_seconds,
                width,
                receive.frequency_offset_hz,
                &mut receive.v_plane[start..start + width as usize],
            );
            if row + 1 < spec.height {
                let next = row_offset(width, row + 1);
                receive
                    .v_plane
                    .copy_within(start..start + width as usize, next);
            }
            emit_rgb_line(receive, row, LineCompleteness::Provisional, sink);
            1
        } else {
            decode_channel(
                line,
                spec.line_seconds,
                chroma_start,
                chroma_seconds,
                width,
                receive.frequency_offset_hz,
                &mut receive.u_plane[start..start + width as usize],
            );
            let previous = row_offset(width, row - 1);
            receive
                .u_plane
                .copy_within(start..start + width as usize, previous);
            emit_rgb_line(receive, row - 1, LineCompleteness::Final, sink);
            emit_rgb_line(receive, row, LineCompleteness::Final, sink);
            2
        }
    } else {
        decode_channel(
            line,
            spec.line_seconds,
            chroma_start,
            chroma_seconds,
            width,
            receive.frequency_offset_hz,
            &mut receive.v_plane[start..start + width as usize],
        );
        decode_channel(
            line,
            spec.line_seconds,
            chroma_start + chroma_seconds + separator_seconds + chroma_porch_seconds,
            chroma_seconds,
            width,
            receive.frequency_offset_hz,
            &mut receive.u_plane[start..start + width as usize],
        );
        emit_rgb_line(receive, row, LineCompleteness::Final, sink);
        1
    }
}

fn decode_pd_line(receive: &mut ReceiveState, line: &[f32], row: u32, channel_seconds: f64) {
    let spec = receive.mode.spec();
    let width = spec.width;
    let row0 = row_offset(width, row);
    let row1 = row_offset(width, row + 1);
    let mut offset = spec.sync_seconds + spec.porch_seconds;
    decode_channel(
        line,
        spec.line_seconds,
        offset,
        channel_seconds,
        width,
        receive.frequency_offset_hz,
        &mut receive.y_plane[row0..row0 + width as usize],
    );
    offset += channel_seconds;
    let mut v = std::mem::take(&mut receive.channel_scratch);
    v.resize(width as usize, 0);
    decode_channel(
        line,
        spec.line_seconds,
        offset,
        channel_seconds,
        width,
        receive.frequency_offset_hz,
        &mut v,
    );
    offset += channel_seconds;
    let mut u = std::mem::take(&mut receive.channel_scratch2);
    u.resize(width as usize, 0);
    decode_channel(
        line,
        spec.line_seconds,
        offset,
        channel_seconds,
        width,
        receive.frequency_offset_hz,
        &mut u,
    );
    offset += channel_seconds;
    decode_channel(
        line,
        spec.line_seconds,
        offset,
        channel_seconds,
        width,
        receive.frequency_offset_hz,
        &mut receive.y_plane[row1..row1 + width as usize],
    );
    receive.u_plane[row0..row0 + width as usize].copy_from_slice(&u);
    receive.u_plane[row1..row1 + width as usize].copy_from_slice(&u);
    receive.v_plane[row0..row0 + width as usize].copy_from_slice(&v);
    receive.v_plane[row1..row1 + width as usize].copy_from_slice(&v);
    receive.channel_scratch = v;
    receive.channel_scratch2 = u;
}

fn decode_channel(
    line: &[f32],
    nominal_line_seconds: f64,
    start_seconds: f64,
    duration_seconds: f64,
    width: u32,
    frequency_offset_hz: f32,
    output: &mut [u8],
) {
    let nominal_line_samples = nominal_line_seconds * f64::from(WORK_SAMPLE_RATE);
    let scale = line.len() as f64 / nominal_line_samples.max(1.0);
    let start = start_seconds * f64::from(WORK_SAMPLE_RATE) * scale;
    let duration = duration_seconds * f64::from(WORK_SAMPLE_RATE) * scale;
    for x in 0..width {
        let center = start + (f64::from(x) + 0.5) * duration / f64::from(width);
        let half_window = (duration / f64::from(width) * 0.25).max(1.0);
        let begin = (center - half_window).max(0.0) as usize;
        let end = ((center + half_window).ceil() as usize).min(line.len());
        let frequency = if begin < end {
            line[begin..end].iter().copied().sum::<f32>() / (end - begin) as f32
        } else {
            1500.0 + frequency_offset_hz
        };
        output[x as usize] = frequency_to_value(frequency, frequency_offset_hz);
    }
}

fn emit_rgb_line(
    receive: &mut ReceiveState,
    row: u32,
    completeness: LineCompleteness,
    sink: &mut impl DecodeSink,
) {
    let spec = receive.mode.spec();
    let width = spec.width as usize;
    let start = row as usize * width;
    if spec.color == ColorLayout::Rgb {
        receive
            .line_scratch
            .copy_from_slice(&receive.rgb_plane[start..start + width]);
    } else {
        for x in 0..width {
            receive.line_scratch[x] = yuv_to_rgb(Yuv {
                y: receive.y_plane[start + x],
                u: receive.u_plane[start + x],
                v: receive.v_plane[start + x],
            });
        }
    }
    let revision = receive.revisions[row as usize];
    receive.revisions[row as usize] = revision + 1;
    receive.last_emitted_row = Some(receive.last_emitted_row.map_or(row, |last| last.max(row)));
    sink.on_event(DecodeEventRef::LineReady {
        image_id: receive.image_id,
        mode: receive.mode,
        line_index: row,
        revision,
        completeness,
        pixels: &receive.line_scratch,
    });
}

#[inline]
fn frequency_to_value(frequency_hz: f32, offset_hz: f32) -> u8 {
    (((frequency_hz - offset_hz - 1500.0) * (255.0 / 800.0)).round() as i32).clamp(0, 255) as u8
}

#[inline]
fn row_offset(width: u32, row: u32) -> usize {
    width as usize * row as usize
}

#[inline]
fn samples(seconds: f64) -> usize {
    (seconds * f64::from(WORK_SAMPLE_RATE)).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_types_are_send_and_sync() {
        static_assertions::assert_impl_all!(SstvDecoder: Send, Sync);
        static_assertions::assert_impl_all!(DecoderConfig: Send, Sync, Copy);
        static_assertions::assert_impl_all!(DecodeEvent: Send, Sync);
    }

    #[test]
    fn empty_push_is_not_eof() {
        let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
        let mut events = Vec::new();
        decoder.process_into(&[], &mut events).unwrap();
        assert_eq!(decoder.sync_state(), SyncState::Searching);
        assert!(events.is_empty());
    }

    #[test]
    fn non_finite_pcm_is_rejected_without_advancing_state() {
        let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
        let mut events = Vec::new();
        assert_eq!(
            decoder.process_into(&[f32::NAN], &mut events),
            Err(Error::NonFiniteSample)
        );
        assert_eq!(decoder.sync_state(), SyncState::Searching);
        assert!(events.is_empty());
    }
}
