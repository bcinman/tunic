//! Portable audio-processing definitions and algorithms.

use std::f64::consts::TAU;
use std::fmt;

use serde::{Deserialize, Serialize};

const EQUALIZER_DOCUMENT_VERSION: u32 = 1;

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
pub enum Filter {
    Peaking {
        frequency: FrequencyHz,
        gain: GainDb,
        quality_factor: QualityFactor,
    },
    LowShelf {
        frequency: FrequencyHz,
        gain: GainDb,
        quality_factor: QualityFactor,
    },
    HighShelf {
        frequency: FrequencyHz,
        gain: GainDb,
        quality_factor: QualityFactor,
    },
}

impl Filter {
    #[must_use]
    pub fn peaking(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::Peaking {
            frequency,
            gain,
            quality_factor,
        }
    }

    #[must_use]
    pub fn low_shelf(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::LowShelf {
            frequency,
            gain,
            quality_factor,
        }
    }

    #[must_use]
    pub fn high_shelf(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::HighShelf {
            frequency,
            gain,
            quality_factor,
        }
    }

    #[must_use]
    pub fn frequency(self) -> FrequencyHz {
        match self {
            Self::Peaking { frequency, .. }
            | Self::LowShelf { frequency, .. }
            | Self::HighShelf { frequency, .. } => frequency,
        }
    }

    #[must_use]
    pub fn gain(self) -> GainDb {
        match self {
            Self::Peaking { gain, .. }
            | Self::LowShelf { gain, .. }
            | Self::HighShelf { gain, .. } => gain,
        }
    }

