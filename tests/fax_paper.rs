use rasterwave::fax::{
    FaxClockSource, FaxEncodeOptions, FaxEncoder, FaxIoc, FaxLpm, FaxModulation, FaxSpec,
};
use rasterwave::{
    FaxPaperConfig, FaxPaperDecoder, FaxPaperEvent, FaxPaperEventRef, FaxPaperMode, GrayImage,
    PaperBoundaryKind,
};

fn test_image(ioc: FaxIoc, lines: u32) -> GrayImage {
    let width = ioc.width();
    let mut pixels = Vec::with_capacity((width * lines) as usize);
    for y in 0..lines {
        for x in 0..width {
            pixels.push((((x + y * 17) * 255) / (width + lines * 17).max(1)) as u8);
        }
    }
    GrayImage::new(width, lines, pixels).unwrap()
}

fn encoded(mut spec: FaxSpec, lines: u32) -> Vec<f32> {
    spec.start_seconds = 1.0;
    spec.phasing_seconds = 3.0;
    spec.stop_seconds = 2.0;
    spec.trailing_black_seconds = 0.1;
    let mut encoder = FaxEncoder::new(
        test_image(spec.ioc, lines),
        spec,
        12_000,
        FaxEncodeOptions::default(),
    )
    .unwrap();
    let mut output = Vec::new();
    let mut chunk = vec![0.0; 4096];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        output.extend_from_slice(&chunk[..count]);
    }
    output
}

fn fast_config(mode: FaxPaperMode) -> FaxPaperConfig {
    FaxPaperConfig {
        mode,
        apt_confirm_seconds: 0.5,
        acquisition_timeout_seconds: 10.0,
        expected_phasing_seconds: Some(3.0),
        stop_confirm_seconds: 0.5,
        signal_loss_seconds: 3.0,
        minimum_signal_level: 0.0005,
        minimum_carrier_coherence: 0.0,
        ..FaxPaperConfig::default()
    }
}

fn feed(decoder: &mut FaxPaperDecoder, samples: &[f32], events: &mut Vec<FaxPaperEvent>) {
    for chunk in samples.chunks(1_200) {
        decoder.process_into(chunk, events).unwrap();
    }
}

