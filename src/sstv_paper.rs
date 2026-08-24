use crate::{
    AbortReason, DecodeEvent, DecodeEventRef, DecoderConfig, DetectionSource, Error,
    LineCompleteness, PaperBoundaryKind, ProcessReport, Result, Rgb, SstvDecoder, SstvMode,
    SyncState,
};

/// How continuous SSTV paper chooses its active mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SstvPaperMode {
    /// Decode immediately with a fallback and switch at trusted protocol boundaries.
    Auto {
        /// Mode used before a trusted header or timing lock.
        fallback: SstvMode,
    },
    /// Keep decoding one caller-selected mode while observing protocol headers.
    Manual {
        /// Forced mode.
        mode: SstvMode,
    },
}

impl Default for SstvPaperMode {
    fn default() -> Self {
        Self::Auto {
            fallback: SstvMode::Robot36,
        }
    }
}

/// Continuous SSTV paper decoder policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SstvPaperConfig {
    /// Automatic fallback or manually locked mode.
    pub mode: SstvPaperMode,
    /// Detect standard calibration and VIS headers in parallel with paper output.
    pub detect_vis: bool,
    /// Detect an unambiguous mode from stable sync timing.
    pub detect_sync_timing: bool,
    /// Minimum signal level used only by the acquisition detector.
    pub minimum_signal_level: f32,
}

impl Default for SstvPaperConfig {
    fn default() -> Self {
        Self {
            mode: SstvPaperMode::default(),
            detect_vis: true,
            detect_sync_timing: true,
            minimum_signal_level: 0.002,
        }
    }
}

/// Borrowed event emitted by [`SstvPaperDecoder`].
#[derive(Debug)]
#[non_exhaustive]
pub enum SstvPaperEventRef<'a> {
    /// An indefinitely growing paper raster began.
    PaperStarted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Initial fallback or manually selected mode.
        mode: SstvMode,
        /// Initial pixel width.
        width: u32,
    },
    /// A meaningful divider in the paper.
    Boundary {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Monotonic boundary identifier.
        boundary_id: u64,
        /// First decoded row after the divider.
        line_index: u64,
        /// Mode active after the divider.
        mode: SstvMode,
        /// Detection path for protocol boundaries.
        detection: Option<DetectionSource>,
        /// Divider classification.
        kind: PaperBoundaryKind,
        /// Whether the divider anchors an automatic capture.
        trusted: bool,
        /// Pixel width after the divider.
        width: u32,
        /// Protocol-defined image height.
        nominal_height: u32,
    },
    /// Sync timing narrowed the input to one or more modes.
    ModeCandidate {
        /// Compatible modes, ordered by match score.
        candidates: &'a [SstvMode],
        /// Timing confidence.
        confidence: f32,
    },
    /// One continuous-paper RGB row is ready.
    LineReady {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary associated with the current segment.
        boundary_id: u64,
        /// Monotonic paper row.
        line_index: u64,
        /// Row inside the current SSTV image cycle.
        mode_line_index: u32,
        /// Active mode.
        mode: SstvMode,
        /// Monotonic revision for this paper row.
        revision: u32,
        /// Whether later chroma may revise this row.
        completeness: LineCompleteness,
        /// RGB pixels, valid until the callback returns.
        pixels: &'a [Rgb],
    },
    /// A trusted protocol capture completed while paper output continues.
    TransmissionCompleted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary that opened the capture.
        boundary_id: u64,
        /// First captured paper row.
        start_line: u64,
        /// Exclusive captured paper end.
        end_line: u64,
        /// Completed mode.
        mode: SstvMode,
        /// Number of completed image rows.
        lines: u32,
    },
    /// A trusted or ambiguous protocol was observed but not accepted.
    ProtocolObserved {
        /// Observed mode.
        mode: SstvMode,
        /// Detection path.
        detection: DetectionSource,
        /// Whether the observation was unambiguous and trusted.
        trusted: bool,
    },
    /// A header-like signal could not be accepted.
    SignalRejected {
        /// Stable machine-readable reason.
        reason: &'static str,
    },
}

