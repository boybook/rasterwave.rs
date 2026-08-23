use std::sync::Arc;

use rasterwave::{
    DecodeEvent, DecoderConfig, EncodeOptions, Rgb, RgbImage, SstvDecoder, SstvEncoder, SstvMode,
};

fn fixture() -> RgbImage {
    let spec = SstvMode::Robot8Bw.spec();
    let pixels = (0..spec.width * spec.height)
        .map(|index| {
            let value = (index % 256) as u8;
            Rgb::new(value, value, value)
        })
        .collect();
    RgbImage::new(spec.width, spec.height, pixels).unwrap()
}

fn encode_with_chunk(chunk_size: usize) -> Vec<f32> {
    let mut encoder = SstvEncoder::new(
        fixture(),
        SstvMode::Robot8Bw,
        12_000,
        EncodeOptions::default(),
    )
    .unwrap();
    let mut output = Vec::new();
    let mut chunk = vec![0.0; chunk_size];
    while !encoder.is_finished() {
        let count = encoder.read_samples(&mut chunk);
        output.extend_from_slice(&chunk[..count]);
    }
    output
}

#[test]
fn independent_sessions_are_deterministic_across_threads() {
    let chunk_sizes = [1, 7, 31, 127, 509, 1024, 4093, 8192];
    let encoders: Vec<_> = chunk_sizes
        .into_iter()
        .map(|chunk_size| std::thread::spawn(move || encode_with_chunk(chunk_size)))
        .collect();
    let encoded: Vec<_> = encoders
        .into_iter()
        .map(|thread| thread.join().expect("encoder thread must finish"))
        .collect();
    for candidate in &encoded[1..] {
        assert_eq!(candidate, &encoded[0]);
    }

    let pcm = Arc::new(encoded.into_iter().next().unwrap());
    let decoders: Vec<_> = chunk_sizes
        .into_iter()
        .map(|chunk_size| {
            let pcm = Arc::clone(&pcm);
            std::thread::spawn(move || {
                let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
                let mut events = Vec::new();
                for chunk in pcm.chunks(chunk_size) {
                    decoder.process_into(chunk, &mut events).unwrap();
                }
                decoder.finish_into(&mut events).unwrap();
                let lines = events
                    .iter()
                    .filter(|event| matches!(event, DecodeEvent::LineReady { .. }))
                    .count();
                let completed = events
                    .iter()
                    .any(|event| matches!(event, DecodeEvent::ImageCompleted { .. }));
                let mode = events.iter().find_map(|event| match event {
                    DecodeEvent::ImageStarted { mode, .. } => Some(*mode),
                    _ => None,
                });
                (lines, completed, mode)
            })
        })
        .collect();
    for decoder in decoders {
        assert_eq!(
            decoder.join().expect("decoder thread must finish"),
            (
                SstvMode::Robot8Bw.spec().height as usize,
                true,
                Some(SstvMode::Robot8Bw)
            )
        );
    }
}
