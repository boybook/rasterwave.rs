use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rasterwave::fax::{FaxEncodeOptions, FaxEncoder, FaxIoc, FaxLpm, FaxSpec};
use rasterwave::{
    DecodeEventRef, DecodeSink, DecoderConfig, EncodeOptions, GrayImage, Rgb, RgbImage,
    SstvDecoder, SstvEncoder, SstvMode, encode_sstv,
};

struct NullSink;

impl DecodeSink for NullSink {
    fn on_event(&mut self, event: DecodeEventRef<'_>) {
        std::hint::black_box(event);
    }
}

fn rgb_fixture(mode: SstvMode) -> RgbImage {
    let spec = mode.spec();
    let pixels = (0..spec.width * spec.height)
        .map(|index| {
            let x = index % spec.width;
            let y = index / spec.width;
            Rgb::new(
                (x * 255 / spec.width.max(1)) as u8,
                (y * 255 / spec.height.max(1)) as u8,
                ((x + y) % 256) as u8,
            )
        })
        .collect();
    RgbImage::new(spec.width, spec.height, pixels).unwrap()
}

fn benchmark_sstv_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("sstv/encode");
    for (mode, sample_rate) in [
        (SstvMode::Robot36, 12_000),
        (SstvMode::Martin1, 12_000),
        (SstvMode::Pd290, 12_000),
    ] {
        let image = rgb_fixture(mode);
        let estimated = ((mode.spec().line_seconds * f64::from(mode.spec().height)
            / f64::from(mode.spec().rows_per_line)
            + 0.91)
            * f64::from(sample_rate)) as u64;
        group.throughput(Throughput::Elements(estimated));
        group.bench_with_input(
            BenchmarkId::new(format!("{mode:?}"), sample_rate),
            &image,
            |b, image| {
                b.iter(|| {
                    let mut encoder = SstvEncoder::new(
                        image.clone(),
                        mode,
                        sample_rate,
                        EncodeOptions::default(),
                    )
                    .unwrap();
                    let mut chunk = [0.0_f32; 4096];
                    while !encoder.is_finished() {
                        std::hint::black_box(encoder.read_samples(&mut chunk));
                    }
                });
            },
        );
    }
    group.finish();
}

fn benchmark_sstv_decode(c: &mut Criterion) {
    let mode = SstvMode::Robot8Bw;
    let pcm = encode_sstv(rgb_fixture(mode), mode, 12_000).unwrap();
    let mut group = c.benchmark_group("sstv/decode");
    group.throughput(Throughput::Elements(pcm.len() as u64));
    group.bench_function("Robot8Bw/12000", |b| {
        b.iter(|| {
            let mut decoder = SstvDecoder::new(12_000, DecoderConfig::default()).unwrap();
            let mut sink = NullSink;
            for chunk in pcm.chunks(1024) {
                decoder.push_f32(chunk, &mut sink).unwrap();
            }
            decoder.finish(&mut sink).unwrap();
        });
    });
    group.finish();
}

fn benchmark_fax_encode(c: &mut Criterion) {
    let spec = FaxSpec::standard(FaxIoc::Ioc576, FaxLpm::LPM_120);
    let image = GrayImage::new(spec.width(), 8, vec![128; spec.width() as usize * 8]).unwrap();
    let samples = (8.0 * spec.lpm.line_seconds() * 12_000.0) as u64;
    let mut group = c.benchmark_group("radiofax/encode");
    group.throughput(Throughput::Elements(samples));
    group.bench_function("IOC576/120LPM/image-only", |b| {
        b.iter(|| {
            let mut encoder = FaxEncoder::new(
                image.clone(),
                spec,
                12_000,
                FaxEncodeOptions {
                    include_apt: false,
                    include_phasing: false,
                    ..FaxEncodeOptions::default()
                },
            )
            .unwrap();
            let mut chunk = [0.0_f32; 4096];
            while !encoder.is_finished() {
                std::hint::black_box(encoder.read_samples(&mut chunk));
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    benchmark_sstv_encode,
    benchmark_sstv_decode,
    benchmark_fax_encode
);
criterion_main!(benches);
