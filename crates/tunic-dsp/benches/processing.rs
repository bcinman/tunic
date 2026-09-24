use std::f32::consts::TAU;
use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor};

const SAMPLE_RATES_HZ: [u32; 3] = [44_100, 48_000, 96_000];
const BUFFER_FRAMES: [usize; 3] = [64, 256, 1_024];
const FILTER_COUNTS: [usize; 4] = [0, 5, 10, 20];
const FILTER_FREQUENCIES_HZ: [f64; 20] = [
    31.5, 50.0, 80.0, 125.0, 200.0, 315.0, 500.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0,
    2_500.0, 3_150.0, 4_000.0, 5_000.0, 6_300.0, 8_000.0, 10_000.0, 16_000.0,
];

fn benchmark_processing(criterion: &mut Criterion) {
    for sample_rate_hz in SAMPLE_RATES_HZ {
        let mut group = criterion.benchmark_group(format!("process_{sample_rate_hz}_hz"));
        group.warm_up_time(Duration::from_secs(1));
        group.measurement_time(Duration::from_secs(2));
        group.sample_size(50);

        for filter_count in FILTER_COUNTS {
            let equalizer = equalizer(filter_count);
            for frames in BUFFER_FRAMES {
                group.throughput(Throughput::Elements(frames as u64));
                group.bench_with_input(
                    BenchmarkId::new(format!("{filter_count}_filters"), frames),
                    &frames,
                    |bencher, &frames| {
                        bencher.iter_batched_ref(
                            || {
                                (
                                    PreparedGraph::prepare(
                                        black_box(&equalizer),
                                        f64::from(sample_rate_hz),
                                    )
                                    .unwrap(),
                                    input(frames, sample_rate_hz),
                                )
                            },
                            |(graph, frames)| {
                                graph.process(black_box(frames));
                                black_box(&*frames);
                            },
                            BatchSize::SmallInput,
                        );
                    },
                );
            }
        }
        group.finish();
    }
}

fn equalizer(filter_count: usize) -> Equalizer {
    let filters = FILTER_FREQUENCIES_HZ
        .into_iter()
        .take(filter_count)
        .enumerate()
        .map(|(index, frequency_hz)| {
            let frequency = FrequencyHz::new(frequency_hz).unwrap();
            let gain = GainDb::new(if index.is_multiple_of(2) { 4.5 } else { -3.0 }).unwrap();
            let quality_factor = QualityFactor::new(0.7 + (index % 4) as f64 * 0.35).unwrap();
            match index % 3 {
                0 => Filter::low_shelf(frequency, gain, quality_factor),
                1 => Filter::peaking(frequency, gain, quality_factor),
                _ => Filter::high_shelf(frequency, gain, quality_factor),
            }
        })
        .collect();
    Equalizer::with_filters(filters)
}

fn input(frames: usize, sample_rate_hz: u32) -> Vec<[f32; 2]> {
    (0..frames)
        .map(|frame| {
            let phase = TAU * 997.0 * frame as f32 / sample_rate_hz as f32;
            [phase.sin() * 0.25, (phase * 1.013).sin() * 0.2]
        })
        .collect()
}

criterion_group!(processing, benchmark_processing);
criterion_main!(processing);
