//! Backend-owned processing profiles and built-in profile presets.
//!
//! Profiles own stable identity, display name, chain, and revision. They are
//! global values rather than being owned by any output device.

use crate::{Chain, FilterId};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileId(String);

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom, AsRef)
)]
pub struct PresetId(String);

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom, AsRef)
)]
pub struct PresetRevision(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileName(String);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProfileRevision(pub u64);

impl ProfileRevision {
    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// A saved reusable processing profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: ProfileName,
    pub chain: Chain,
    pub revision: ProfileRevision,
    pub origin: Option<PresetOrigin>,
    pub adjustments: Vec<Adjustment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetSummary {
    pub id: PresetId,
    pub revision: PresetRevision,
    pub brand: String,
    pub model: String,
    pub variant: Option<String>,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribution {
    pub provider: String,
    pub measurement_source: Option<String>,
    pub source_url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjustableParameter {
    GainDb,
    FrequencyHz,
    QualityFactor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Adjustment {
    pub label: String,
    pub filter: FilterId,
    pub parameter: AdjustableParameter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetOrigin {
    pub id: PresetId,
    pub revision: PresetRevision,
    pub attribution: Attribution,
}

/// A catalog-provided starting point for a profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub summary: PresetSummary,
    pub attribution: Attribution,
    pub chain: Chain,
    pub adjustments: Vec<Adjustment>,
}

#[cfg(test)]
mod tests {
    use super::{PresetId, PresetRevision, ProfileId, ProfileName};

    #[test]
    fn identifiers_are_not_empty() {
        assert!(ProfileId::try_new("").is_err());
        assert!(PresetId::try_new("").is_err());
        assert!(PresetRevision::try_new("").is_err());
        assert!(ProfileId::try_new("studio").is_ok());
    }

    #[test]
    fn profile_names_are_trimmed_and_not_blank() {
        assert!(ProfileName::try_new("   ").is_err());
        assert_eq!(
            ProfileName::try_new("  Studio  ").unwrap().into_inner(),
            "Studio"
        );
    }
}
