use rasterwave::{
    DecodeEvent, DecoderConfig, EncodeOptions, EncoderStage, Rgb, RgbImage, SstvDecoder,
    SstvEncoder, SstvMode, SstvStationId, SstvTransmissionEnvelope,
};

const SAMPLE_RATE: u32 = 12_000;

fn image(mode: SstvMode) -> RgbImage {
    let spec = mode.spec();
    RgbImage::filled(spec.width, spec.height, Rgb::new(72, 128, 196))
}

fn collect(mut encoder: SstvEncoder, chunk_size: usize) -> (Vec<f32>, u64, u64) {
    let initial = encoder.progress();
    let mut samples = Vec::new();
    let mut chunk = vec![0.0; chunk_size];
    while !encoder.is_finished() {
        let written = encoder.read_samples(&mut chunk);
        samples.extend_from_slice(&chunk[..written]);
    }
    assert_eq!(encoder.progress().stage, EncoderStage::Finished);
    (
        samples,
        initial.raster_start_sample,
        initial.raster_end_sample,
    )
}

fn power_at(samples: &[f32], frequency_hz: f64) -> f64 {
    let step = std::f64::consts::TAU * frequency_hz / f64::from(SAMPLE_RATE);
    let (mut real, mut imaginary) = (0.0, 0.0);
    for (index, sample) in samples.iter().enumerate() {
        let phase = step * index as f64;
        real += f64::from(*sample) * phase.cos();
        imaginary -= f64::from(*sample) * phase.sin();
    }
    real * real + imaginary * imaginary
}

fn dominant_frequency(samples: &[f32], candidates: &[f64]) -> f64 {
    candidates
        .iter()
        .copied()
        .max_by(|left, right| {
            power_at(samples, *left)
                .partial_cmp(&power_at(samples, *right))
                .unwrap()
        })
        .unwrap()
}

#[test]
fn empty_envelope_is_sample_identical_to_legacy_constructor() {
    let legacy = SstvEncoder::new(
        image(SstvMode::Robot8Bw),
        SstvMode::Robot8Bw,
        SAMPLE_RATE,
        EncodeOptions::default(),
    )
    .unwrap();
    let explicit = SstvEncoder::new_with_envelope(
        image(SstvMode::Robot8Bw),
        SstvMode::Robot8Bw,
        SAMPLE_RATE,
        EncodeOptions::default(),
        SstvTransmissionEnvelope::default(),
    )
    .unwrap();
    assert_eq!(collect(legacy, 137).0, collect(explicit, 4096).0);
}

#[test]
fn enhanced_preamble_and_fsk_id_have_exact_boundaries() {
    let encoder = SstvEncoder::new_with_envelope(
        image(SstvMode::Robot8Bw),
        SstvMode::Robot8Bw,
        SAMPLE_RATE,
        EncodeOptions::default(),
        SstvTransmissionEnvelope {
            enhanced_preamble: true,
            station_id: SstvStationId::Fsk {
                callsign: "BG5DRB".to_owned(),
            },
            post_image_gap_seconds: 0.5,
            end_guard_seconds: 0.3,
        },
    )
    .unwrap();
    let (samples, raster_start, raster_end) = collect(encoder, 257);
    assert_eq!(raster_start, 20_520);
    assert_eq!(raster_end - raster_start, 95_040);

    let preamble = [
        1900.0, 1500.0, 1900.0, 1500.0, 2300.0, 1500.0, 2300.0, 1500.0,
    ];
    for (index, expected) in preamble.into_iter().enumerate() {
        let start = index * 1_200 + 120;
        assert_eq!(
            dominant_frequency(&samples[start..start + 960], &[1500.0, 1900.0, 2300.0]),
            expected
        );
    }

    let gap_start = raster_end as usize;
    assert!(
        samples[gap_start..gap_start + 6_000]
            .iter()
            .all(|sample| *sample == 0.0)
    );
    let fsk_start = gap_start + 6_000;
    assert_eq!(
        dominant_frequency(
            &samples[fsk_start + 120..fsk_start + 3_480],
            &[1500.0, 1900.0, 2100.0]
        ),
        1500.0
    );
    let guard_start = samples.len() - 3_600;
    assert!(samples[guard_start..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn enhanced_envelope_preserves_vis_self_decode() {
    for mode in [
        SstvMode::Robot36,
        SstvMode::Robot8Bw,
        SstvMode::Martin1,
        SstvMode::Pd120,
    ] {
        let encoder = SstvEncoder::new_with_envelope(
            image(mode),
            mode,
            SAMPLE_RATE,
            EncodeOptions::default(),
            SstvTransmissionEnvelope {
                enhanced_preamble: true,
                station_id: SstvStationId::Cw {
                    callsign: "BG5DRB".to_owned(),
                    wpm: 20,
                    tone_hz: 800.0,
                },
                post_image_gap_seconds: 0.5,
                end_guard_seconds: 0.3,
            },
        )
        .unwrap();
        let (samples, _, _) = collect(encoder, 1_013);
        let mut decoder = SstvDecoder::new(SAMPLE_RATE, DecoderConfig::default()).unwrap();
        let mut events = Vec::new();
        for chunk in samples.chunks(997) {
            decoder.process_into(chunk, &mut events).unwrap();
        }
        decoder.finish_into(&mut events).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            DecodeEvent::ImageStarted { mode: decoded, .. } if *decoded == mode
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DecodeEvent::ImageCompleted { mode: decoded, .. } if *decoded == mode
        )));
    }
}

#[test]
fn envelope_validation_is_explicit() {
    let error = SstvEncoder::new_with_envelope(
        image(SstvMode::Robot8Bw),
        SstvMode::Robot8Bw,
        SAMPLE_RATE,
        EncodeOptions {
            include_vis_header: false,
            ..EncodeOptions::default()
        },
        SstvTransmissionEnvelope {
            enhanced_preamble: true,
            ..SstvTransmissionEnvelope::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires include_vis_header"));

    let error = SstvEncoder::new_with_envelope(
        image(SstvMode::Robot8Bw),
        SstvMode::Robot8Bw,
        SAMPLE_RATE,
        EncodeOptions::default(),
        SstvTransmissionEnvelope {
            station_id: SstvStationId::Fsk {
                callsign: "bg5drb".to_owned(),
            },
            ..SstvTransmissionEnvelope::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("uppercase"));
}
