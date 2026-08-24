use rasterwave::{
    AbortReason, DecodeEvent, DecodeEventRef, DecoderConfig, DetectionSource, EncodeOptions,
    LineCompleteness, Rgb, RgbImage, SSTV_MODES, SstvDecoder, SstvEncoder, SstvMode,
};

fn test_image(mode: SstvMode) -> RgbImage {
    let spec = mode.spec();
    let mut pixels = Vec::with_capacity(spec.width as usize * spec.height as usize);
    for y in 0..spec.height {
        for x in 0..spec.width {
            pixels.push(Rgb::new(
                (x * 255 / spec.width.max(1)) as u8,
                (y * 255 / spec.height.max(1)) as u8,
                ((x + y) * 255 / (spec.width + spec.height).max(1)) as u8,
            ));
        }
    }
    RgbImage::new(spec.width, spec.height, pixels).unwrap()
}

fn encode(mode: SstvMode, tone_offset_hz: f32) -> Vec<f32> {
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            tone_offset_hz,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let mut output = Vec::new();
    let mut chunk = [0.0_f32; 127];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        output.extend_from_slice(&chunk[..count]);
    }
    output
}

#[test]
fn decoder_emits_lines_before_end_of_input() {
    let mode = SstvMode::Robot8Bw;
    let pcm = encode(mode, 0.0);
    let halfway = pcm.len() / 2;
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();

    for chunk in pcm[..halfway].chunks(113) {
        decoder.process_into(chunk, &mut events).unwrap();
    }

    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageStarted { mode: detected, .. } if *detected == mode
    )));
    assert!(
        events.iter().any(|event| matches!(
            event,
            DecodeEvent::LineReady {
                completeness: LineCompleteness::Final,
                ..
            }
        )),
        "at least one line must be visible before EOF"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ImageCompleted { .. }))
    );

    for chunk in pcm[halfway..].chunks(113) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();

    let final_rows = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            )
        })
        .count();
    assert_eq!(final_rows, mode.spec().height as usize);
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageCompleted { mode: completed, .. } if *completed == mode
    )));
}

#[test]
fn vis_detection_tracks_frequency_offset() {
    let mode = SstvMode::Robot8Bw;
    for expected_offset in [-250.0, -75.0, 75.0, 250.0] {
        let pcm = encode(mode, expected_offset);
        let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
        let mut events = Vec::new();
        for chunk in pcm.chunks(257) {
            decoder.process_into(chunk, &mut events).unwrap();
        }
        let offset = events
            .iter()
            .find_map(|event| match event {
                DecodeEvent::ImageStarted {
                    mode: detected,
                    frequency_offset_hz,
                    ..
                } if *detected == mode => Some(*frequency_offset_hz),
                _ => None,
            })
            .expect("VIS should start an image");
        assert!(
            (offset - expected_offset).abs() < 30.0,
            "expected {expected_offset} Hz, estimated {offset} Hz"
        );
    }
}

#[test]
fn sync_timing_detects_mode_without_vis() {
    let mode = SstvMode::Robot8Bw;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 173];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ModeCandidate { candidates, .. } if candidates == &[mode]
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageStarted {
            mode: detected,
            detection: rasterwave::DetectionSource::SyncTiming { ambiguous: false, .. },
            ..
        } if *detected == mode
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::LineReady { mode: detected, .. } if *detected == mode
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            ))
            .count(),
        mode.spec().height as usize
    );
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageCompleted { mode: completed, .. } if *completed == mode
    )));
}

#[test]
fn ambiguous_sync_timing_never_invents_exact_mode() {
    let mode = SstvMode::Martin1;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 251];
    let sample_limit = (mode.spec().line_seconds * 12_000.0 * 10.0) as usize;
    let mut emitted = 0;
    while emitted < sample_limit {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
        emitted += count;
    }

    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ModeCandidate { candidates, .. }
            if candidates.contains(&SstvMode::Martin1)
                && candidates.contains(&SstvMode::Martin3)
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ImageStarted { .. }))
    );
}

#[test]
fn robot36_revises_provisional_chroma_rows() {
    let mode = SstvMode::Robot36;
    let pcm = encode(mode, 0.0);
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    for chunk in pcm.chunks(997) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();

    let row_zero: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            DecodeEvent::LineReady {
                line_index: 0,
                revision,
                completeness,
                ..
            } => Some((*revision, *completeness)),
            _ => None,
        })
        .collect();
    assert_eq!(
        row_zero,
        vec![
            (0, LineCompleteness::Provisional),
            (1, LineCompleteness::Final)
        ]
    );
    let final_rows = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            )
        })
        .count();
    assert_eq!(final_rows, mode.spec().height as usize);
}

