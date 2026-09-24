use std::fmt;

use serde::{Deserialize, Serialize};

const DOCUMENT_VERSION: u32 = 1;

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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

impl fmt::Display for FilterKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Peaking => "peaking",
            Self::LowShelf => "low-shelf",
            Self::HighShelf => "high-shelf",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    kind: FilterKind,
    frequency: FrequencyHz,
    gain: GainDb,
    quality_factor: QualityFactor,
}

impl Filter {
    #[must_use]
    pub fn new(
        kind: FilterKind,
        frequency: FrequencyHz,
        gain: GainDb,
        quality_factor: QualityFactor,
    ) -> Self {
        Self {
            kind,
            frequency,
            gain,
            quality_factor,
        }
    }

    #[must_use]
    pub fn peaking(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::new(FilterKind::Peaking, frequency, gain, quality_factor)
    }

    #[must_use]
    pub fn low_shelf(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::new(FilterKind::LowShelf, frequency, gain, quality_factor)
    }

    #[must_use]
    pub fn high_shelf(frequency: FrequencyHz, gain: GainDb, quality_factor: QualityFactor) -> Self {
        Self::new(FilterKind::HighShelf, frequency, gain, quality_factor)
    }

    #[must_use]
    pub fn kind(self) -> FilterKind {
        self.kind
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
        if document.version != DOCUMENT_VERSION {
            return Err(EqualizerError::new(format!(
                "unsupported equalizer document version {}",
                document.version
            )));
        }
        let filters = document
            .filters
            .into_iter()
            .map(Filter::try_from)
            .collect::<Result<_, _>>()?;
        Ok(Self { filters })
    }

    pub fn to_canonical_json(&self) -> Result<String, EqualizerError> {
        let document = EqualizerDocument {
            version: DOCUMENT_VERSION,
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
#[serde(deny_unknown_fields)]
struct FilterDocument {
    #[serde(rename = "type")]
    kind: FilterKind,
    frequency_hz: f64,
    gain_db: f64,
    q: f64,
}

impl From<Filter> for FilterDocument {
    fn from(filter: Filter) -> Self {
        Self {
            kind: filter.kind(),
            frequency_hz: filter.frequency().get(),
            gain_db: filter.gain().get(),
            q: filter.quality_factor().get(),
        }
    }
}

impl TryFrom<FilterDocument> for Filter {
    type Error = EqualizerError;

    fn try_from(filter: FilterDocument) -> Result<Self, Self::Error> {
        Ok(Self::new(
            filter.kind,
            FrequencyHz::new(filter.frequency_hz)?,
            GainDb::new(filter.gain_db)?,
            QualityFactor::new(filter.q)?,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EqualizerError(String);

impl EqualizerError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EqualizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for EqualizerError {}

#[cfg(test)]
mod tests {
    use super::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};

    #[test]
    fn parameter_types_reject_non_finite_and_non_positive_values() {
        assert!(FrequencyHz::new(0.0).is_err());
        assert!(FrequencyHz::new(f64::NAN).is_err());
        assert!(GainDb::new(f64::INFINITY).is_err());
        assert!(QualityFactor::new(-1.0).is_err());
    }

    #[test]
    fn json_round_trips_all_filter_types_in_order() {
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
    fn json_rejects_unknown_versions_fields_and_invalid_parameters() {
        let unknown_version = r#"{"version":2,"filters":[]}"#;
        let unknown_field = r#"{"version":1,"filters":[{"type":"peaking","frequency_hz":1000.0,"gain_db":3.0,"q":1.0,"enabled":true}]}"#;
        let invalid_q = r#"{"version":1,"filters":[{"type":"peaking","frequency_hz":1000.0,"gain_db":3.0,"q":0.0}]}"#;

        assert!(
            Equalizer::parse_json(unknown_version)
                .unwrap_err()
                .to_string()
                .contains("unsupported equalizer document version 2")
        );
        assert!(Equalizer::parse_json(unknown_field).is_err());
        assert!(Equalizer::parse_json(invalid_q).is_err());
    }
}
