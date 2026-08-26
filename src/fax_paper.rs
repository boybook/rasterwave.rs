use crate::fax::{
    FaxClockCalibrationPoint, FaxClockRecoveryMode, FaxDecodeEvent, FaxDecodeEventRef, FaxDecoder,
    FaxDecoderConfig, FaxIoc, FaxLpm, FaxModulation, FaxRasterBasis, FaxSpec,
};
use crate::{Error, PaperBoundaryKind, Result};

/// How continuous radiofax paper chooses its active parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaxPaperMode {
    /// Print with fallback parameters while detecting IOC, LPM and FM/AM.
    Auto {
        /// Parameters used before APT and phasing establish a trusted page.
        fallback: FaxSpec,
    },
    /// Keep one selected specification while observing matching APT/phasing.
    Manual {
        /// Forced parameters.
        spec: FaxSpec,
    },
}

impl Default for FaxPaperMode {
    fn default() -> Self {
        Self::Auto {
            fallback: FaxSpec::standard(FaxIoc::Ioc576, FaxLpm::LPM_120),
        }
    }
}

/// Continuous radiofax paper decoder policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaxPaperConfig {
    /// Automatic fallback or manually locked parameters.
    pub mode: FaxPaperMode,
    /// Automatic line-clock and horizontal-phase recovery.
    pub clock_recovery: FaxClockRecoveryMode,
    /// AM candidate evaluated alongside the fallback FM detector in Auto.
    pub auto_am_modulation: FaxModulation,
    /// Full-scale amplitude used by AM decoding.
    pub am_full_scale: f32,
    /// Stable APT evidence required before IOC selection.
    pub apt_confirm_seconds: f32,
    /// Maximum APT/phasing acquisition time.
    pub acquisition_timeout_seconds: f32,
    /// Expected phasing duration, or `None` for pattern-end detection.
    pub expected_phasing_seconds: Option<f32>,
    /// Stable APT stop evidence required to end a capture.
    pub stop_confirm_seconds: f32,
    /// Signal-loss timeout used by acquisition detectors only.
    pub signal_loss_seconds: f32,
    /// Minimum PCM signal level used by acquisition detectors.
    pub minimum_signal_level: f32,
    /// Minimum target-subcarrier coherence.
    pub minimum_carrier_coherence: f32,
}

impl Default for FaxPaperConfig {
    fn default() -> Self {
        let defaults = FaxDecoderConfig::default();
        Self {
            mode: FaxPaperMode::default(),
            clock_recovery: FaxClockRecoveryMode::Auto,
            auto_am_modulation: FaxModulation::AmSubcarrier {
                carrier_hz: 1900.0,
                black_level: 0.0,
                white_level: 1.0,
            },
            am_full_scale: defaults.am_full_scale,
            apt_confirm_seconds: defaults.apt_confirm_seconds,
            acquisition_timeout_seconds: defaults.acquisition_timeout_seconds,
            expected_phasing_seconds: defaults.expected_phasing_seconds,
            stop_confirm_seconds: defaults.stop_confirm_seconds,
            signal_loss_seconds: defaults.signal_loss_seconds,
            minimum_signal_level: defaults.minimum_signal_level,
            minimum_carrier_coherence: defaults.minimum_carrier_coherence,
        }
    }
}

/// Borrowed event emitted by [`FaxPaperDecoder`].
#[derive(Debug)]
#[non_exhaustive]
pub enum FaxPaperEventRef<'a> {
    /// An indefinitely growing fax paper raster began.
    PaperStarted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Initial fallback or manually selected parameters.
        spec: FaxSpec,
    },
    /// A meaningful divider in the fax paper.
    Boundary {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Monotonic boundary identifier.
        boundary_id: u64,
        /// First row after the divider.
        line_index: u64,
        /// Parameters active after the divider.
        spec: FaxSpec,
        /// Divider classification.
        kind: PaperBoundaryKind,
        /// Whether the divider anchors an automatic capture.
        trusted: bool,
    },
    /// APT identified an IOC candidate.
    AptDetected {
        /// Candidate IOC.
        ioc: FaxIoc,
    },
    /// A clock model for correcting the current nominal paper segment.
    ClockCalibration {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Segment boundary to which the calibration applies.
        boundary_id: u64,
        /// Calibration control point.
        calibration: FaxClockCalibrationPoint,
    },
    /// One continuous-paper grayscale row is ready.
    LineReady {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Current segment boundary.
        boundary_id: u64,
        /// Monotonic paper row.
        line_index: u64,
        /// Row within the current segment.
        segment_line_index: u32,
        /// Active fax parameters.
        spec: FaxSpec,
        /// Grayscale pixels, valid until the callback returns.
        pixels: &'a [u8],
        /// Coordinate basis of the row.
        basis: FaxRasterBasis,
    },
    /// A trusted APT-delimited fax capture completed.
    TransmissionCompleted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary that opened the capture.
        boundary_id: u64,
        /// First captured row.
        start_line: u64,
        /// Exclusive captured end.
        end_line: u64,
        /// Active parameters.
        spec: FaxSpec,
        /// Number of completed rows.
        lines: u32,
    },
    /// A valid fax header was rejected by a manual lock.
    ProtocolObserved {
        /// Observed parameters.
        spec: FaxSpec,
        /// Whether APT and phasing fully established the parameters.
        trusted: bool,
    },
    /// APT or phasing acquisition was rejected.
    SignalRejected {
        /// Stable reason.
        reason: &'static str,
    },
}

