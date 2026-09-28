use crate::{Chain, DeviceId, EditRevision, PresetId, ProfileId};

/// An intended change to Tunic's product state.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetBypassed(bool),
    CreateProfile {
        device: DeviceId,
        name: String,
        source: ProfileSource,
    },
    RenameProfile {
        profile: ProfileId,
        name: String,
    },
    DeleteProfile(ProfileId),
    SelectProfile {
        device: DeviceId,
        profile: ProfileId,
    },
    PreviewChain {
        profile: ProfileId,
        chain: Chain,
        expected_revision: EditRevision,
    },
    SaveChain {
        profile: ProfileId,
        expected_revision: EditRevision,
    },
    DiscardChain {
        profile: ProfileId,
        expected_revision: EditRevision,
    },
}

/// Initial contents for a newly created profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileSource {
    Flat,
    Preset(PresetId),
    Copy(ProfileId),
}
