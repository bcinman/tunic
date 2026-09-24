use std::f64::consts::TAU;

use crate::{Equalizer, EqualizerError, Filter, FilterKind};

/// A stateful stereo processing graph prepared for one sample rate.
#[derive(Debug, Default)]
pub struct PreparedGraph {
    filters: Vec<StereoBiquad>,
}

impl PreparedGraph {
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    pub fn prepare(equalizer: &Equalizer, sample_rate_hz: f64) -> Result<Self, EqualizerError> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(EqualizerError::new(
                "sample rate must be finite and greater than zero",
            ));
        }
        let filters = equalizer
            .filters()
            .iter()
            .copied()
            .map(|filter| StereoBiquad::prepare(filter, sample_rate_hz))
            .collect::<Result<_, _>>()?;
        Ok(Self { filters })
    }

    /// Process stereo frames in place without allocating.
    pub fn process(&mut self, frames: &mut [[f32; 2]]) {
        for filter in &mut self.filters {
            for [left, right] in frames.iter_mut() {
                *left = filter.left.process(*left);
                *right = filter.right.process(*right);
            }
        }
    }

    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.left.reset();
            filter.right.reset();
        }
    }

    #[must_use]
    pub fn latency_frames(&self) -> usize {
        0
    }
}

#[derive(Debug)]
struct StereoBiquad {
    left: Biquad,
    right: Biquad,
}