/// Owned form of [`FaxPaperEventRef`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FaxPaperEvent {
    /// A fax paper raster began.
    PaperStarted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Initial parameters.
        spec: FaxSpec,
    },
    /// A fax paper divider.
    Boundary {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary identifier.
        boundary_id: u64,
        /// First row after the divider.
        line_index: u64,
        /// Active parameters.
        spec: FaxSpec,
        /// Divider classification.
        kind: PaperBoundaryKind,
        /// Whether the divider anchors an automatic capture.
        trusted: bool,
    },
    /// APT identified an IOC candidate.
    AptDetected {
        /// Candidate IOC.
        ioc: FaxIoc,
    },
    /// An owned clock calibration point.
    ClockCalibration {
        /// Paper identifier.
        paper_id: u64,
        /// Segment boundary identifier.
        boundary_id: u64,
        /// Calibration control point.
        calibration: FaxClockCalibrationPoint,
    },
    /// One owned fax paper row.
    LineReady {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Current segment boundary.
        boundary_id: u64,
        /// Monotonic paper row.
        line_index: u64,
        /// Row within the current segment.
        segment_line_index: u32,
        /// Active parameters.
        spec: FaxSpec,
        /// Owned grayscale pixels.
        pixels: Vec<u8>,
        /// Coordinate basis of the row.
        basis: FaxRasterBasis,
    },
    /// A trusted fax transmission completed.
    TransmissionCompleted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary that opened the capture.
        boundary_id: u64,
        /// First captured row.
        start_line: u64,
        /// Exclusive captured end.
        end_line: u64,
        /// Active parameters.
        spec: FaxSpec,
        /// Number of completed rows.
        lines: u32,
    },
    /// A fax protocol was observed but rejected.
    ProtocolObserved {
        /// Observed parameters.
        spec: FaxSpec,
        /// Whether the observation was trusted.
        trusted: bool,
    },
    /// Acquisition was rejected.
    SignalRejected {
        /// Stable reason.
        reason: &'static str,
    },
}

impl FaxPaperEventRef<'_> {
    /// Copy a borrowed event into an owned value.
    pub fn to_owned(&self) -> FaxPaperEvent {
        match self {
            Self::PaperStarted { paper_id, spec } => FaxPaperEvent::PaperStarted {
                paper_id: *paper_id,
                spec: *spec,
            },
            Self::Boundary {
                paper_id,
                boundary_id,
                line_index,
                spec,
                kind,
                trusted,
            } => FaxPaperEvent::Boundary {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                line_index: *line_index,
                spec: *spec,
                kind: *kind,
                trusted: *trusted,
            },
            Self::AptDetected { ioc } => FaxPaperEvent::AptDetected { ioc: *ioc },
            Self::ClockCalibration {
                paper_id,
                boundary_id,
                calibration,
            } => FaxPaperEvent::ClockCalibration {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                calibration: *calibration,
            },
            Self::LineReady {
                paper_id,
                boundary_id,
                line_index,
                segment_line_index,
                spec,
                pixels,
                basis,
            } => FaxPaperEvent::LineReady {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                line_index: *line_index,
                segment_line_index: *segment_line_index,
                spec: *spec,
                pixels: pixels.to_vec(),
                basis: *basis,
            },
            Self::TransmissionCompleted {
                paper_id,
                boundary_id,
                start_line,
                end_line,
                spec,
                lines,
            } => FaxPaperEvent::TransmissionCompleted {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                start_line: *start_line,
                end_line: *end_line,
                spec: *spec,
                lines: *lines,
            },
            Self::ProtocolObserved { spec, trusted } => FaxPaperEvent::ProtocolObserved {
                spec: *spec,
                trusted: *trusted,
            },
            Self::SignalRejected { reason } => FaxPaperEvent::SignalRejected { reason },
        }
    }
}

/// Synchronous consumer for borrowed continuous fax events.
pub trait FaxPaperSink {
    /// Handle one event before borrowed pixels are reused.
    fn on_event(&mut self, event: FaxPaperEventRef<'_>);
}

impl<F> FaxPaperSink for F
where
    F: for<'a> FnMut(FaxPaperEventRef<'a>),
{
    fn on_event(&mut self, event: FaxPaperEventRef<'_>) {
        self(event);
    }
}

#[derive(Debug)]
struct Detector {
    decoder: FaxDecoder,
}

#[derive(Debug)]
struct Capture {
    detector: usize,
    page_id: u64,
    boundary_id: u64,
    start_line: u64,
    spec: FaxSpec,
}

const DEAD_SECTOR_TRACKING_LINES: usize = 64;
const DEAD_SECTOR_TRACKING_STRIDE: u64 = 32;
const DEAD_SECTOR_MAX_PPM: i32 = 5_000;
const DEAD_SECTOR_COARSE_PPM_STEP: i32 = 100;
const DEAD_SECTOR_FINE_PPM_STEP: i32 = 10;
const DEAD_SECTOR_ULTRA_FINE_PPM_STEP: i32 = 1;