/// Owned form of [`SstvPaperEventRef`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SstvPaperEvent {
    /// A paper raster began.
    PaperStarted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Initial mode.
        mode: SstvMode,
        /// Initial width.
        width: u32,
    },
    /// A meaningful paper divider.
    Boundary {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary identifier.
        boundary_id: u64,
        /// First row after the divider.
        line_index: u64,
        /// Active mode.
        mode: SstvMode,
        /// Detection path, when applicable.
        detection: Option<DetectionSource>,
        /// Divider classification.
        kind: PaperBoundaryKind,
        /// Whether the divider anchors an automatic capture.
        trusted: bool,
        /// Pixel width.
        width: u32,
        /// Protocol-defined image height.
        nominal_height: u32,
    },
    /// Mode candidates from sync timing.
    ModeCandidate {
        /// Compatible modes.
        candidates: Vec<SstvMode>,
        /// Timing confidence.
        confidence: f32,
    },
    /// One owned paper row.
    LineReady {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Current segment boundary.
        boundary_id: u64,
        /// Monotonic paper row.
        line_index: u64,
        /// Row within the mode cycle.
        mode_line_index: u32,
        /// Active mode.
        mode: SstvMode,
        /// Row revision.
        revision: u32,
        /// Completion state.
        completeness: LineCompleteness,
        /// Owned RGB pixels.
        pixels: Vec<Rgb>,
    },
    /// A trusted transmission completed.
    TransmissionCompleted {
        /// Decoder-local paper identifier.
        paper_id: u64,
        /// Boundary that opened the capture.
        boundary_id: u64,
        /// First captured row.
        start_line: u64,
        /// Exclusive captured end.
        end_line: u64,
        /// Completed mode.
        mode: SstvMode,
        /// Number of completed rows.
        lines: u32,
    },
    /// A protocol was observed but not accepted.
    ProtocolObserved {
        /// Observed mode.
        mode: SstvMode,
        /// Detection path.
        detection: DetectionSource,
        /// Whether the observation was trusted.
        trusted: bool,
    },
    /// A header-like signal was rejected.
    SignalRejected {
        /// Stable reason.
        reason: &'static str,
    },
}

impl SstvPaperEventRef<'_> {
    /// Copy a borrowed event into an owned value.
    pub fn to_owned(&self) -> SstvPaperEvent {
        match self {
            Self::PaperStarted {
                paper_id,
                mode,
                width,
            } => SstvPaperEvent::PaperStarted {
                paper_id: *paper_id,
                mode: *mode,
                width: *width,
            },
            Self::Boundary {
                paper_id,
                boundary_id,
                line_index,
                mode,
                detection,
                kind,
                trusted,
                width,
                nominal_height,
            } => SstvPaperEvent::Boundary {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                line_index: *line_index,
                mode: *mode,
                detection: *detection,
                kind: *kind,
                trusted: *trusted,
                width: *width,
                nominal_height: *nominal_height,
            },
            Self::ModeCandidate {
                candidates,
                confidence,
            } => SstvPaperEvent::ModeCandidate {
                candidates: candidates.to_vec(),
                confidence: *confidence,
            },
            Self::LineReady {
                paper_id,
                boundary_id,
                line_index,
                mode_line_index,
                mode,
                revision,
                completeness,
                pixels,
            } => SstvPaperEvent::LineReady {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                line_index: *line_index,
                mode_line_index: *mode_line_index,
                mode: *mode,
                revision: *revision,
                completeness: *completeness,
                pixels: pixels.to_vec(),
            },
            Self::TransmissionCompleted {
                paper_id,
                boundary_id,
                start_line,
                end_line,
                mode,
                lines,
            } => SstvPaperEvent::TransmissionCompleted {
                paper_id: *paper_id,
                boundary_id: *boundary_id,
                start_line: *start_line,
                end_line: *end_line,
                mode: *mode,
                lines: *lines,
            },
            Self::ProtocolObserved {
                mode,
                detection,
                trusted,
            } => SstvPaperEvent::ProtocolObserved {
                mode: *mode,
                detection: *detection,
                trusted: *trusted,
            },
            Self::SignalRejected { reason } => SstvPaperEvent::SignalRejected { reason },
        }
    }
}

/// Synchronous consumer for borrowed continuous SSTV events.
pub trait SstvPaperSink {
    /// Handle one event before borrowed pixels are reused.
    fn on_event(&mut self, event: SstvPaperEventRef<'_>);
}

impl<F> SstvPaperSink for F
where
    F: for<'a> FnMut(SstvPaperEventRef<'a>),
{
    fn on_event(&mut self, event: SstvPaperEventRef<'_>) {
        self(event);
    }
}

#[derive(Debug)]
struct Capture {
    boundary_id: u64,
    start_line: u64,
    image_id: u64,
}

/// Continuous SSTV raster that decodes fallback noise while acquiring headers.
#[derive(Debug)]
pub struct SstvPaperDecoder {
    input_sample_rate: u32,
    config: SstvPaperConfig,
    detector: SstvDecoder,
    raster: SstvDecoder,
    paper_id: u64,
    next_boundary_id: u64,
    active_boundary_id: u64,
    paper_line: u64,
    current_mode: SstvMode,
    raster_image_id: Option<u64>,
    raster_frame_start: u64,
    capture: Option<Capture>,
    started: bool,
    finished: bool,
}

