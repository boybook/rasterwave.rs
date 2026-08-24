use rasterwave::{
    EncodeOptions, PaperBoundaryKind, Rgb, RgbImage, SstvEncoder, SstvMode, SstvPaperConfig,
    SstvPaperDecoder, SstvPaperEvent, SstvPaperMode,
};

fn encoded(mode: SstvMode) -> Vec<f32> {
    let spec = mode.spec();
    let mut pixels = Vec::with_capacity((spec.width * spec.height) as usize);
    for y in 0..spec.height {
        for x in 0..spec.width {
            pixels.push(Rgb::new(
                ((x * 255) / spec.width.max(1)) as u8,
                ((y * 255) / spec.height.max(1)) as u8,
                (((x + y) * 255) / (spec.width + spec.height).max(1)) as u8,
            ));
        }
    }
    let image = RgbImage::new(spec.width, spec.height, pixels).unwrap();
    let mut encoder = SstvEncoder::new(image, mode, 12_000, EncodeOptions::default()).unwrap();
    let mut output = Vec::new();
    let mut chunk = vec![0.0; 4096];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        output.extend_from_slice(&chunk[..count]);
    }
    output
}

fn feed(decoder: &mut SstvPaperDecoder, samples: &[f32], events: &mut Vec<SstvPaperEvent>) {
    for chunk in samples.chunks(1_200) {
        decoder.process_into(chunk, events).unwrap();
    }
}

#[test]
fn auto_prints_robot36_rows_before_any_protocol_header() {
    let mut decoder = SstvPaperDecoder::new(12_000, SstvPaperConfig::default()).unwrap();
    let mut events = Vec::new();
    let samples = vec![0.0; (SstvMode::Robot36.spec().line_seconds * 12_000.0 * 2.2) as usize];

    feed(&mut decoder, &samples, &mut events);

    assert!(matches!(
        events.first(),
        Some(SstvPaperEvent::PaperStarted {
            mode: SstvMode::Robot36,
            ..
        })
    ));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SstvPaperEvent::LineReady { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SstvPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn manual_mode_continues_beyond_two_nominal_images_without_completing() {
    let mode = SstvMode::Robot8Bw;
    let spec = mode.spec();
    let mut decoder = SstvPaperDecoder::new(
        12_000,
        SstvPaperConfig {
            mode: SstvPaperMode::Manual { mode },
            detect_vis: false,
            detect_sync_timing: false,
            minimum_signal_level: 0.0,
        },
    )
    .unwrap();
    let mut events = Vec::new();
    let seconds = spec.line_seconds * f64::from(spec.height) * 2.1;

    feed(
        &mut decoder,
        &vec![0.0; (seconds * 12_000.0) as usize],
        &mut events,
    );

    let max_line = events
        .iter()
        .filter_map(|event| match event {
            SstvPaperEvent::LineReady { line_index, .. } => Some(*line_index),
            _ => None,
        })
        .max()
        .unwrap();
    assert!(max_line >= u64::from(spec.height) * 2 - 2);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SstvPaperEvent::TransmissionCompleted { .. }))
    );
}

#[test]
fn auto_self_decode_marks_vis_capture_and_keeps_printing_after_completion() {
    let mode = SstvMode::Robot36;
    let mut decoder = SstvPaperDecoder::new(12_000, SstvPaperConfig::default()).unwrap();
    let mut events = Vec::new();
    let prefix = vec![0.0; (mode.spec().line_seconds * 12_000.0 * 2.0) as usize];
    feed(&mut decoder, &prefix, &mut events);
    feed(&mut decoder, &encoded(mode), &mut events);
    let completed_at = events.len();
    let suffix = vec![0.0; (mode.spec().line_seconds * 12_000.0 * 2.2) as usize];
    feed(&mut decoder, &suffix, &mut events);

    let trusted = events
        .iter()
        .find_map(|event| match event {
            SstvPaperEvent::Boundary {
                boundary_id,
                line_index,
                kind: PaperBoundaryKind::Vis,
                trusted: true,
                ..
            } => Some((*boundary_id, *line_index)),
            _ => None,
        })
        .expect("VIS boundary");
    let completion = events
        .iter()
        .find_map(|event| match event {
            SstvPaperEvent::TransmissionCompleted {
                boundary_id,
                start_line,
                end_line,
                lines,
                ..
            } => Some((*boundary_id, *start_line, *end_line, *lines)),
            _ => None,
        })
        .expect("trusted completion");
    assert_eq!(completion.0, trusted.0);
    assert_eq!(completion.1, trusted.1);
    assert_eq!(completion.2 - completion.1, u64::from(mode.spec().height));
    assert_eq!(completion.3, mode.spec().height);
    assert!(events[completed_at..].iter().any(|event| matches!(event, SstvPaperEvent::LineReady { line_index, .. } if *line_index >= completion.2)));
}

#[test]
fn manual_lock_reports_mismatched_vis_without_switching_or_completing() {
    let locked = SstvMode::Robot12Bw;
    let observed = SstvMode::Robot8Bw;
    let mut decoder = SstvPaperDecoder::new(
        12_000,
        SstvPaperConfig {
            mode: SstvPaperMode::Manual { mode: locked },
            ..SstvPaperConfig::default()
        },
    )
    .unwrap();
    let mut events = Vec::new();

    feed(&mut decoder, &encoded(observed), &mut events);

    assert!(events.iter().any(|event| matches!(event, SstvPaperEvent::ProtocolObserved { mode, trusted: true, .. } if *mode == observed)));
    assert!(
        events
            .iter()
            .filter_map(|event| match event {
                SstvPaperEvent::LineReady { mode, .. } => Some(*mode),
                _ => None,
            })
            .all(|mode| mode == locked)
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SstvPaperEvent::TransmissionCompleted { .. }))
    );
}