#[derive(Clone, Copy, Debug)]
struct DeadSectorCandidate {
    sector_start: usize,
    clock_ppm: f64,
    score: f64,
}

#[derive(Clone, Copy, Debug)]
struct DeadSectorMeasurement {
    reference_line: u64,
    phase_pixels: f64,
    clock_ppm: f64,
    confidence: f64,
}

#[derive(Debug, Default)]
struct DeadSectorClockTracker {
    rows: std::collections::VecDeque<(u64, Vec<u8>)>,
    measurements: std::collections::VecDeque<DeadSectorMeasurement>,
    revision: u32,
    last_emitted: Option<FaxClockCalibrationPoint>,
}

impl DeadSectorClockTracker {
    fn reset(&mut self, initial: Option<FaxClockCalibrationPoint>) {
        self.rows.clear();
        self.measurements.clear();
        self.revision = initial.map_or(0, |point| point.revision);
        self.last_emitted = initial;
    }

    fn observe(
        &mut self,
        line_index: u64,
        spec: FaxSpec,
        pixels: &[u8],
    ) -> Option<FaxClockCalibrationPoint> {
        if pixels.len() != spec.width() as usize {
            return None;
        }
        self.rows.push_back((line_index, pixels.to_vec()));
        while self.rows.len() > DEAD_SECTOR_TRACKING_LINES {
            self.rows.pop_front();
        }
        if self.rows.len() < DEAD_SECTOR_TRACKING_LINES
            || line_index % DEAD_SECTOR_TRACKING_STRIDE != 0
        {
            return None;
        }
        let width = spec.width() as usize;
        let sector = width.saturating_sub(spec.active_width() as usize).max(1);
        let mut minimum = u8::MAX;
        let mut maximum = u8::MIN;
        for (_, row) in &self.rows {
            for value in row.iter().copied() {
                minimum = minimum.min(value);
                maximum = maximum.max(value);
            }
        }
        if maximum.saturating_sub(minimum) < 40 {
            return None;
        }
        let prior_sector_start = self.last_emitted.map(|previous| {
            let predicted_phase = f64::from(previous.phase_pixels)
                + line_index.saturating_sub(previous.reference_line) as f64
                    * f64::from(previous.clock_ppm)
                    * width as f64
                    / 1_000_000.0;
            wrap_column(
                (f64::from(spec.active_width()) - predicted_phase).round() as i64,
                width,
            )
        });
        let mut candidates = Vec::new();
        let has_tracking_lock = self
            .last_emitted
            .is_some_and(|point| point.source == crate::fax::FaxClockSource::DeadSector);
        let (coarse_start, coarse_end, coarse_step) = if has_tracking_lock {
            let previous = self.last_emitted.expect("tracking lock exists");
            let center = previous.clock_ppm.round() as i32;
            (
                (center - 250).max(-DEAD_SECTOR_MAX_PPM),
                (center + 250).min(DEAD_SECTOR_MAX_PPM),
                25,
            )
        } else {
            (
                -DEAD_SECTOR_MAX_PPM,
                DEAD_SECTOR_MAX_PPM,
                DEAD_SECTOR_COARSE_PPM_STEP,
            )
        };
        for ppm in (coarse_start..=coarse_end).step_by(coarse_step as usize) {
            candidates.extend(self.evaluate_clock(spec, f64::from(ppm), false, prior_sector_start));
        }
        let coarse = candidates
            .iter()
            .copied()
            .max_by(|left, right| left.score.total_cmp(&right.score))?;
        candidates.clear();
        let fine_radius = if has_tracking_lock {
            25
        } else {
            DEAD_SECTOR_COARSE_PPM_STEP
        };
        let fine_step = if has_tracking_lock {
            5
        } else {
            DEAD_SECTOR_FINE_PPM_STEP
        };
        let fine_start = (coarse.clock_ppm as i32 - fine_radius).max(-DEAD_SECTOR_MAX_PPM);
        let fine_end = (coarse.clock_ppm as i32 + fine_radius).min(DEAD_SECTOR_MAX_PPM);
        for ppm in (fine_start..=fine_end).step_by(fine_step as usize) {
            candidates.extend(self.evaluate_clock(spec, f64::from(ppm), false, prior_sector_start));
        }
        let fine = candidates
            .iter()
            .copied()
            .max_by(|left, right| left.score.total_cmp(&right.score))?;
        candidates.clear();
        let ultra_radius = if has_tracking_lock {
            5
        } else {
            DEAD_SECTOR_FINE_PPM_STEP
        };
        let ultra_start = (fine.clock_ppm as i32 - ultra_radius).max(-DEAD_SECTOR_MAX_PPM);
        let ultra_end = (fine.clock_ppm as i32 + ultra_radius).min(DEAD_SECTOR_MAX_PPM);
        for ppm in (ultra_start..=ultra_end).step_by(DEAD_SECTOR_ULTRA_FINE_PPM_STEP as usize) {
            candidates.extend(self.evaluate_clock(spec, f64::from(ppm), true, prior_sector_start));
        }
        candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
        let best = *candidates.first()?;
        let second = candidates.iter().copied().find(|candidate| {
            circular_distance(candidate.sector_start, best.sector_start, width) >= sector / 2
                || (candidate.clock_ppm - best.clock_ppm).abs() >= 250.0
        });
        let strength = best.score.clamp(0.0, 1.0);
        let second_score = second.map_or(0.0, |candidate| candidate.score);
        let uniqueness = ((best.score - second_score) / best.score.abs().max(0.01)).clamp(0.0, 1.0);
        let confidence = strength * 0.72 + uniqueness * 0.28;
        if confidence < 0.47 {
            return None;
        }
        let desired = spec.active_width() as f64;
        let mut phase = desired - best.sector_start as f64;
        let half = width as f64 * 0.5;
        while phase > half {
            phase -= width as f64;
        }
        while phase < -half {
            phase += width as f64;
        }
        let continuity = self
            .measurements
            .back()
            .map(|measurement| {
                measurement.phase_pixels
                    + line_index.saturating_sub(measurement.reference_line) as f64
                        * measurement.clock_ppm
                        * width as f64
                        / 1_000_000.0
            })
            .or_else(|| {
                self.last_emitted.map(|previous| {
                    f64::from(previous.phase_pixels)
                        + line_index.saturating_sub(previous.reference_line) as f64
                            * f64::from(previous.clock_ppm)
                            * width as f64
                            / 1_000_000.0
                })
            });
        if let Some(predicted) = continuity {
            while phase - predicted > half {
                phase -= width as f64;
            }
            while phase - predicted < -half {
                phase += width as f64;
            }
        }
        self.measurements.push_back(DeadSectorMeasurement {
            reference_line: line_index,
            phase_pixels: phase,
            clock_ppm: best.clock_ppm,
            confidence,
        });
        while self.measurements.len() > 5 {
            self.measurements.pop_front();
        }
        if self.measurements.len() < 3 {
            return None;
        }
        let mut ppm_values: Vec<_> = self
            .measurements
            .iter()
            .map(|measurement| measurement.clock_ppm)
            .collect();
        let clock_ppm = median_value(&mut ppm_values)?;
        let mut projected_phases: Vec<_> = self
            .measurements
            .iter()
            .map(|measurement| {
                measurement.phase_pixels
                    + line_index.saturating_sub(measurement.reference_line) as f64
                        * measurement.clock_ppm
                        * width as f64
                        / 1_000_000.0
            })
            .collect();
        let phase_anchor = *projected_phases.last()?;
        for projected in &mut projected_phases {
            while *projected - phase_anchor > half {
                *projected -= width as f64;
            }
            while *projected - phase_anchor < -half {
                *projected += width as f64;
            }
        }
        phase = median_value(&mut projected_phases)?;
        let mut ppm_deviations: Vec<_> = self
            .measurements
            .iter()
            .map(|measurement| (measurement.clock_ppm - clock_ppm).abs())
            .collect();
        let mut phase_deviations: Vec<_> = projected_phases
            .iter()
            .map(|projected| (*projected - phase).abs())
            .collect();
        let ppm_mad = median_value(&mut ppm_deviations).unwrap_or(f64::INFINITY);
        let phase_mad = median_value(&mut phase_deviations).unwrap_or(f64::INFINITY);
        if ppm_mad > 60.0 || phase_mad > (sector as f64 * 0.15).max(4.0) {
            return None;
        }
        let mean_confidence = self
            .measurements
            .iter()
            .map(|measurement| measurement.confidence)
            .sum::<f64>()
            / self.measurements.len() as f64;
        let consistency = (1.0 - ppm_mad / 60.0).clamp(0.0, 1.0) * 0.5
            + (1.0 - phase_mad / (sector as f64 * 0.15).max(4.0)).clamp(0.0, 1.0) * 0.5;
        let confidence = (mean_confidence * 0.75 + consistency * 0.25) as f32;
        if confidence < 0.58 {
            return None;
        }
        let point = FaxClockCalibrationPoint {
            revision: self.revision.saturating_add(1),
            reference_line: line_index,
            phase_pixels: phase as f32,
            clock_ppm: clock_ppm as f32,
            confidence,
            source: crate::fax::FaxClockSource::DeadSector,
            status: crate::fax::FaxClockStatus::Tracking,
        };
        let changed = self.last_emitted.is_none_or(|previous| {
            (previous.phase_pixels - point.phase_pixels).abs() >= 0.25
                || (previous.clock_ppm - point.clock_ppm).abs() >= 5.0
        });
        if !changed {
            return None;
        }
        self.revision = point.revision;
        self.last_emitted = Some(point);
        Some(point)
    }

