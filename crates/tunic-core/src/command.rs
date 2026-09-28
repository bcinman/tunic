use crate::{Chain, DeviceId, PresetId, ProfileId, ProfileName, ProfileRevision};

/// An intended change to Tunic's product state.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    CreateProfile {
        id: ProfileId,
        name: ProfileName,
        source: ProfileSource,
    },
    RenameProfile {
        profile: ProfileId,
        name: ProfileName,
    },
    DeleteProfile(ProfileId),
    SelectProfile {
        device: DeviceId,
        profile: ProfileId,
    },
    ClearProfile(DeviceId),
    UpdateProfile {
        profile: ProfileId,
        chain: Chain,
        expected_revision: ProfileRevision,
    },
}

/// Initial contents for a newly created profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileSource {
    Flat,
    Preset(PresetId),
    Copy(ProfileId),
}
