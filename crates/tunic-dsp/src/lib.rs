//! Portable audio-processing definitions and algorithms.

use std::f64::consts::TAU;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrequencyHz(f64);

impl FrequencyHz {
    pub fn new(value: f64) -> Result<Self, EqualizerError> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(EqualizerError::new(
                "filter frequency must be finite and greater than zero",
            ))
        }
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainDb(f64);

impl GainDb {
    pub fn new(value: f64) -> Result<Self, EqualizerError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(EqualizerError::new("filter gain must be finite"))
        }
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualityFactor(f64);

impl QualityFactor {
    pub fn new(value: f64) -> Result<Self, EqualizerError> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(EqualizerError::new(
                "filter Q must be finite and greater than zero",
            ))
        }
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeakingFilter {
    frequency: FrequencyHz,
    gain: GainDb,
    quality_factor: QualityFactor,
}

impl PeakingFilter {
    #[must_use]
    pub fn new(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self {
            frequency,
            gain,
            quality_factor,
        }
    }

    #[must_use]
    pub fn frequency(self) -> FrequencyHz {
        self.frequency
    }

    #[must_use]
    pub fn gain(self) -> GainDb {
        self.gain
    }

    #[must_use]
    pub fn quality_factor(self) -> QualityFactor {
        self.quality_factor
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equalizer {
    peaking_filters: Vec<PeakingFilter>,
}

impl Equalizer {
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_peaking_filter(filter: PeakingFilter) -> Self {
        Self {
            peaking_filters: vec![filter],
        }
    }

    #[must_use]
    pub fn with_peaking_filters(filters: Vec<PeakingFilter>) -> Self {
        Self {
            peaking_filters: filters,
        }
    }

    #[must_use]
    pub fn peaking_filters(&self) -> &[PeakingFilter] {
        &self.peaking_filters
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EqualizerError(String);

impl EqualizerError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EqualizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for EqualizerError {}

/// A processing graph prepared for a concrete audio stream.
#[derive(Debug, Default)]
pub struct PreparedGraph {
    peaking_filters: Vec<StereoBiquad>,
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
        let peaking_filters = equalizer
            .peaking_filters()
            .iter()
            .copied()
            .map(|filter| StereoBiquad::peaking(filter, sample_rate_hz))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { peaking_filters })
    }

    /// Process stereo audio in place without allocating.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        for filter in &mut self.peaking_filters {
            for (left, right) in left.iter_mut().zip(right.iter_mut()) {
                *left = filter.left.process(*left);
                *right = filter.right.process(*right);
            }
        }
    }