    fn evaluate_clock(
        &self,
        spec: FaxSpec,
        correction_ppm: f64,
        interpolate: bool,
        prior_sector_start: Option<usize>,
    ) -> [DeadSectorCandidate; 2] {
        let width = spec.width() as usize;
        let sector = width.saturating_sub(spec.active_width() as usize).max(1);
        let reference_line = self.rows.back().map_or(0, |(line, _)| *line);
        let mut sums = vec![0.0_f64; width];
        let mut squares = vec![0.0_f64; width];
        for (line, row) in &self.rows {
            let delta = *line as f64 - reference_line as f64;
            let shift = correction_ppm * width as f64 * delta / 1_000_000.0;
            for column in 0..width {
                let source = column as f64 - shift;
                let left_unwrapped = source.floor();
                let fraction = source - left_unwrapped;
                let left = wrap_column(left_unwrapped as i64, width);
                let value = if interpolate {
                    let right = (left + 1) % width;
                    f64::from(row[left]) + (f64::from(row[right]) - f64::from(row[left])) * fraction
                } else {
                    f64::from(row[wrap_column(source.round() as i64, width)])
                };
                sums[column] += value;
                squares[column] += value * value;
            }
        }
        let count = self.rows.len() as f64;
        let means: Vec<_> = sums.iter().map(|sum| *sum / count).collect();
        let column_scores: Vec<_> = sums
            .iter()
            .zip(&squares)
            .map(|(sum, square)| column_score(*sum, *square, count))
            .collect();
        let mean_prefix = circular_prefix(&means);
        let mean_square_prefix =
            circular_prefix(&means.iter().map(|value| value * value).collect::<Vec<_>>());
        let score_prefix = circular_prefix(&column_scores);
        let edge = (sector / 4).max(4);
        let mut scored = Vec::with_capacity(width);
        for start in 0..width {
            let band_score = window_sum(&score_prefix, start, sector) / sector as f64;
            let band_mean = window_sum(&mean_prefix, start, sector) / sector as f64;
            let band_square = window_sum(&mean_square_prefix, start, sector) / sector as f64;
            let spatial_variance = (band_square - band_mean * band_mean).max(0.0);
            let uniformity = (1.0 - spatial_variance.sqrt() / 96.0).clamp(0.0, 1.0);
            let before = window_sum(&mean_prefix, start + width - edge, edge) / edge as f64;
            let after = window_sum(&mean_prefix, start + sector, edge) / edge as f64;
            let edge_contrast =
                (((band_mean - before).abs() + (band_mean - after).abs()) / 510.0).clamp(0.0, 1.0);
            let evidence =
                (band_score * 0.62 + uniformity * 0.13 + edge_contrast * 0.25).clamp(0.0, 1.0);
            let score = if let Some(prior) = prior_sector_start {
                let alignment = (1.0
                    - circular_distance(start, prior, width) as f64 / (sector * 3) as f64)
                    .clamp(0.0, 1.0);
                evidence * 0.82 + alignment * 0.18
            } else {
                evidence
            };
            scored.push((start, score));
        }
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        let best = scored[0];
        let second = scored
            .iter()
            .copied()
            .find(|candidate| circular_distance(candidate.0, best.0, width) >= sector / 2)
            .unwrap_or(best);
        [
            DeadSectorCandidate {
                sector_start: best.0,
                clock_ppm: correction_ppm,
                score: best.1,
            },
            DeadSectorCandidate {
                sector_start: second.0,
                clock_ppm: correction_ppm,
                score: second.1,
            },
        ]
    }
}

