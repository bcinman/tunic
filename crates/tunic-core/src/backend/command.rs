//! Product intents accepted by the backend.
//!
//! Commands describe requested state changes without performing persistence,
//! device I/O, or real-time processor control.

use super::{DeviceId, PresetId, ProfileId, ProfileName, ProfileRevision};
use crate::Chain;

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