#[test]
fn truncated_signal_followed_by_silence_aborts_without_false_completion() {
    let mode = SstvMode::Robot8Bw;
    let pcm = encode(mode, 0.0);
    let line_samples = (mode.spec().line_seconds * 12_000.0).round() as usize;
    let cutoff = (12_000_f64 * 0.91).round() as usize + line_samples * 10;
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();

    for chunk in pcm[..cutoff.min(pcm.len())].chunks(211) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    for chunk in vec![0.0_f32; (line_samples * 4).max(9_000)].chunks(211) {
        decoder.process_into(chunk, &mut events).unwrap();
    }

    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageAborted {
            reason: AbortReason::SyncLost,
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ImageCompleted { .. }))
    );
    let rows = events
        .iter()
        .filter(|event| matches!(event, DecodeEvent::LineReady { .. }))
        .count();
    assert!(rows < mode.spec().height as usize);
}

#[test]
fn decoder_recovers_after_several_missing_sync_pulses_with_carrier_present() {
    let mode = SstvMode::Robot8Bw;
    let mut pcm = encode(mode, 0.0);
    let body_start = (0.91_f64 * 12_000.0).round() as usize;
    let sync_samples = (mode.spec().sync_seconds * 12_000.0).round() as usize;
    for line in 10..15 {
        let start =
            body_start + (line as f64 * mode.spec().line_seconds * 12_000.0).round() as usize;
        for (offset, sample) in pcm[start..start + sync_samples].iter_mut().enumerate() {
            let phase = std::f64::consts::TAU * 1_900.0 * (start + offset) as f64 / 12_000.0;
            *sample = (phase.sin() * 0.5) as f32;
        }
    }
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    for chunk in pcm.chunks(227) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();
    assert!(!events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageAborted {
            reason: AbortReason::SyncLost,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageCompleted { mode: completed, .. } if *completed == mode
    )));
}

#[test]
fn end_of_input_never_pads_a_partial_line() {
    let mode = SstvMode::Robot8Bw;
    let pcm = encode(mode, 0.0);
    let header_samples = (12_000_f64 * 0.91).round() as usize;
    let line_samples = (mode.spec().line_seconds * 12_000.0).round() as usize;
    let cutoff = header_samples + line_samples * 3 + line_samples * 3 / 4;
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    for chunk in pcm[..cutoff.min(pcm.len())].chunks(197) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();

    let final_rows = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            )
        })
        .count();
    assert_eq!(final_rows, 3);
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageAborted {
            reason: AbortReason::EndOfInput,
            last_line: Some(2),
            ..
        }
    )));
}

#[test]
fn pd_abort_reports_image_row_not_radio_line() {
    let mode = SstvMode::Pd50;
    let pcm = encode(mode, 0.0);
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    let mut consumed = 0;
    while consumed < pcm.len() {
        let end = (consumed + 313).min(pcm.len());
        decoder
            .process_into(&pcm[consumed..end], &mut events)
            .unwrap();
        consumed = end;
        let final_rows = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    DecodeEvent::LineReady {
                        completeness: LineCompleteness::Final,
                        ..
                    }
                )
            })
            .count();
        if final_rows >= 6 {
            break;
        }
    }
    let mut sink = |event: DecodeEventRef<'_>| events.push(event.to_owned());
    decoder.mark_discontinuity(1, &mut sink).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageAborted {
            last_line: Some(5),
            reason: AbortReason::InputDiscontinuity,
            ..
        }
    )));
}

#[test]
fn disabled_sync_timing_emits_no_timing_candidates() {
    let mode = SstvMode::Robot8Bw;
    let mut pcm = encode(mode, 0.0);
    pcm.drain(..(12_000_f64 * 0.91).round() as usize);
    let config = DecoderConfig {
        detect_vis: false,
        detect_sync_timing: false,
        ..DecoderConfig::default()
    };
    let mut decoder = SstvDecoder::new(12_000, config).unwrap();
    let mut events = Vec::new();
    for chunk in pcm.chunks(223) {
        decoder.process_into(chunk, &mut events).unwrap();
    }
    assert!(!events.iter().any(|event| matches!(
        event,
        DecodeEvent::ModeCandidate { .. } | DecodeEvent::ImageStarted { .. }
    )));
}

#[test]
fn manual_mode_overrides_compatible_vis_mode() {
    let pcm = encode(SstvMode::Martin1, 0.0);
    let config = DecoderConfig {
        manual_mode: Some(SstvMode::Martin3),
        ..DecoderConfig::default()
    };
    let mut decoder = SstvDecoder::new(12_000, config).unwrap();
    let mut events = Vec::new();
    for chunk in pcm.chunks(269) {
        decoder.process_into(chunk, &mut events).unwrap();
        if events.iter().any(|event| {
            matches!(
                event,
                DecodeEvent::ImageStarted {
                    mode: SstvMode::Martin3,
                    detection: DetectionSource::Manual,
                    ..
                }
            )
        }) {
            break;
        }
    }
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageStarted {
            mode: SstvMode::Martin3,
            detection: DetectionSource::Manual,
            ..
        }
    )));
}

