use std::f64::consts::TAU;

use crate::{Chain, Filter, FilterKind, ProcessorError, SampleRateHz};

/// A processing chain compiled for one sample rate.
pub(super) struct PreparedChain {
    preamp_gain: f32,
    filters: Vec<StereoBiquad>,
}

impl PreparedChain {
    pub(super) fn prepare(
        chain: &Chain,
        sample_rate: SampleRateHz,
    ) -> Result<Self, ProcessorError> {
        let preamp_gain = 10.0_f64.powf(chain.equalizer.preamp.into_inner() / 20.0) as f32;
        if !preamp_gain.is_finite() {
            return Err(ProcessorError::PreampOutOfRange);
        }

        let sample_rate = sample_rate.into_inner();
        let filters = chain
            .equalizer
            .filters
            .iter()
            .copied()
            .enumerate()
            .map(|(index, filter)| StereoBiquad::prepare(filter, sample_rate, index))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            preamp_gain,
            filters,
        })
    }

    /// Processes interleaved stereo samples in place without allocating.
    pub(super) fn process(&mut self, samples: &mut [f32]) {
        if self.preamp_gain != 1.0 {
            for sample in samples.iter_mut() {
                *sample *= self.preamp_gain;
            }
        }
        for filter in &mut self.filters {
            for frame in samples.as_chunks_mut::<2>().0 {
                frame[0] = filter.left.process(frame[0]);
                frame[1] = filter.right.process(frame[1]);
            }
        }
    }
}

struct StereoBiquad {
    left: Biquad,
    right: Biquad,
}

impl StereoBiquad {
    fn prepare(filter: Filter, sample_rate: f64, index: usize) -> Result<Self, ProcessorError> {
        if filter.frequency.into_inner() >= sample_rate / 2.0 {
            return Err(ProcessorError::FilterAtOrAboveNyquist { filter: index });
        }

        let coefficients = Coefficients::for_filter(filter, sample_rate);
        if !coefficients.is_finite() || !coefficients.is_stable() {
            return Err(ProcessorError::UnstableFilter { filter: index });
        }

        Ok(Self {
            left: Biquad::new(coefficients),
            right: Biquad::new(coefficients),
        })
    }
}

#[derive(Clone, Copy)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    fn for_filter(filter: Filter, sample_rate: f64) -> Self {
        match filter.kind {
            FilterKind::Peaking => Self::peaking(filter, sample_rate),
            FilterKind::LowShelf => Self::low_shelf(filter, sample_rate),
            FilterKind::HighShelf => Self::high_shelf(filter, sample_rate),
        }
    }

    fn peaking(filter: Filter, sample_rate: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain.into_inner() / 40.0);
        let angular_frequency = TAU * filter.frequency.into_inner() / sample_rate;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor.into_inner());
        let cosine = angular_frequency.cos();
        let a0 = 1.0 + alpha / amplitude;
        Self {
            b0: ((1.0 + alpha * amplitude) / a0) as f32,
            b1: (-2.0 * cosine / a0) as f32,
            b2: ((1.0 - alpha * amplitude) / a0) as f32,
            a1: (-2.0 * cosine / a0) as f32,
            a2: ((1.0 - alpha / amplitude) / a0) as f32,
        }
    }

    fn low_shelf(filter: Filter, sample_rate: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain.into_inner() / 40.0);
        let angular_frequency = TAU * filter.frequency.into_inner() / sample_rate;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor.into_inner());
        let cosine = angular_frequency.cos();
        let two_sqrt_a_alpha = 2.0 * amplitude.sqrt() * alpha;
        let a0 = (amplitude + 1.0) + (amplitude - 1.0) * cosine + two_sqrt_a_alpha;
        Self {
            b0: (amplitude * ((amplitude + 1.0) - (amplitude - 1.0) * cosine + two_sqrt_a_alpha)
                / a0) as f32,
            b1: (2.0 * amplitude * ((amplitude - 1.0) - (amplitude + 1.0) * cosine) / a0) as f32,
            b2: (amplitude * ((amplitude + 1.0) - (amplitude - 1.0) * cosine - two_sqrt_a_alpha)
                / a0) as f32,
            a1: (-2.0 * ((amplitude - 1.0) + (amplitude + 1.0) * cosine) / a0) as f32,
            a2: (((amplitude + 1.0) + (amplitude - 1.0) * cosine - two_sqrt_a_alpha) / a0) as f32,
        }
    }

    fn high_shelf(filter: Filter, sample_rate: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain.into_inner() / 40.0);
        let angular_frequency = TAU * filter.frequency.into_inner() / sample_rate;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor.into_inner());
        let cosine = angular_frequency.cos();
        let two_sqrt_a_alpha = 2.0 * amplitude.sqrt() * alpha;
        let a0 = (amplitude + 1.0) - (amplitude - 1.0) * cosine + two_sqrt_a_alpha;
        Self {
            b0: (amplitude * ((amplitude + 1.0) + (amplitude - 1.0) * cosine + two_sqrt_a_alpha)
                / a0) as f32,
            b1: (-2.0 * amplitude * ((amplitude - 1.0) + (amplitude + 1.0) * cosine) / a0) as f32,
            b2: (amplitude * ((amplitude + 1.0) + (amplitude - 1.0) * cosine - two_sqrt_a_alpha)
                / a0) as f32,
            a1: (2.0 * ((amplitude - 1.0) - (amplitude + 1.0) * cosine) / a0) as f32,
            a2: (((amplitude + 1.0) - (amplitude - 1.0) * cosine - two_sqrt_a_alpha) / a0) as f32,
        }
    }

    fn is_finite(self) -> bool {
        [self.b0, self.b1, self.b2, self.a1, self.a2]
            .into_iter()
            .all(f32::is_finite)
    }

    fn is_stable(self) -> bool {
        let a1 = f64::from(self.a1);
        let a2 = f64::from(self.a2);
        a2.abs() < 1.0 && 1.0 + a1 + a2 > 0.0 && 1.0 - a1 + a2 > 0.0
    }
}

