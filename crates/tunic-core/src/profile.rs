use crate::{Chain, DeviceId};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileId(String);

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct PresetId(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileName(String);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProfileRevision(pub u64);

/// A saved profile and its optional in-memory preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub device: DeviceId,
    pub name: ProfileName,
    pub chain: Chain,
    pub revision: ProfileRevision,
    pub preview: Option<Chain>,
}

/// A built-in starting point for a profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: PresetId,
    pub brand: String,
    pub model: String,
    pub chain: Chain,
}

#[cfg(test)]
mod tests {
    use super::{PresetId, ProfileId, ProfileName};

    #[test]
    fn identifiers_are_not_empty() {
        assert!(ProfileId::try_new("").is_err());
        assert!(PresetId::try_new("").is_err());
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