    /// Process interleaved stereo frames in place without allocating.
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        debug_assert_eq!(samples.len() % 2, 0);
        for filter in &mut self.peaking_filters {
            for frame in samples.as_chunks_mut::<2>().0 {
                frame[0] = filter.left.process(frame[0]);
                frame[1] = filter.right.process(frame[1]);
            }
        }
    }

    pub fn reset(&mut self) {
        for filter in &mut self.peaking_filters {
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
    fn peaking(filter: PeakingFilter, sample_rate_hz: f64) -> Result<Self, EqualizerError> {
        let frequency_hz = filter.frequency().get();
        if frequency_hz >= sample_rate_hz / 2.0 {
            return Err(EqualizerError::new(format!(
                "filter frequency {frequency_hz} Hz must be below Nyquist for {sample_rate_hz} Hz audio"
            )));
        }
        let amplitude = 10.0_f64.powf(filter.gain().get() / 40.0);
        let angular_frequency = TAU * frequency_hz / sample_rate_hz;
        let alpha = angular_frequency.sin() / (2.0 * filter.quality_factor().get());
        let cosine = angular_frequency.cos();
        let a0 = 1.0 + alpha / amplitude;
        let coefficients = Coefficients {
            b0: ((1.0 + alpha * amplitude) / a0) as f32,
            b1: (-2.0 * cosine / a0) as f32,
            b2: ((1.0 - alpha * amplitude) / a0) as f32,
            a1: (-2.0 * cosine / a0) as f32,
            a2: ((1.0 - alpha / amplitude) / a0) as f32,
        };
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

    use super::{Equalizer, FrequencyHz, GainDb, PeakingFilter, PreparedGraph, QualityFactor};

    #[test]
    fn identity_graph_preserves_asymmetric_stereo_samples() {
        let mut graph = PreparedGraph::identity();
        let mut left = [0.25, -0.75, 1.0];
        let mut right = [-0.5, 0.125, -1.0];

        graph.process(&mut left, &mut right);

        assert_eq!(left, [0.25, -0.75, 1.0]);
        assert_eq!(right, [-0.5, 0.125, -1.0]);

        let mut interleaved = [0.25, -0.5, -0.75, 0.125, 1.0, -1.0];
        graph.process_interleaved_stereo(&mut interleaved);
        assert_eq!(interleaved, [0.25, -0.5, -0.75, 0.125, 1.0, -1.0]);
        assert_eq!(graph.latency_frames(), 0);
    }

    #[test]
    fn peaking_filter_applies_requested_gain_at_center_frequency_to_both_channels() {
        const SAMPLE_RATE: usize = 48_000;
        const FREQUENCY: f32 = 1_000.0;
        let filter = PeakingFilter::new(
            FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let mut graph =
            PreparedGraph::prepare(&Equalizer::with_peaking_filter(filter), SAMPLE_RATE as f64)
                .unwrap();
        let mut samples = (0..SAMPLE_RATE)
            .flat_map(|frame| {
                let sample = (TAU * FREQUENCY * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample * 0.25]
            })
            .collect::<Vec<_>>();

        graph.process_interleaved_stereo(&mut samples);

        let settled = &samples[SAMPLE_RATE / 5 * 2..];
        let left_rms = channel_rms(settled, 0);
        let right_rms = channel_rms(settled, 1);
        let expected_gain = 10.0_f32.powf(6.0 / 20.0);
        assert!((left_rms / (1.0 / 2.0_f32.sqrt()) - expected_gain).abs() < 0.01);
        assert!((left_rms / right_rms - 4.0).abs() < 0.01);
    }

    #[test]
    fn multiple_peaking_filters_are_applied_as_an_ordered_cascade() {
        const SAMPLE_RATE: usize = 48_000;
        const FREQUENCY: f32 = 1_000.0;
        let filters = [3.0, 6.0].map(|gain| {
            PeakingFilter::new(
                FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
                GainDb::new(gain).unwrap(),
                QualityFactor::new(1.0).unwrap(),
            )
        });
        let equalizer = Equalizer::with_peaking_filters(filters.to_vec());
        let mut graph = PreparedGraph::prepare(&equalizer, SAMPLE_RATE as f64).unwrap();
        let mut samples = (0..SAMPLE_RATE)
            .flat_map(|frame| {
                let sample = (TAU * FREQUENCY * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        graph.process_interleaved_stereo(&mut samples);

        let settled = &samples[SAMPLE_RATE / 5 * 2..];
        let measured_gain = channel_rms(settled, 0) / (1.0 / 2.0_f32.sqrt());
        let expected_gain = 10.0_f32.powf(9.0 / 20.0);
        assert!((measured_gain - expected_gain).abs() < 0.02);
    }

    #[test]
    fn preparation_rejects_a_filter_at_or_above_nyquist() {
        let filter = PeakingFilter::new(
            FrequencyHz::new(24_000.0).unwrap(),
            GainDb::new(3.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_peaking_filter(filter), 48_000.0).is_err());
    }

    #[test]
    fn parameter_types_reject_non_finite_and_non_positive_values() {
        assert!(FrequencyHz::new(0.0).is_err());
        assert!(FrequencyHz::new(f64::NAN).is_err());
        assert!(GainDb::new(f64::INFINITY).is_err());
        assert!(QualityFactor::new(-1.0).is_err());
    }

    #[test]
    fn preparation_rejects_finite_parameters_that_overflow_coefficients() {
        let filter = PeakingFilter::new(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(f64::MAX).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_peaking_filter(filter), 48_000.0).is_err());
    }

    #[test]
    fn preparation_rejects_coefficients_destabilized_by_f32_quantization() {
        let filter = PeakingFilter::new(
            FrequencyHz::new(1.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_peaking_filter(filter), 48_000.0).is_err());
    }

    fn channel_rms(samples: &[f32], channel: usize) -> f32 {
        let values = samples.iter().skip(channel).step_by(2);
        let count = values.clone().count();
        (values.map(|sample| sample * sample).sum::<f32>() / count as f32).sqrt()
    }
}