fn column_score(sum: f64, squares: f64, count: f64) -> f64 {
    let mean = sum / count;
    let variance = (squares / count - mean * mean).max(0.0);
    let extreme = ((mean - 127.5).abs() / 127.5).clamp(0.0, 1.0);
    let stability = (1.0 - variance.sqrt() / 96.0).clamp(0.0, 1.0);
    extreme * 0.58 + stability * 0.42
}

fn circular_prefix(values: &[f64]) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(values.len() * 2 + 1);
    prefix.push(0.0);
    for value in values.iter().chain(values) {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + value);
    }
    prefix
}

fn window_sum(prefix: &[f64], start: usize, length: usize) -> f64 {
    prefix[start + length] - prefix[start]
}

fn wrap_column(column: i64, width: usize) -> usize {
    column.rem_euclid(width as i64) as usize
}

fn circular_distance(left: usize, right: usize, width: usize) -> usize {
    let direct = left.abs_diff(right);
    direct.min(width.saturating_sub(direct))
}

fn median_value(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) * 0.5
    } else {
        values[middle]
    })
}

/// Continuous radiofax raster with parallel APT/phasing acquisition.
#[derive(Debug)]
pub struct FaxPaperDecoder {
    input_sample_rate: u32,
    config: FaxPaperConfig,
    detectors: Vec<Detector>,
    raster: FaxDecoder,
    paper_id: u64,
    next_boundary_id: u64,
    active_boundary_id: u64,
    paper_line: u64,
    current_spec: FaxSpec,
    raster_page_id: Option<u64>,
    raster_segment_start: u64,
    capture: Option<Capture>,
    started: bool,
    finished: bool,
    dead_sector_clock: DeadSectorClockTracker,
}

