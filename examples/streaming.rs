use rasterwave::{
    DecodeEventRef, DecodeSink, DecoderConfig, EncodeOptions, Rgb, RgbImage, SstvDecoder,
    SstvEncoder, SstvMode,
};

#[derive(Default)]
struct RowCounter {
    rows: u32,
}

impl DecodeSink for RowCounter {
    fn on_event(&mut self, event: DecodeEventRef<'_>) {
        if let DecodeEventRef::LineReady {
            line_index, pixels, ..
        } = event
        {
            // Borrowed pixels are valid only until this callback returns.
            self.rows = self.rows.max(line_index + 1);
            println!("row {line_index}: {} pixels", pixels.len());
        }
    }
}

fn main() -> rasterwave::Result<()> {
    let sample_rate = 12_000;
    let mode = SstvMode::Robot8Bw;
    let spec = mode.spec();
    let image = RgbImage::filled(spec.width, spec.height, Rgb::new(32, 128, 224));

    let mut encoder = SstvEncoder::new(image, mode, sample_rate, EncodeOptions::default())?;
    let mut decoder = SstvDecoder::new(sample_rate, DecoderConfig::default())?;
    let mut sink = RowCounter::default();
    let mut pcm = [0.0_f32; 731];

    while !encoder.is_finished() {
        let written = encoder.read_samples(&mut pcm);
        decoder.push_f32(&pcm[..written], &mut sink)?;
    }
    decoder.finish(&mut sink)?;

    println!("decoded {} rows", sink.rows);
    Ok(())
}