impl SstvPaperDecoder {
    /// Construct a continuous SSTV paper decoder.
    pub fn new(input_sample_rate: u32, config: SstvPaperConfig) -> Result<Self> {
        if !config.minimum_signal_level.is_finite() || config.minimum_signal_level < 0.0 {
            return Err(Error::InvalidConfiguration(
                "minimum_signal_level must be finite and non-negative",
            ));
        }
        let initial_mode = match config.mode {
            SstvPaperMode::Auto { fallback } => fallback,
            SstvPaperMode::Manual { mode } => mode,
        };
        let detector = SstvDecoder::new(
            input_sample_rate,
            DecoderConfig {
                detect_vis: config.detect_vis,
                detect_sync_timing: config.detect_sync_timing,
                manual_mode: None,
                minimum_signal_level: config.minimum_signal_level,
                ..DecoderConfig::default()
            },
        )?;
        Ok(Self {
            input_sample_rate,
            config,
            detector,
            raster: immediate_decoder(input_sample_rate, initial_mode)?,
            paper_id: 1,
            next_boundary_id: 2,
            active_boundary_id: 1,
            paper_line: 0,
            current_mode: initial_mode,
            raster_image_id: None,
            raster_frame_start: 0,
            capture: None,
            started: false,
            finished: false,
        })
    }

    /// Push arbitrary-sized mono PCM and emit rows before the call returns.
    pub fn push_f32(
        &mut self,
        input: &[f32],
        sink: &mut impl SstvPaperSink,
    ) -> Result<ProcessReport> {
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
        if !self.started {
            self.started = true;
            let spec = self.current_mode.spec();
            sink.on_event(SstvPaperEventRef::PaperStarted {
                paper_id: self.paper_id,
                mode: self.current_mode,
                width: spec.width,
            });
            sink.on_event(SstvPaperEventRef::Boundary {
                paper_id: self.paper_id,
                boundary_id: self.active_boundary_id,
                line_index: 0,
                mode: self.current_mode,
                detection: match self.config.mode {
                    SstvPaperMode::Manual { .. } => Some(DetectionSource::Manual),
                    SstvPaperMode::Auto { .. } => None,
                },
                kind: PaperBoundaryKind::Initial,
                trusted: false,
                width: spec.width,
                nominal_height: spec.height,
            });
            report.events_emitted += 2;
        }

        for chunk in input.chunks(64) {
            let capture_was_active = self.capture.is_some();
            let mut detector_events = Vec::new();
            let detector_report = self.detector.process_into(chunk, &mut detector_events)?;
            report.working_samples += detector_report.working_samples;
            let mut detector_claimed_chunk = capture_was_active;
            for event in detector_events {
                report.events_emitted +=
                    self.handle_detector_event(event, &mut detector_claimed_chunk, sink)?;
            }
            if !detector_claimed_chunk && self.capture.is_none() {
                let mut raster_events = Vec::new();
                self.raster.process_into(chunk, &mut raster_events)?;
                for event in raster_events {
                    report.events_emitted += self.handle_raster_event(event, sink);
                }
            }
        }
        Ok(report)
    }

    /// Collect owned events for one PCM chunk.
    pub fn process_into(
        &mut self,
        input: &[f32],
        output: &mut Vec<SstvPaperEvent>,
    ) -> Result<ProcessReport> {
        let mut sink = |event: SstvPaperEventRef<'_>| output.push(event.to_owned());
        self.push_f32(input, &mut sink)
    }

