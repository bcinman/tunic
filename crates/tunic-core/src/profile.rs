use crate::{Chain, DeviceId};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProfileId(pub String);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PresetId(pub String);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProfileRevision(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct EditRevision(pub u64);

/// A saved profile and its optional in-memory edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub device: DeviceId,
    pub name: String,
    pub chain: Chain,
    pub revision: ProfileRevision,
    pub edit: Option<ProfileEdit>,
}

/// An unsaved chain preview.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileEdit {
    pub chain: Chain,
    pub revision: EditRevision,
}

/// A built-in starting point for a profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: PresetId,
    pub brand: String,
    pub model: String,
    pub chain: Chain,
}