impl FaxPaperDecoder {
    /// Construct a continuous radiofax paper decoder.
    pub fn new(input_sample_rate: u32, config: FaxPaperConfig) -> Result<Self> {
        let initial_spec = match config.mode {
            FaxPaperMode::Auto { fallback } => fallback,
            FaxPaperMode::Manual { spec } => spec,
        };
        let raster = immediate_decoder(input_sample_rate, initial_spec, config.am_full_scale)?;
        let detectors = build_detectors(input_sample_rate, config)?;
        Ok(Self {
            input_sample_rate,
            config,
            detectors,
            raster,
            paper_id: 1,
            next_boundary_id: 2,
            active_boundary_id: 1,
            paper_line: 0,
            current_spec: initial_spec,
            raster_page_id: None,
            raster_segment_start: 0,
            capture: None,
            started: false,
            finished: false,
            dead_sector_clock: DeadSectorClockTracker::default(),
        })
    }

    /// Push arbitrary-sized mono PCM and emit fax rows before returning.
    pub fn push_f32(&mut self, input: &[f32], sink: &mut impl FaxPaperSink) -> Result<usize> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(Error::NonFiniteSample);
        }
        if input.is_empty() {
            return Ok(0);
        }
        let mut emitted = 0;
        if !self.started {
            self.started = true;
            sink.on_event(FaxPaperEventRef::PaperStarted {
                paper_id: self.paper_id,
                spec: self.current_spec,
            });
            sink.on_event(FaxPaperEventRef::Boundary {
                paper_id: self.paper_id,
                boundary_id: self.active_boundary_id,
                line_index: 0,
                spec: self.current_spec,
                kind: PaperBoundaryKind::Initial,
                trusted: false,
            });
            emitted += 2;
        }
        for chunk in input.chunks(64) {
            let capture_was_active = self.capture.is_some();
            let mut claimed = capture_was_active;
            for detector_index in 0..self.detectors.len() {
                let mut events = Vec::new();
                self.detectors[detector_index]
                    .decoder
                    .process_into(chunk, &mut events)?;
                for event in events {
                    emitted +=
                        self.handle_detector_event(detector_index, event, &mut claimed, sink)?;
                }
            }
            if !claimed && self.capture.is_none() {
                let mut events = Vec::new();
                self.raster.process_into(chunk, &mut events)?;
                for event in events {
                    emitted += self.handle_raster_event(event, sink);
                }
            }
        }
        Ok(emitted)
    }

    /// Collect owned events for one PCM chunk.
    pub fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<FaxPaperEvent>,
    ) -> Result<usize> {
        let mut sink = |event: FaxPaperEventRef<'_>| output.push(event.to_owned());
        self.push_f32(input, &mut sink)
    }

    /// Finalize input without completing an untrusted or partial capture.
    pub fn finish(&mut self, sink: &mut impl FaxPaperSink) -> Result<usize> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut emitted = 0;
        for detector_index in 0..self.detectors.len() {
            let mut events = Vec::new();
            self.detectors[detector_index]
                .decoder
                .finish(&mut |event: FaxDecodeEventRef<'_>| events.push(event.to_owned()))?;
            let mut claimed = self.capture.is_some();
            for event in events {
                emitted += self.handle_detector_event(detector_index, event, &mut claimed, sink)?;
            }
        }
        let mut ignored = Vec::new();
        let _ = self
            .raster
            .finish(&mut |event: FaxDecodeEventRef<'_>| ignored.push(event.to_owned()));
        self.finished = true;
        Ok(emitted)
    }

    /// End receiver continuity without completing or persisting a page.
    pub fn mark_signal_lost(&mut self, sink: &mut impl FaxPaperSink) -> Result<usize> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        self.rebuild(PaperBoundaryKind::Discontinuity, sink)?;
        Ok(1)
    }

    /// Reset acquisition and continue the same paper with an explicit divider.
    pub fn reset(&mut self, sink: &mut impl FaxPaperSink) -> Result<usize> {
        self.finished = false;
        self.rebuild(PaperBoundaryKind::Reset, sink)?;
        Ok(1)
    }

    /// Monotonic row after the latest emitted paper row.
    pub const fn next_line_index(&self) -> u64 {
        self.paper_line
    }

    fn handle_detector_event(
        &mut self,
        detector_index: usize,
        event: FaxDecodeEvent,
        claimed: &mut bool,
        sink: &mut impl FaxPaperSink,
    ) -> Result<usize> {
        match event {
            FaxDecodeEvent::AptDetected { ioc } => {
                sink.on_event(FaxPaperEventRef::AptDetected { ioc });
                Ok(1)
            }
            FaxDecodeEvent::PageStarted {
                page_id,
                spec,
                mut clock,
            } => {
                if self.capture.is_some() {
                    return Ok(0);
                }
                let accepted = match self.config.mode {
                    FaxPaperMode::Auto { .. } => true,
                    FaxPaperMode::Manual { spec: locked } => same_decode_spec(locked, spec),
                };
                if !accepted {
                    sink.on_event(FaxPaperEventRef::ProtocolObserved {
                        spec,
                        trusted: true,
                    });
                    return Ok(1);
                }
                *claimed = true;
                let boundary_id = self.next_boundary_id;
                self.next_boundary_id += 1;
                self.active_boundary_id = boundary_id;
                self.current_spec = spec;
                self.capture = Some(Capture {
                    detector: detector_index,
                    page_id,
                    boundary_id,
                    start_line: self.paper_line,
                    spec,
                });
                sink.on_event(FaxPaperEventRef::Boundary {
                    paper_id: self.paper_id,
                    boundary_id,
                    line_index: self.paper_line,
                    spec,
                    kind: PaperBoundaryKind::AptPhasing,
                    trusted: true,
                });
                clock.reference_line = self.paper_line;
                self.dead_sector_clock.reset(Some(clock));
                sink.on_event(FaxPaperEventRef::ClockCalibration {
                    paper_id: self.paper_id,
                    boundary_id,
                    calibration: clock,
                });
                Ok(2)
            }
            FaxDecodeEvent::LineReady {
                page_id,
                line_index,
                pixels,
                basis,
            } => {
                let Some(capture) = self.capture.as_ref().filter(|capture| {
                    capture.detector == detector_index && capture.page_id == page_id
                }) else {
                    return Ok(0);
                };
                *claimed = true;
                let paper_line = capture.start_line + u64::from(line_index);
                self.paper_line = self.paper_line.max(paper_line + 1);
                sink.on_event(FaxPaperEventRef::LineReady {
                    paper_id: self.paper_id,
                    boundary_id: capture.boundary_id,
                    line_index: paper_line,
                    segment_line_index: line_index,
                    spec: capture.spec,
                    pixels: &pixels,
                    basis,
                });
                let calibration = self
                    .dead_sector_clock
                    .observe(paper_line, capture.spec, &pixels);
                if let Some(calibration) = calibration {
                    sink.on_event(FaxPaperEventRef::ClockCalibration {
                        paper_id: self.paper_id,
                        boundary_id: capture.boundary_id,
                        calibration,
                    });
                    Ok(2)
                } else {
                    Ok(1)
                }
            }
            FaxDecodeEvent::PageCompleted {
                page_id,
                lines,
                partial,
            } => {
                let Some(capture) = self.capture.take().filter(|capture| {
                    capture.detector == detector_index && capture.page_id == page_id
                }) else {
                    return Ok(0);
                };
                *claimed = true;
                if partial {
                    self.raster = immediate_decoder(
                        self.input_sample_rate,
                        self.current_spec,
                        self.config.am_full_scale,
                    )?;
                    self.raster_page_id = None;
                    self.raster_segment_start = self.paper_line;
                    self.emit_boundary(PaperBoundaryKind::Discontinuity, sink);
                    return Ok(1);
                }
                let end_line = capture.start_line + u64::from(lines);
                self.paper_line = self.paper_line.max(end_line);
                sink.on_event(FaxPaperEventRef::TransmissionCompleted {
                    paper_id: self.paper_id,
                    boundary_id: capture.boundary_id,
                    start_line: capture.start_line,
                    end_line,
                    spec: capture.spec,
                    lines,
                });
                self.raster = immediate_decoder(
                    self.input_sample_rate,
                    capture.spec,
                    self.config.am_full_scale,
                )?;
                self.raster_page_id = None;
                self.raster_segment_start = end_line;
                self.emit_boundary(PaperBoundaryKind::ProtocolEnd, sink);
                Ok(2)
            }
            FaxDecodeEvent::SignalRejected { reason } => {
                sink.on_event(FaxPaperEventRef::SignalRejected { reason });
                Ok(1)
            }
            FaxDecodeEvent::PhasingLocked { .. } => Ok(0),
        }
    }

    fn handle_raster_event(
        &mut self,
        event: FaxDecodeEvent,
        sink: &mut impl FaxPaperSink,
    ) -> usize {
        match event {
            FaxDecodeEvent::PageStarted { page_id, spec, .. } => {
                self.raster_page_id = Some(page_id);
                self.raster_segment_start = self.paper_line;
                self.current_spec = spec;
                0
            }
            FaxDecodeEvent::LineReady {
                page_id,
                line_index,
                pixels,
                basis,
            } if self.raster_page_id == Some(page_id) => {
                let paper_line = self.raster_segment_start + u64::from(line_index);
                self.paper_line = self.paper_line.max(paper_line + 1);
                sink.on_event(FaxPaperEventRef::LineReady {
                    paper_id: self.paper_id,
                    boundary_id: self.active_boundary_id,
                    line_index: paper_line,
                    segment_line_index: line_index,
                    spec: self.current_spec,
                    pixels: &pixels,
                    basis,
                });
                if let Some(calibration) =
                    self.dead_sector_clock
                        .observe(paper_line, self.current_spec, &pixels)
                {
                    sink.on_event(FaxPaperEventRef::ClockCalibration {
                        paper_id: self.paper_id,
                        boundary_id: self.active_boundary_id,
                        calibration,
                    });
                    2
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    fn rebuild(&mut self, kind: PaperBoundaryKind, sink: &mut impl FaxPaperSink) -> Result<()> {
        self.capture = None;
        self.raster = immediate_decoder(
            self.input_sample_rate,
            self.current_spec,
            self.config.am_full_scale,
        )?;
        self.detectors = build_detectors(self.input_sample_rate, self.config)?;
        self.raster_page_id = None;
        self.raster_segment_start = self.paper_line;
        self.emit_boundary(kind, sink);
        Ok(())
    }

    fn emit_boundary(&mut self, kind: PaperBoundaryKind, sink: &mut impl FaxPaperSink) {
        let boundary_id = self.next_boundary_id;
        self.next_boundary_id += 1;
        self.active_boundary_id = boundary_id;
        self.dead_sector_clock.reset(None);
        sink.on_event(FaxPaperEventRef::Boundary {
            paper_id: self.paper_id,
            boundary_id,
            line_index: self.paper_line,
            spec: self.current_spec,
            kind,
            trusted: false,
        });
    }
}

fn immediate_decoder(
    input_sample_rate: u32,
    spec: FaxSpec,
    am_full_scale: f32,
) -> Result<FaxDecoder> {
    FaxDecoder::new(
        input_sample_rate,
        FaxDecoderConfig {
            immediate_decode: true,
            clock_recovery: FaxClockRecoveryMode::Off,
            raster_basis: FaxRasterBasis::NominalPaper,
            ioc: Some(spec.ioc),
            lpm: Some(spec.lpm),
            modulation: spec.modulation,
            max_lines: None,
            am_full_scale,
            minimum_signal_level: 0.0,
            minimum_carrier_coherence: 0.0,
            ..FaxDecoderConfig::default()
        },
    )
}

fn build_detectors(input_sample_rate: u32, config: FaxPaperConfig) -> Result<Vec<Detector>> {
    let mut detectors = Vec::with_capacity(2);
    match config.mode {
        FaxPaperMode::Manual { spec } => detectors.push(Detector {
            decoder: acquisition_decoder(input_sample_rate, config, spec.modulation, None)?,
        }),
        FaxPaperMode::Auto { fallback } => {
            detectors.push(Detector {
                decoder: acquisition_decoder(input_sample_rate, config, fallback.modulation, None)?,
            });
            if modulation_kind(fallback.modulation) != modulation_kind(config.auto_am_modulation) {
                detectors.push(Detector {
                    decoder: acquisition_decoder(
                        input_sample_rate,
                        config,
                        config.auto_am_modulation,
                        None,
                    )?,
                });
            }
        }
    }
    Ok(detectors)
}

fn acquisition_decoder(
    input_sample_rate: u32,
    paper: FaxPaperConfig,
    modulation: FaxModulation,
    locked: Option<FaxSpec>,
) -> Result<FaxDecoder> {
    FaxDecoder::new(
        input_sample_rate,
        FaxDecoderConfig {
            immediate_decode: false,
            clock_recovery: paper.clock_recovery,
            raster_basis: FaxRasterBasis::NominalPaper,
            ioc: locked.map(|spec| spec.ioc),
            lpm: locked.map(|spec| spec.lpm),
            modulation,
            max_lines: None,
            am_full_scale: paper.am_full_scale,
            expected_phasing_seconds: locked
                .map(|spec| spec.phasing_seconds)
                .or(paper.expected_phasing_seconds),
            apt_confirm_seconds: paper.apt_confirm_seconds,
            acquisition_timeout_seconds: paper.acquisition_timeout_seconds,
            stop_confirm_seconds: paper.stop_confirm_seconds,
            signal_loss_seconds: paper.signal_loss_seconds,
            minimum_signal_level: paper.minimum_signal_level,
            minimum_carrier_coherence: paper.minimum_carrier_coherence,
        },
    )
}

fn modulation_kind(modulation: FaxModulation) -> u8 {
    match modulation {
        FaxModulation::FmSubcarrier { .. } => 0,
        FaxModulation::AmSubcarrier { .. } => 1,
    }
}

fn same_decode_spec(left: FaxSpec, right: FaxSpec) -> bool {
    left.ioc == right.ioc
        && left.lpm == right.lpm
        && modulation_kind(left.modulation) == modulation_kind(right.modulation)
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn joint_dead_sector_search_recovers_midstream_phase_and_small_clock_error() {
        let spec = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::LPM_120);
        let width = spec.width() as usize;
        let sector = width - spec.active_width() as usize;
        let base_phase = 230.0_f64;
        let expected_ppm = 37.0_f64;
        let mut tracker = DeadSectorClockTracker::default();
        let mut latest = None;
        for line in 0..160_u64 {
            let mut row = vec![0_u8; width];
            for (column, value) in row.iter_mut().enumerate() {
                *value = 72 + ((column as u64 * 17 + line * 29) % 144) as u8;
            }
            let correction = base_phase + line as f64 * expected_ppm * width as f64 / 1_000_000.0;
            let observed = wrap_column(
                (f64::from(spec.active_width()) - correction).round() as i64,
                width,
            );
            for offset in 0..sector {
                row[(observed + offset) % width] = (line % 3) as u8;
            }
            latest = tracker.observe(line, spec, &row).or(latest);
        }
        let point = latest.expect("dead-sector calibration");
        let expected_phase =
            base_phase + point.reference_line as f64 * expected_ppm * width as f64 / 1_000_000.0;
        assert!(
            (f64::from(point.phase_pixels) - expected_phase).abs() <= 3.0,
            "point={point:?}, expected_phase={expected_phase}"
        );
        assert!(
            (f64::from(point.clock_ppm) - expected_ppm).abs() <= 6.0,
            "point={point:?}"
        );
    }
}
