use crate::{Chain, DeviceId, PresetId, ProfileId, ProfileName};

/// An intended change to Tunic's product state.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetBypassed(bool),
    CreateProfile {
        device: DeviceId,
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
    PreviewChain {
        profile: ProfileId,
        chain: Chain,
    },
    SaveChain(ProfileId),
    DiscardChain(ProfileId),
}

/// Initial contents for a newly created profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileSource {
    Flat,
    Preset(PresetId),
    Copy(ProfileId),
}