    #[must_use]
    pub fn quality_factor(self) -> QualityFactor {
        match self {
            Self::Peaking { quality_factor, .. }
            | Self::LowShelf { quality_factor, .. }
            | Self::HighShelf { quality_factor, .. } => quality_factor,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equalizer {
    filters: Vec<Filter>,
}

impl Equalizer {
    #[must_use]
    pub fn identity() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_filter(filter: Filter) -> Self {
        Self {
            filters: vec![filter],
        }
    }

    #[must_use]
    pub fn with_filters(filters: Vec<Filter>) -> Self {
        Self { filters }
    }

    #[must_use]
    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    pub fn parse_json(input: &str) -> Result<Self, EqualizerError> {
        let document: EqualizerDocument = serde_json::from_str(input)
            .map_err(|error| EqualizerError::new(format!("invalid equalizer JSON: {error}")))?;
        if document.version != EQUALIZER_DOCUMENT_VERSION {
            return Err(EqualizerError::new(format!(
                "unsupported equalizer document version {}",
                document.version
            )));
        }
        let filters = document
            .filters
            .into_iter()
            .map(Filter::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::with_filters(filters))
    }

    pub fn to_canonical_json(&self) -> Result<String, EqualizerError> {
        let document = EqualizerDocument {
            version: EQUALIZER_DOCUMENT_VERSION,
            filters: self
                .filters
                .iter()
                .copied()
                .map(FilterDocument::from)
                .collect(),
        };
        serde_json::to_string(&document)
            .map_err(|error| EqualizerError::new(format!("serialize equalizer: {error}")))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EqualizerDocument {
    version: u32,
    filters: Vec<FilterDocument>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum FilterDocument {
    Peaking {
        frequency_hz: f64,
        gain_db: f64,
        q: f64,
    },
    LowShelf {
        frequency_hz: f64,
        gain_db: f64,
        q: f64,
    },
    HighShelf {
        frequency_hz: f64,
        gain_db: f64,
        q: f64,
    },
}

impl From<Filter> for FilterDocument {
    fn from(filter: Filter) -> Self {
        let fields = (
            filter.frequency().get(),
            filter.gain().get(),
            filter.quality_factor().get(),
        );
        match filter {
            Filter::Peaking { .. } => Self::Peaking {
                frequency_hz: fields.0,
                gain_db: fields.1,
                q: fields.2,
            },
            Filter::LowShelf { .. } => Self::LowShelf {
                frequency_hz: fields.0,
                gain_db: fields.1,
                q: fields.2,
            },
            Filter::HighShelf { .. } => Self::HighShelf {
                frequency_hz: fields.0,
                gain_db: fields.1,
                q: fields.2,
            },
        }
    }
}

impl TryFrom<FilterDocument> for Filter {
    type Error = EqualizerError;

    fn try_from(filter: FilterDocument) -> Result<Self, Self::Error> {
        let (kind, frequency_hz, gain_db, q) = match filter {
            FilterDocument::Peaking {
                frequency_hz,
                gain_db,
                q,
            } => (Self::peaking as fn(_, _, _) -> _, frequency_hz, gain_db, q),
            FilterDocument::LowShelf {
                frequency_hz,
                gain_db,
                q,
            } => (
                Self::low_shelf as fn(_, _, _) -> _,
                frequency_hz,
                gain_db,
                q,
            ),
            FilterDocument::HighShelf {
                frequency_hz,
                gain_db,
                q,
            } => (
                Self::high_shelf as fn(_, _, _) -> _,
                frequency_hz,
                gain_db,
                q,
            ),
        };
        Ok(kind(
            FrequencyHz::new(frequency_hz)?,
            GainDb::new(gain_db)?,
            QualityFactor::new(q)?,
        ))
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
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { filters })
    }

    /// Process stereo audio in place without allocating.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len());
        for filter in &mut self.filters {
            for (left, right) in left.iter_mut().zip(right.iter_mut()) {
                *left = filter.left.process(*left);
                *right = filter.right.process(*right);
            }
        }
    }

    /// Process interleaved stereo frames in place without allocating.
    pub fn process_interleaved_stereo(&mut self, samples: &mut [f32]) {
        debug_assert_eq!(samples.len() % 2, 0);
        for filter in &mut self.filters {
            for frame in samples.as_chunks_mut::<2>().0 {
                frame[0] = filter.left.process(frame[0]);
                frame[1] = filter.right.process(frame[1]);
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
        let coefficients = match filter {
            Filter::Peaking { .. } => Coefficients::peaking(filter, sample_rate_hz),
            Filter::LowShelf { .. } => Coefficients::low_shelf(filter, sample_rate_hz),
            Filter::HighShelf { .. } => Coefficients::high_shelf(filter, sample_rate_hz),
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

    use super::{Equalizer, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor};

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
        let filter = Filter::peaking(
            FrequencyHz::new(f64::from(FREQUENCY)).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let mut graph =
            PreparedGraph::prepare(&Equalizer::with_filter(filter), SAMPLE_RATE as f64).unwrap();
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
    fn mixed_filters_are_applied_as_an_ordered_cascade() {
        const SAMPLE_RATE: usize = 48_000;
        const FREQUENCY: f32 = 1_000.0;
        let filters = vec![
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
        ];
        let equalizer = Equalizer::with_filters(filters);
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
    fn low_shelf_boosts_and_cuts_lows_while_preserving_highs() {
        let boosted = Filter::low_shelf(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let cut = Filter::low_shelf(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(-6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!((measured_gain(boosted, 100.0) - db_gain(6.0)).abs() < 0.02);
        assert!((measured_gain(cut, 100.0) - db_gain(-6.0)).abs() < 0.02);
        assert!((measured_gain(boosted, 10_000.0) - 1.0).abs() < 0.01);
        assert!((measured_gain(cut, 10_000.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn high_shelf_boosts_and_cuts_highs_while_preserving_lows() {
        let boosted = Filter::high_shelf(
            FrequencyHz::new(2_000.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );
        let cut = Filter::high_shelf(
            FrequencyHz::new(2_000.0).unwrap(),
            GainDb::new(-6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!((measured_gain(boosted, 12_000.0) - db_gain(6.0)).abs() < 0.02);
        assert!((measured_gain(cut, 12_000.0) - db_gain(-6.0)).abs() < 0.02);
        assert!((measured_gain(boosted, 100.0) - 1.0).abs() < 0.01);
        assert!((measured_gain(cut, 100.0) - 1.0).abs() < 0.01);
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

        // Reference gains are independent f64 transfer-function evaluations;
        // processing under test is the time-domain f32 implementation.
        for (filter, probe_hz, expected_gain) in cases {
            assert!((measured_gain(filter, probe_hz) - expected_gain).abs() < 0.002);
        }
    }

    #[test]
    fn preparation_rejects_a_filter_at_or_above_nyquist() {
        let filter = Filter::peaking(
            FrequencyHz::new(24_000.0).unwrap(),
            GainDb::new(3.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_filter(filter), 48_000.0).is_err());
    }

    #[test]
    fn parameter_types_reject_non_finite_and_non_positive_values() {
        assert!(FrequencyHz::new(0.0).is_err());
        assert!(FrequencyHz::new(f64::NAN).is_err());
        assert!(GainDb::new(f64::INFINITY).is_err());
        assert!(QualityFactor::new(-1.0).is_err());
    }

    #[test]
    fn equalizer_json_round_trips_all_filter_types_in_order() {
        let equalizer = Equalizer::with_filters(vec![
            Filter::low_shelf(
                FrequencyHz::new(80.0).unwrap(),
                GainDb::new(2.5).unwrap(),
                QualityFactor::new(0.7).unwrap(),
            ),
            Filter::peaking(
                FrequencyHz::new(1_234.0).unwrap(),
                GainDb::new(-4.5).unwrap(),
                QualityFactor::new(1.25).unwrap(),
            ),
            Filter::high_shelf(
                FrequencyHz::new(9_000.0).unwrap(),
                GainDb::new(1.5).unwrap(),
                QualityFactor::new(0.9).unwrap(),
            ),
        ]);

        let json = equalizer.to_canonical_json().unwrap();

        assert_eq!(Equalizer::parse_json(&json).unwrap(), equalizer);
        assert_eq!(
            json,
            r#"{"version":1,"filters":[{"type":"low-shelf","frequency_hz":80.0,"gain_db":2.5,"q":0.7},{"type":"peaking","frequency_hz":1234.0,"gain_db":-4.5,"q":1.25},{"type":"high-shelf","frequency_hz":9000.0,"gain_db":1.5,"q":0.9}]}"#
        );
    }

    #[test]
    fn equalizer_json_rejects_unknown_versions_and_invalid_parameters() {
        let unknown_version = r#"{"version":2,"filters":[]}"#;
        let invalid_q = r#"{"version":1,"filters":[{"type":"peaking","frequency_hz":1000.0,"gain_db":3.0,"q":0.0}]}"#;

        assert!(
            Equalizer::parse_json(unknown_version)
                .unwrap_err()
                .to_string()
                .contains("unsupported equalizer document version 2")
        );
        assert!(Equalizer::parse_json(invalid_q).is_err());
    }

    #[test]
    fn preparation_rejects_finite_parameters_that_overflow_coefficients() {
        let filter = Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(f64::MAX).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        );

        assert!(PreparedGraph::prepare(&Equalizer::with_filter(filter), 48_000.0).is_err());
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

    fn measured_gain(filter: Filter, frequency_hz: f32) -> f32 {
        const SAMPLE_RATE: usize = 48_000;
        let mut graph =
            PreparedGraph::prepare(&Equalizer::with_filter(filter), SAMPLE_RATE as f64).unwrap();
        let mut samples = (0..SAMPLE_RATE)
            .flat_map(|frame| {
                let sample = (TAU * frequency_hz * frame as f32 / SAMPLE_RATE as f32).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();
        graph.process_interleaved_stereo(&mut samples);
        channel_rms(&samples[SAMPLE_RATE..], 0) / (1.0 / 2.0_f32.sqrt())
    }

    fn db_gain(gain_db: f32) -> f32 {
        10.0_f32.powf(gain_db / 20.0)
    }

    fn channel_rms(samples: &[f32], channel: usize) -> f32 {
        let values = samples.iter().skip(channel).step_by(2);
        let count = values.clone().count();
        (values.map(|sample| sample * sample).sum::<f32>() / count as f32).sqrt()
    }
}