    /// Finalize input without completing an untrusted paper tail.
    pub fn finish(&mut self, sink: &mut impl SstvPaperSink) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut detector_events = Vec::new();
        let mut report = self.detector.finish_into(&mut detector_events)?;
        let mut claimed = self.capture.is_some();
        for event in detector_events {
            report.events_emitted += self.handle_detector_event(event, &mut claimed, sink)?;
        }
        let mut ignored = Vec::new();
        let _ = self.raster.finish_into(&mut ignored);
        self.finished = true;
        Ok(report)
    }

    /// Collect owned finalization events.
    pub fn finish_into(&mut self, output: &mut Vec<SstvPaperEvent>) -> Result<ProcessReport> {
        let mut sink = |event: SstvPaperEventRef<'_>| output.push(event.to_owned());
        self.finish(&mut sink)
    }

    /// Mark dropped input, close any capture without completing it and continue paper output.
    pub fn mark_discontinuity(
        &mut self,
        dropped_input_samples: u64,
        sink: &mut impl SstvPaperSink,
    ) -> Result<ProcessReport> {
        if self.finished {
            return Err(Error::DecoderFinished);
        }
        let mut ignored = Vec::new();
        self.detector
            .mark_discontinuity(dropped_input_samples, &mut |event: DecodeEventRef<'_>| {
                ignored.push(event.to_owned());
            })?;
        self.raster = immediate_decoder(self.input_sample_rate, self.current_mode)?;
        self.capture = None;
        self.raster_image_id = None;
        self.raster_frame_start = self.paper_line;
        self.emit_boundary(PaperBoundaryKind::Discontinuity, None, false, sink);
        Ok(ProcessReport {
            events_emitted: 1,
            ..ProcessReport::default()
        })
    }

    /// Reset acquisition and continue the same paper with an explicit divider.
    pub fn reset(&mut self, sink: &mut impl SstvPaperSink) -> Result<ProcessReport> {
        if self.finished {
            self.finished = false;
        }
        let mut ignored = Vec::new();
        self.detector
            .reset_with_sink(&mut |event: DecodeEventRef<'_>| {
                ignored.push(event.to_owned());
            });
        self.raster = immediate_decoder(self.input_sample_rate, self.current_mode)?;
        self.capture = None;
        self.raster_image_id = None;
        self.raster_frame_start = self.paper_line;
        self.emit_boundary(PaperBoundaryKind::Reset, None, false, sink);
        Ok(ProcessReport {
            events_emitted: 1,
            ..ProcessReport::default()
        })
    }

    /// Current acquisition state. Paper output itself remains active while searching.
    pub fn sync_state(&self) -> SyncState {
        if self.finished {
            SyncState::Finished
        } else {
            self.detector.sync_state()
        }
    }

    /// Monotonic row after the latest emitted paper row.
    pub const fn next_line_index(&self) -> u64 {
        self.paper_line
    }

    fn handle_detector_event(
        &mut self,
        event: DecodeEvent,
        claimed_chunk: &mut bool,
        sink: &mut impl SstvPaperSink,
    ) -> Result<usize> {
        match event {
            DecodeEvent::ModeCandidate {
                candidates,
                confidence,
            } => {
                sink.on_event(SstvPaperEventRef::ModeCandidate {
                    candidates: &candidates,
                    confidence,
                });
                Ok(1)
            }
            DecodeEvent::ImageStarted {
                image_id,
                mode,
                detection,
                ..
            } => {
                let trusted = trusted_detection(detection);
                let accepted = trusted
                    && match self.config.mode {
                        SstvPaperMode::Auto { .. } => true,
                        SstvPaperMode::Manual { mode: locked } => locked == mode,
                    };
                if !accepted {
                    sink.on_event(SstvPaperEventRef::ProtocolObserved {
                        mode,
                        detection,
                        trusted,
                    });
                    return Ok(1);
                }
                *claimed_chunk = true;
                let boundary_id = self.next_boundary_id;
                self.next_boundary_id += 1;
                self.active_boundary_id = boundary_id;
                self.current_mode = mode;
                self.capture = Some(Capture {
                    boundary_id,
                    start_line: self.paper_line,
                    image_id,
                });
                let spec = mode.spec();
                sink.on_event(SstvPaperEventRef::Boundary {
                    paper_id: self.paper_id,
                    boundary_id,
                    line_index: self.paper_line,
                    mode,
                    detection: Some(detection),
                    kind: detection_boundary(detection),
                    trusted: true,
                    width: spec.width,
                    nominal_height: spec.height,
                });
                Ok(1)
            }
            DecodeEvent::LineReady {
                image_id,
                mode,
                line_index,
                revision,
                completeness,
                pixels,
            } => {
                let Some(capture) = self
                    .capture
                    .as_ref()
                    .filter(|value| value.image_id == image_id)
                else {
                    return Ok(0);
                };
                *claimed_chunk = true;
                let paper_line = capture.start_line + u64::from(line_index);
                self.paper_line = self.paper_line.max(paper_line + 1);
                sink.on_event(SstvPaperEventRef::LineReady {
                    paper_id: self.paper_id,
                    boundary_id: capture.boundary_id,
                    line_index: paper_line,
                    mode_line_index: line_index,
                    mode,
                    revision,
                    completeness,
                    pixels: &pixels,
                });
                Ok(1)
            }
            DecodeEvent::ImageCompleted {
                image_id,
                mode,
                lines,
            } => {
                let Some(capture) = self
                    .capture
                    .take()
                    .filter(|value| value.image_id == image_id)
                else {
                    return Ok(0);
                };
                *claimed_chunk = true;
                let end_line = capture.start_line + u64::from(lines);
                self.paper_line = self.paper_line.max(end_line);
                sink.on_event(SstvPaperEventRef::TransmissionCompleted {
                    paper_id: self.paper_id,
                    boundary_id: capture.boundary_id,
                    start_line: capture.start_line,
                    end_line,
                    mode,
                    lines,
                });
                self.raster = immediate_decoder(self.input_sample_rate, mode)?;
                self.raster_image_id = None;
                self.raster_frame_start = end_line;
                self.emit_boundary(PaperBoundaryKind::ProtocolEnd, None, false, sink);
                Ok(2)
            }
            DecodeEvent::ImageAborted {
                image_id, reason, ..
            } => {
                if self
                    .capture
                    .as_ref()
                    .is_none_or(|value| value.image_id != image_id)
                {
                    return Ok(0);
                }
                *claimed_chunk = true;
                self.capture = None;
                self.raster = immediate_decoder(self.input_sample_rate, self.current_mode)?;
                self.raster_image_id = None;
                self.raster_frame_start = self.paper_line;
                let kind = match reason {
                    AbortReason::Reset => PaperBoundaryKind::Reset,
                    _ => PaperBoundaryKind::Discontinuity,
                };
                self.emit_boundary(kind, None, false, sink);
                Ok(1)
            }
            DecodeEvent::SignalRejected { reason } => {
                sink.on_event(SstvPaperEventRef::SignalRejected { reason });
                Ok(1)
            }
        }
    }

    fn handle_raster_event(&mut self, event: DecodeEvent, sink: &mut impl SstvPaperSink) -> usize {
        match event {
            DecodeEvent::ImageStarted { image_id, mode, .. } => {
                self.raster_image_id = Some(image_id);
                self.raster_frame_start = self.paper_line;
                self.current_mode = mode;
                0
            }
            DecodeEvent::LineReady {
                image_id,
                mode,
                line_index,
                revision,
                completeness,
                pixels,
            } if self.raster_image_id == Some(image_id) => {
                let paper_line = self.raster_frame_start + u64::from(line_index);
                self.paper_line = self.paper_line.max(paper_line + 1);
                sink.on_event(SstvPaperEventRef::LineReady {
                    paper_id: self.paper_id,
                    boundary_id: self.active_boundary_id,
                    line_index: paper_line,
                    mode_line_index: line_index,
                    mode,
                    revision,
                    completeness,
                    pixels: &pixels,
                });
                1
            }
            DecodeEvent::ImageCompleted {
                image_id, lines, ..
            } if self.raster_image_id == Some(image_id) => {
                self.paper_line = self
                    .paper_line
                    .max(self.raster_frame_start + u64::from(lines));
                self.raster_image_id = None;
                0
            }
            DecodeEvent::ImageAborted { image_id, .. }
                if self.raster_image_id == Some(image_id) =>
            {
                self.raster_image_id = None;
                0
            }
            _ => 0,
        }
    }

    fn emit_boundary(
        &mut self,
        kind: PaperBoundaryKind,
        detection: Option<DetectionSource>,
        trusted: bool,
        sink: &mut impl SstvPaperSink,
    ) {
        let boundary_id = self.next_boundary_id;
        self.next_boundary_id += 1;
        self.active_boundary_id = boundary_id;
        let spec = self.current_mode.spec();
        sink.on_event(SstvPaperEventRef::Boundary {
            paper_id: self.paper_id,
            boundary_id,
            line_index: self.paper_line,
            mode: self.current_mode,
            detection,
            kind,
            trusted,
            width: spec.width,
            nominal_height: spec.height,
        });
    }
}

fn immediate_decoder(input_sample_rate: u32, mode: SstvMode) -> Result<SstvDecoder> {
    SstvDecoder::new(
        input_sample_rate,
        DecoderConfig {
            immediate_decode: true,
            detect_vis: false,
            detect_sync_timing: true,
            manual_mode: Some(mode),
            minimum_signal_level: 0.0,
        },
    )
}

fn trusted_detection(detection: DetectionSource) -> bool {
    match detection {
        DetectionSource::Vis { .. } | DetectionSource::Manual => true,
        DetectionSource::SyncTiming {
            ambiguous,
            candidate_count,
        } => !ambiguous && candidate_count == 1,
    }
}

fn detection_boundary(detection: DetectionSource) -> PaperBoundaryKind {
    match detection {
        DetectionSource::Vis { .. } => PaperBoundaryKind::Vis,
        DetectionSource::SyncTiming { .. } | DetectionSource::Manual => {
            PaperBoundaryKind::SyncTiming
        }
    }
}