#[test]
fn auto_prints_fallback_snow_without_apt() {
    let mut decoder = FaxPaperDecoder::new(12_000, FaxPaperConfig::default()).unwrap();
    let mut events = Vec::new();

    feed(&mut decoder, &vec![0.0; 12_000], &mut events);

    assert!(
        matches!(events.first(), Some(FaxPaperEvent::PaperStarted { spec, .. }) if spec.ioc == FaxIoc::Ioc576 && spec.lpm == FaxLpm::LPM_120)
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, FaxPaperEvent::LineReady { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, FaxPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn manual_fax_keeps_printing_past_1200_lines() {
    let spec = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::new(480).unwrap());
    let mut decoder = FaxPaperDecoder::new(
        12_000,
        FaxPaperConfig {
            mode: FaxPaperMode::Manual { spec },
            ..FaxPaperConfig::default()
        },
    )
    .unwrap();
    let mut events = Vec::new();
    let samples = vec![0.0; (spec.lpm.line_seconds() * 12_000.0 * 1_205.0) as usize];

    feed(&mut decoder, &samples, &mut events);

    assert!(events.iter().any(
        |event| matches!(event, FaxPaperEvent::LineReady { line_index, .. } if *line_index >= 1_200)
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, FaxPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn fm_self_decode_marks_apt_boundary_completes_and_continues() {
    let spec = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::LPM_240);
    let mut decoder =
        FaxPaperDecoder::new(12_000, fast_config(FaxPaperMode::Auto { fallback: spec })).unwrap();
    let mut events = Vec::new();
    feed(&mut decoder, &vec![0.0; 12_000], &mut events);
    feed(&mut decoder, &encoded(spec, 4), &mut events);
    let completion_position = events.len();
    feed(&mut decoder, &vec![0.0; 12_000], &mut events);

    let trusted = events
        .iter()
        .find_map(|event| match event {
            FaxPaperEvent::Boundary {
                boundary_id,
                line_index,
                kind: PaperBoundaryKind::AptPhasing,
                trusted: true,
                ..
            } => Some((*boundary_id, *line_index)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("APT/phasing boundary: {events:?}"));
    let completion = events
        .iter()
        .find_map(|event| match event {
            FaxPaperEvent::TransmissionCompleted {
                boundary_id,
                start_line,
                end_line,
                lines,
                ..
            } => Some((*boundary_id, *start_line, *end_line, *lines)),
            _ => None,
        })
        .unwrap_or_else(|| {
            let summary: Vec<_> = events
                .iter()
                .map(|event| match event {
                    FaxPaperEvent::PaperStarted { .. } => "paper",
                    FaxPaperEvent::Boundary { kind, .. } => match kind {
                        PaperBoundaryKind::Initial => "boundary-initial",
                        PaperBoundaryKind::AptPhasing => "boundary-apt",
                        PaperBoundaryKind::ProtocolEnd => "boundary-end",
                        PaperBoundaryKind::Discontinuity => "boundary-discontinuity",
                        _ => "boundary-other",
                    },
                    FaxPaperEvent::AptDetected { .. } => "apt",
                    FaxPaperEvent::LineReady { .. } => "line",
                    FaxPaperEvent::TransmissionCompleted { .. } => "completed",
                    FaxPaperEvent::ProtocolObserved { .. } => "observed",
                    FaxPaperEvent::SignalRejected { .. } => "rejected",
                    _ => "other",
                })
                .collect();
            panic!("APT stop completion: {summary:?}")
        });
    assert_eq!(completion.0, trusted.0);
    assert_eq!(completion.1, trusted.1);
    assert_eq!(completion.2 - completion.1, u64::from(completion.3));
    assert!(events[completion_position..].iter().any(|event| matches!(event, FaxPaperEvent::LineReady { line_index, .. } if *line_index >= completion.2)));
}

#[test]
fn auto_detects_am_candidate() {
    let mut spec = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::LPM_240);
    spec.modulation = FaxModulation::AmSubcarrier {
        carrier_hz: 1900.0,
        black_level: 0.0,
        white_level: 1.0,
    };
    let fallback = FaxSpec::standard(FaxIoc::Ioc576, FaxLpm::LPM_120);
    let mut decoder =
        FaxPaperDecoder::new(12_000, fast_config(FaxPaperMode::Auto { fallback })).unwrap();
    let mut events = Vec::new();

    feed(&mut decoder, &encoded(spec, 3), &mut events);

    assert!(events.iter().any(|event| matches!(event, FaxPaperEvent::Boundary { spec: detected, kind: PaperBoundaryKind::AptPhasing, trusted: true, .. } if matches!(detected.modulation, FaxModulation::AmSubcarrier { .. }))), "events: {events:?}");
}

#[test]
fn signal_lost_inserts_boundary_without_completing() {
    let mut decoder = FaxPaperDecoder::new(12_000, FaxPaperConfig::default()).unwrap();
    let mut events = Vec::new();
    feed(&mut decoder, &vec![0.0; 12_000], &mut events);

    decoder
        .mark_signal_lost(&mut |event: FaxPaperEventRef<'_>| events.push(event.to_owned()))
        .unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        FaxPaperEvent::Boundary {
            kind: PaperBoundaryKind::Discontinuity,
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, FaxPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn manual_lock_reports_mismatched_fax_header() {
    let locked = FaxSpec::standard(FaxIoc::Ioc576, FaxLpm::LPM_120);
    let observed = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::LPM_240);
    let mut decoder =
        FaxPaperDecoder::new(12_000, fast_config(FaxPaperMode::Manual { spec: locked })).unwrap();
    let mut events = Vec::new();

    feed(&mut decoder, &encoded(observed, 3), &mut events);

    assert!(events.iter().any(|event| matches!(event, FaxPaperEvent::ProtocolObserved { spec, trusted: true } if spec.ioc == observed.ioc && spec.lpm == observed.lpm)), "events: {events:?}");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, FaxPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn image_dead_sector_recovers_midstream_horizontal_phase() {
    let spec = FaxSpec::standard(FaxIoc::Ioc288, FaxLpm::LPM_240);
    let image = GrayImage::new(
        spec.active_width(),
        160,
        vec![128; spec.active_width() as usize * 160],
    )
    .unwrap();
    let mut encoder = FaxEncoder::new(
        image,
        spec,
        12_000,
        FaxEncodeOptions {
            include_apt: false,
            include_phasing: false,
            ..FaxEncodeOptions::default()
        },
    )
    .unwrap();
    let mut samples = vec![0.0_f32; 900];
    let mut chunk = [0.0_f32; 1_200];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        samples.extend_from_slice(&chunk[..count]);
    }
    let mut decoder = FaxPaperDecoder::new(
        12_000,
        FaxPaperConfig {
            mode: FaxPaperMode::Manual { spec },
            ..FaxPaperConfig::default()
        },
    )
    .unwrap();
    let mut events = Vec::new();
    feed(&mut decoder, &samples, &mut events);
    let calibration = events.iter().find_map(|event| match event {
        FaxPaperEvent::ClockCalibration { calibration, .. }
            if calibration.source == FaxClockSource::DeadSector =>
        {
            Some(calibration)
        }
        _ => None,
    });
    assert!(calibration.is_some(), "events: {events:?}");
    assert!(calibration.unwrap().phase_pixels.abs() > 20.0);
}
