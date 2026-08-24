use crate::fax::{
    FaxDecodeEvent, FaxDecodeEventRef, FaxDecoder, FaxDecoderConfig, FaxIoc, FaxLpm, FaxModulation,
    FaxSpec,
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
            Self::LineReady {
                paper_id,
                boundary_id,
                line_index,
                segment_line_index,
                spec,
                pixels,
            } => FaxPaperEvent::LineReady {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                line_index: *line_index,
                segment_line_index: *segment_line_index,
                spec: *spec,
                pixels: pixels.to_vec(),
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
            FaxDecodeEvent::PageStarted { page_id, spec } => {
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
                Ok(1)
            }
            FaxDecodeEvent::LineReady {
                page_id,
                line_index,
                pixels,
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
                });
                Ok(1)
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
            FaxDecodeEvent::PageStarted { page_id, spec } => {
                self.raster_page_id = Some(page_id);
                self.raster_segment_start = self.paper_line;
                self.current_spec = spec;
                0
            }
            FaxDecodeEvent::LineReady {
                page_id,
                line_index,
                pixels,
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
                });
                1
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