#[test]
fn manual_mode_does_not_lock_an_incompatible_sync_train() {
    let mode = SstvMode::Robot8Bw;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let config = DecoderConfig {
        detect_vis: false,
        manual_mode: Some(SstvMode::Martin1),
        ..DecoderConfig::default()
    };
    let mut decoder = SstvDecoder::new(12_000, config).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 337];
    let sample_limit = (mode.spec().line_seconds * 12_000.0 * 10.0) as usize;
    let mut emitted = 0;
    while emitted < sample_limit {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
        emitted += count;
    }
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ImageStarted { .. }))
    );
}

#[test]
fn scottie_without_vis_reports_candidates_but_does_not_auto_lock() {
    let mode = SstvMode::Scottie1;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 281];
    let sample_limit = (mode.spec().line_seconds * 12_000.0 * 10.0) as usize;
    let mut emitted = 0;
    while emitted < sample_limit {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
        emitted += count;
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ModeCandidate { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DecodeEvent::ImageStarted { .. }))
    );
}

#[test]
fn manual_scottie_without_vis_replays_acquisition_history() {
    let mode = SstvMode::Scottie1;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let config = DecoderConfig {
        detect_vis: false,
        manual_mode: Some(mode),
        ..DecoderConfig::default()
    };
    let mut decoder = SstvDecoder::new(12_000, config).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 401];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            ))
            .count(),
        mode.spec().height as usize
    );
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageCompleted { mode: completed, .. } if *completed == mode
    )));
}

#[test]
fn unique_scottie_dx_timing_auto_locks_without_vis() {
    let mode = SstvMode::ScottieDx;
    let mut encoder = SstvEncoder::new(
        test_image(mode),
        mode,
        12_000,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
    )
    .unwrap();
    let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
    let mut events = Vec::new();
    let mut chunk = [0.0_f32; 509];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        decoder.process_into(&chunk[..count], &mut events).unwrap();
    }
    decoder.finish_into(&mut events).unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageStarted {
            mode: SstvMode::ScottieDx,
            detection: DetectionSource::SyncTiming {
                ambiguous: false,
                ..
            },
            ..
        }
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                DecodeEvent::LineReady {
                    completeness: LineCompleteness::Final,
                    ..
                }
            ))
            .count(),
        mode.spec().height as usize
    );
    assert!(events.iter().any(|event| matches!(
        event,
        DecodeEvent::ImageCompleted { mode: completed, .. } if *completed == mode
    )));
}

#[test]
fn every_registered_vis_mode_completes_its_declared_raster() {
    for spec in SSTV_MODES {
        let mode = spec.mode;
        let mut encoder =
            SstvEncoder::new(test_image(mode), mode, 12_000, EncodeOptions::default()).unwrap();
        let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
        let mut events = Vec::new();
        let mut chunk = [0.0_f32; 8_192];
        while !encoder.is_finished() {
            let count = encoder.read_samples(&mut chunk);
            decoder.process_into(&chunk[..count], &mut events).unwrap();
        }
        decoder.finish_into(&mut events).unwrap();
        let final_rows = events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    DecodeEvent::LineReady {
                        completeness: LineCompleteness::Final,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(final_rows, spec.height as usize, "mode {mode:?}");
        assert!(
            events.iter().any(|event| matches!(
                event,
                DecodeEvent::ImageCompleted { mode: completed, lines, .. }
                    if *completed == mode && *lines == spec.height
            )),
            "mode {mode:?} did not complete"
        );
    }
}

#[test]
fn immediate_decode_starts_and_emits_a_row_for_every_mode_without_sync() {
    for spec in SSTV_MODES {
        let mode = spec.mode;
        let mut decoder = SstvDecoder::new(
            12_000,
            DecoderConfig {
                immediate_decode: true,
                detect_vis: false,
                detect_sync_timing: false,
                manual_mode: Some(mode),
                minimum_signal_level: 1.0,
            },
        )
        .unwrap();
        let mut events = Vec::new();
        let samples = vec![0.0; (spec.line_seconds * 12_000.0).ceil() as usize + 2];

        decoder.process_into(&samples, &mut events).unwrap();

        assert!(
            matches!(
                events.first(),
                Some(DecodeEvent::ImageStarted {
                    mode: started,
                    detection: DetectionSource::Manual,
                    ..
                }) if *started == mode
            ),
            "mode {mode:?} did not start immediately: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                DecodeEvent::LineReady { mode: decoded, .. } if *decoded == mode
            )),
            "mode {mode:?} did not emit a row without sync"
        );
    }
}

#[test]
fn immediate_decode_requires_a_manual_mode() {
    assert!(
        SstvDecoder::new(
            12_000,
            DecoderConfig {
                immediate_decode: true,
                ..DecoderConfig::default()
            }
        )
        .is_err()
    );
}