struct Biquad {
    coefficients: Coefficients,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(coefficients: Coefficients) -> Self {
        Self {
            coefficients,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.coefficients.b0 * input + self.z1;
        self.z1 = self.coefficients.b1 * input - self.coefficients.a1 * output + self.z2;
        self.z2 = self.coefficients.b2 * input - self.coefficients.a2 * output;
        output
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::PreparedChain;
    use crate::{
        Chain, Equalizer, Filter, FilterKind, FrequencyHz, GainDb, ProcessorError, QualityFactor,
        SampleRateHz,
    };

    const SAMPLE_RATE: usize = 48_000;

    #[test]
    fn flat_chain_is_identity_and_preamp_applies_linear_gain() {
        let mut flat = PreparedChain::prepare(&Chain::default(), sample_rate()).unwrap();
        let mut flat_samples = [0.25, -0.5, -0.75, 0.125];
        flat.process(&mut flat_samples);
        assert_eq!(flat_samples, [0.25, -0.5, -0.75, 0.125]);

        let mut amplified = PreparedChain::prepare(
            &Chain {
                equalizer: Equalizer {
                    preamp: GainDb::try_new(6.0).unwrap(),
                    filters: Vec::new(),
                },
            },
            sample_rate(),
        )
        .unwrap();
        let mut amplified_samples = [0.25, -0.5];
        amplified.process(&mut amplified_samples);
        let gain = 10.0_f32.powf(6.0 / 20.0);
        assert!((amplified_samples[0] - 0.25 * gain).abs() < f32::EPSILON);
        assert!((amplified_samples[1] + 0.5 * gain).abs() < f32::EPSILON);
    }

    #[test]
    fn peaking_filter_applies_requested_gain_to_asymmetric_stereo() {
        let mut prepared = prepare(vec![filter(FilterKind::Peaking, 1_000.0, 6.0)]).unwrap();
        let mut samples = sine(1_000.0, 0.25);

        prepared.process(&mut samples);

        let settled = &samples[(SAMPLE_RATE / 5) * 2..];
        let left_rms = channel_rms(settled, 0);
        let right_rms = channel_rms(settled, 1);
        let expected_gain = 10.0_f32.powf(6.0 / 20.0);
        assert!((left_rms / (1.0 / 2.0_f32.sqrt()) - expected_gain).abs() < 0.01);
        assert!((left_rms / right_rms - 4.0).abs() < 0.01);
    }

    #[test]
    fn shelf_filters_shape_the_expected_side_of_the_spectrum() {
        for (kind, center, affected, preserved) in [
            (FilterKind::LowShelf, 1_000.0, 100.0, 10_000.0),
            (FilterKind::HighShelf, 2_000.0, 12_000.0, 100.0),
        ] {
            assert!(
                (measured_gain(filter(kind, center, 6.0), affected) - db_gain(6.0)).abs() < 0.02
            );
            assert!((measured_gain(filter(kind, center, 6.0), preserved) - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn mixed_filters_form_one_cascade() {
        let filters = vec![
            filter(FilterKind::LowShelf, 1_000.0, 6.0),
            filter(FilterKind::Peaking, 1_000.0, 3.0),
            filter(FilterKind::HighShelf, 1_000.0, 6.0),
        ];
        let mut prepared = prepare(filters).unwrap();
        let mut samples = sine(1_000.0, 1.0);

        prepared.process(&mut samples);

        let measured = channel_rms(&samples[(SAMPLE_RATE / 5) * 2..], 0) / (1.0 / 2.0_f32.sqrt());
        assert!((measured - db_gain(9.0)).abs() < 0.02);
    }

    #[test]
    fn preparation_reports_the_invalid_filter_index() {
        let result = prepare(vec![
            filter(FilterKind::Peaking, 1_000.0, 3.0),
            filter(FilterKind::Peaking, 24_000.0, 3.0),
        ]);
        assert!(matches!(
            result,
            Err(ProcessorError::FilterAtOrAboveNyquist { filter: 1 })
        ));

        for kind in [
            FilterKind::Peaking,
            FilterKind::LowShelf,
            FilterKind::HighShelf,
        ] {
            assert!(matches!(
                prepare(vec![filter(kind, 1.0, 6.0)]),
                Err(ProcessorError::UnstableFilter { filter: 0 })
            ));
        }
    }

    #[test]
    fn preparation_rejects_unrepresentable_gain() {
        let chain = Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(f64::MAX).unwrap(),
                filters: Vec::new(),
            },
        };
        assert!(matches!(
            PreparedChain::prepare(&chain, sample_rate()),
            Err(ProcessorError::PreampOutOfRange)
        ));

        assert!(matches!(
            prepare(vec![filter(FilterKind::Peaking, 1_000.0, f64::MAX)]),
            Err(ProcessorError::UnstableFilter { filter: 0 })
        ));
    }

    fn prepare(filters: Vec<Filter>) -> Result<PreparedChain, ProcessorError> {
        PreparedChain::prepare(
            &Chain {
                equalizer: Equalizer {
                    preamp: GainDb::default(),
                    filters,
                },
            },
            sample_rate(),
        )
    }

    fn filter(kind: FilterKind, frequency: f64, gain: f64) -> Filter {
        Filter {
            kind,
            frequency: FrequencyHz::try_new(frequency).unwrap(),
            gain: GainDb::try_new(gain).unwrap(),
            quality_factor: QualityFactor::try_new(1.0).unwrap(),
        }
    }

    fn sample_rate() -> SampleRateHz {
        SampleRateHz::try_new(SAMPLE_RATE as f64).unwrap()
    }

    fn sine(frequency: f32, right_scale: f32) -> Vec<f32> {
        (0..SAMPLE_RATE)
            .flat_map(|frame| {
                let sample = (TAU * frequency * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample * right_scale]
            })
            .collect()
    }

    fn measured_gain(filter: Filter, frequency: f32) -> f32 {
        let mut prepared = prepare(vec![filter]).unwrap();
        let mut samples = sine(frequency, 1.0);
        prepared.process(&mut samples);
        channel_rms(&samples[SAMPLE_RATE..], 0) / (1.0 / 2.0_f32.sqrt())
    }

    fn db_gain(gain: f32) -> f32 {
        10.0_f32.powf(gain / 20.0)
    }

    fn channel_rms(samples: &[f32], channel: usize) -> f32 {
        let frames = samples.len() / 2;
        (samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame[channel] * frame[channel])
            .sum::<f32>()
            / frames as f32)
            .sqrt()
    }
}