impl StereoBiquad {
    fn prepare(filter: Filter, sample_rate_hz: f64) -> Result<Self, EqualizerError> {
        let frequency_hz = filter.frequency().get();
        if frequency_hz >= sample_rate_hz / 2.0 {
            return Err(EqualizerError::new(format!(
                "filter frequency {frequency_hz} Hz must be below Nyquist for {sample_rate_hz} Hz audio"
            )));
        }
        let coefficients = Coefficients::for_filter(filter, sample_rate_hz);
        if !coefficients.is_finite() || !coefficients.is_stable() {
            return Err(EqualizerError::new(
                "filter parameters do not produce a stable finite filter",
            ));
        }
        Ok(Self {
            left: Biquad::new(coefficients),
            right: Biquad::new(coefficients),
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    fn for_filter(filter: Filter, sample_rate_hz: f64) -> Self {
        match filter.kind() {
            FilterKind::Peaking => Self::peaking(filter, sample_rate_hz),
            FilterKind::LowShelf => Self::low_shelf(filter, sample_rate_hz),
            FilterKind::HighShelf => Self::high_shelf(filter, sample_rate_hz),
        }
    }

    fn peaking(filter: Filter, sample_rate_hz: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain().get() / 40.0);
        let angular_frequency = TAU * filter.frequency().get() / sample_rate_hz;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor().get());
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

    fn low_shelf(filter: Filter, sample_rate_hz: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain().get() / 40.0);
        let angular_frequency = TAU * filter.frequency().get() / sample_rate_hz;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor().get());
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

    fn high_shelf(filter: Filter, sample_rate_hz: f64) -> Self {
        let amplitude = 10.0_f64.powf(filter.gain().get() / 40.0);
        let angular_frequency = TAU * filter.frequency().get() / sample_rate_hz;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor().get());
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

#[derive(Debug)]
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

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use crate::{Equalizer, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor};

    #[test]
    fn identity_graph_preserves_asymmetric_stereo_frames() {
        let mut graph = PreparedGraph::identity();
        let mut frames = [[0.25, -0.5], [-0.75, 0.125], [1.0, -1.0]];

        graph.process(&mut frames);

        assert_eq!(frames, [[0.25, -0.5], [-0.75, 0.125], [1.0, -1.0]]);
        assert_eq!(graph.latency_frames(), 0);
    }

    #[test]
    fn peaking_filter_applies_requested_gain_at_center_frequency_to_both_channels() {
        const SAMPLE_RATE: usize = 48_000;
        const FREQUENCY: f32 = 1_000.0;
        let filter = Filter::peaking(
            FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let mut graph =
            PreparedGraph::prepare(&Equalizer::with_filter(filter), SAMPLE_RATE as f64).unwrap();
        let mut frames = (0..SAMPLE_RATE)
            .map(|frame| {
                let sample = (TAU * FREQUENCY * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample * 0.25]
            })
            .collect::<Vec<_>>();

        graph.process(&mut frames);

        let settled = &frames[SAMPLE_RATE / 5..];
        let left_rms = channel_rms(settled, 0);
        let right_rms = channel_rms(settled, 1);
        let expected_gain = 10.0_f32.powf(6.0 / 20.0);
        assert!((left_rms / (1.0 / 2.0_f32.sqrt()) - expected_gain).abs() < 0.01);
        assert!((left_rms / right_rms - 4.0).abs() < 0.01);
    }

    #[test]
    fn mixed_filters_are_applied_as_an_ordered_cascade() {
        const SAMPLE_RATE: usize = 48_000;
        const FREQUENCY: f32 = 1_000.0;
        let equalizer = Equalizer::with_filters(vec![
            Filter::low_shelf(
                FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
                GainDb::new(6.0).unwrap(),
                QualityFactor::new(1.0).unwrap(),
            ),
            Filter::peaking(
                FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
                GainDb::new(3.0).unwrap(),
                QualityFactor::new(1.0).unwrap(),
            ),
            Filter::high_shelf(
                FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
                GainDb::new(6.0).unwrap(),
                QualityFactor::new(1.0).unwrap(),
            ),
        ]);
        let mut graph = PreparedGraph::prepare(&equalizer, SAMPLE_RATE as f64).unwrap();
        let mut frames = (0..SAMPLE_RATE)
            .map(|frame| {
                let sample = (TAU * FREQUENCY * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        graph.process(&mut frames);

        let measured_gain = channel_rms(&frames[SAMPLE_RATE / 5..], 0) / (1.0 / 2.0_f32.sqrt());
        let expected_gain = 10.0_f32.powf(9.0 / 20.0);
        assert!((measured_gain - expected_gain).abs() < 0.02);
    }

    #[test]
    fn shelves_apply_gain_on_one_side_and_preserve_the_other() {
        for (filter, affected_hz, preserved_hz, gain_db) in [
            (low_shelf(6.0), 100.0, 10_000.0, 6.0),
            (low_shelf(-6.0), 100.0, 10_000.0, -6.0),
            (high_shelf(6.0), 12_000.0, 100.0, 6.0),
            (high_shelf(-6.0), 12_000.0, 100.0, -6.0),
        ] {
            assert!((measured_gain(filter, affected_hz) - db_gain(gain_db)).abs() < 0.02);
            assert!((measured_gain(filter, preserved_hz) - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn shelf_quality_factor_shapes_the_transition() {
        let frequency = FrequencyHz::new(1_000.0).unwrap();
        let gain = GainDb::new(6.0).unwrap();
        let cases = [
            (
                Filter::low_shelf(frequency, gain, QualityFactor::new(0.5).unwrap()),
                500.0,
                1.735_935,
            ),
            (
                Filter::low_shelf(frequency, gain, QualityFactor::new(2.0).unwrap()),
                500.0,
                2.380_045,
            ),
            (
                Filter::high_shelf(frequency, gain, QualityFactor::new(0.5).unwrap()),
                2_000.0,
                1.737_173,
            ),
            (
                Filter::high_shelf(frequency, gain, QualityFactor::new(2.0).unwrap()),
                2_000.0,
                2.377_1,
            ),
        ];

        for (filter, probe_hz, expected_gain) in cases {
            assert!((measured_gain(filter, probe_hz) - expected_gain).abs() < 0.002);
        }
    }

    #[test]
    fn preparation_rejects_invalid_stream_specific_filters() {
        let at_nyquist = Filter::peaking(
            FrequencyHz::new(24_000.0).unwrap(),
            GainDb::new(3.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let overflowing = Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(f64::MAX).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_filter(at_nyquist), 48_000.0).is_err());
        assert!(PreparedGraph::prepare(&Equalizer::with_filter(overflowing), 48_000.0).is_err());
    }

    #[test]
    fn preparation_rejects_coefficients_destabilized_by_f32_quantization() {
        let frequency = FrequencyHz::new(1.0).unwrap();
        let gain = GainDb::new(6.0).unwrap();
        let quality_factor = QualityFactor::new(1.0).unwrap();
        for filter in [
            Filter::peaking(frequency, gain, quality_factor),
            Filter::low_shelf(frequency, gain, quality_factor),
            Filter::high_shelf(frequency, gain, quality_factor),
        ] {
            assert!(PreparedGraph::prepare(&Equalizer::with_filter(filter), 48_000.0).is_err());
        }
    }

    fn low_shelf(gain_db: f64) -> Filter {
        Filter::low_shelf(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        )
    }

    fn high_shelf(gain_db: f64) -> Filter {
        Filter::high_shelf(
            FrequencyHz::new(2_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        )
    }

    fn measured_gain(filter: Filter, frequency_hz: f32) -> f32 {
        const SAMPLE_RATE: usize = 48_000;
        let mut graph =
            PreparedGraph::prepare(&Equalizer::with_filter(filter), SAMPLE_RATE as f64).unwrap();
        let mut frames = (0..SAMPLE_RATE)
            .map(|frame| {
                let sample = (TAU * frequency_hz * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();
        graph.process(&mut frames);
        channel_rms(&frames[SAMPLE_RATE / 2..], 0) / (1.0 / 2.0_f32.sqrt())
    }

    fn db_gain(gain_db: f32) -> f32 {
        10.0_f32.powf(gain_db / 20.0)
    }

    fn channel_rms(frames: &[[f32; 2]], channel: usize) -> f32 {
        (frames
            .iter()
            .map(|frame| frame[channel] * frame[channel])
            .sum::<f32>()
            / frames.len() as f32)
            .sqrt()
    }
}
