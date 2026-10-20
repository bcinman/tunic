//! Measure the public processor against analog prototypes, not its coefficient or graph code.
use std::f64::consts::PI;
use std::num::NonZeroUsize;

use rustfft::{FftPlanner, num_complex::Complex};
use tunic_core::{
    AudioFormat, Chain, Equalizer, Filter, FilterId, FilterKind, FilterParameters, FrequencyHz,
    GainDb, Processor, QualityFactor, SampleRateHz,
};

fn filter(kind: FilterKind, frequency: f64, gain: f64, quality: f64) -> Filter {
    Filter {
        id: FilterId::try_new(1).unwrap(),
        parameters: FilterParameters {
            kind,
            frequency: FrequencyHz::try_new(frequency).unwrap(),
            gain: GainDb::try_new(gain).unwrap(),
            quality_factor: QualityFactor::try_new(quality).unwrap(),
        },
    }
}

fn format(rate: f64) -> AudioFormat {
    AudioFormat {
        sample_rate: SampleRateHz::try_new(rate).unwrap(),
        maximum_frame_count: NonZeroUsize::new(1024).unwrap(),
    }
}

// W3C Audio EQ Cookbook, analog H(s) prototypes and prewarped bilinear substitution:
// https://www.w3.org/TR/audio-eq-cookbook/
// At a digital frequency f, s = j tan(pi f/fs) / tan(pi f0/fs).
// This deliberately does not construct digital biquad coefficients.
fn reference(filter: &Filter, frequency: f64, rate: f64) -> Complex<f64> {
    let parameters = filter.parameters;
    let amplitude = 10.0_f64.powf(parameters.gain.into_inner() / 40.0);
    let quality = parameters.quality_factor.into_inner();
    let ratio =
        (PI * frequency / rate).tan() / (PI * parameters.frequency.into_inner() / rate).tan();
    let squared = -ratio * ratio;
    match parameters.kind {
        FilterKind::Peaking => {
            Complex::new(1.0 + squared, ratio * amplitude / quality)
                / Complex::new(1.0 + squared, ratio / (amplitude * quality))
        }
        FilterKind::LowShelf => {
            amplitude * Complex::new(squared + amplitude, ratio * amplitude.sqrt() / quality)
                / Complex::new(
                    amplitude * squared + 1.0,
                    ratio * amplitude.sqrt() / quality,
                )
        }
        FilterKind::HighShelf => {
            amplitude
                * Complex::new(
                    amplitude * squared + 1.0,
                    ratio * amplitude.sqrt() / quality,
                )
                / Complex::new(squared + amplitude, ratio * amplitude.sqrt() / quality)
        }
    }
}

#[test]
fn impulse_response_matches_independent_analog_reference() {
    let mut worst_db = 0.0_f64;
    let mut worst_complex = 0.0_f64;
    let mut worst_case = String::new();
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        // At least half a second accommodates the slowest pole in this matrix.
        let length = (rate as usize / 2).next_power_of_two();
        let fft = FftPlanner::new().plan_fft_forward(length);
        for kind in [
            FilterKind::Peaking,
            FilterKind::LowShelf,
            FilterKind::HighShelf,
        ] {
            for center in [40.0, 1_234.0, rate * 0.45] {
                for gain in [-12.0, 9.0] {
                    for quality in [0.5, 2.5] {
                        let filter = filter(kind, center, gain, quality);
                        let chain = Chain {
                            equalizer: Equalizer {
                                preamp: GainDb::try_new(-6.0).unwrap(),
                                filters: vec![filter],
                            },
                        };
                        let (mut processor, _controller) =
                            Processor::new(format(rate), chain, false).unwrap();
                        let mut signal = vec![0.0_f32; length * 2];
                        signal[0] = 0.25;
                        for block in signal.chunks_mut(512) {
                            processor.process(block);
                        }
                        assert!(signal.iter().all(|sample| sample.is_finite()));
                        assert!(
                            signal
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .all(|frame| frame[1] == 0.0),
                            "channel leakage"
                        );
                        let mut spectrum: Vec<_> = signal
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|frame| Complex::new(f64::from(frame[0]) / 0.25, 0.0))
                            .collect();
                        fft.process(&mut spectrum);
                        for point in 0..128 {
                            let frequency =
                                20.0 * (rate * 0.49 / 20.0).powf(f64::from(point) / 127.0);
                            let bin = (frequency * length as f64 / rate).round() as usize;
                            let frequency = bin as f64 * rate / length as f64;
                            let expected =
                                reference(&filter, frequency, rate) * 10.0_f64.powf(-6.0 / 20.0);
                            let measured = spectrum[bin];
                            let db_error =
                                (20.0 * (measured.norm() / expected.norm()).log10()).abs();
                            let complex_error = (measured - expected).norm() / expected.norm();
                            if db_error > worst_db {
                                worst_case = format!(
                                    "{kind:?} fs={rate} center={center} gain={gain} Q={quality} f={frequency}"
                                );
                                worst_db = db_error;
                            }
                            worst_complex = worst_complex.max(complex_error);
                            assert!(db_error.is_finite() && complex_error.is_finite());
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "Worst response error: {worst_db:.6} dB ({worst_case}); relative complex error: {worst_complex:.6}"
    );
    assert!(worst_db < 0.05 && worst_complex < 0.01);
}

#[test]
fn stereo_stream_is_block_size_independent_and_bypass_nulls_exactly() {
    let chain = Chain {
        equalizer: Equalizer {
            preamp: GainDb::try_new(-9.0).unwrap(),
            filters: vec![
                filter(FilterKind::LowShelf, 83.0, 6.0, 0.707),
                Filter {
                    id: FilterId::try_new(2).unwrap(),
                    ..filter(FilterKind::Peaking, 2_731.0, -8.0, 3.0)
                },
            ],
        },
    };
    // Distinct deterministic channels exercise history, sign, and stereo isolation.
    let input: Vec<f32> = (0..96_000)
        .map(|index| {
            let value = (index as u32)
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            (f64::from(value) / f64::from(u32::MAX) - 0.5) as f32
        })
        .collect();
    for rate in [44_100.0, 96_000.0] {
        let mut expected = input.clone();
        let (mut processor, _) = Processor::new(format(rate), chain.clone(), false).unwrap();
        for block in expected.chunks_mut(2048) {
            processor.process(block);
        }
        for frames in [1, 7, 127, 511] {
            let (mut processor, controller) =
                Processor::new(format(rate), chain.clone(), false).unwrap();
            let mut actual = input.clone();
            for block in actual.chunks_mut(frames * 2) {
                processor.process(block);
            }
            assert_eq!(actual, expected, "block size={frames}, rate={rate}");
            controller.set_bypassed(true);
            let mut bypass = input.clone();
            for block in bypass.chunks_mut(frames * 2) {
                processor.process(block);
            }
            assert_eq!(
                bypass, input,
                "bypass must null exactly, including after nonzero filter history"
            );
        }
    }
}
