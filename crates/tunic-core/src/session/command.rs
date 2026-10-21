//! Product intents accepted by the session.
//!
//! Commands describe requested state changes without performing persistence,
//! device I/O, or real-time processor control.

use super::{DeviceId, FilterControlName, PresetId, ProfileId, ProfileName};
use crate::{FilterId, FrequencyHz, GainDb};

#[derive(Clone, Debug, PartialEq)]
pub enum ProfileSource {
    Preset(PresetId),
    Flat,
}

/// An intended change to Tunic's product state.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetProcessingEnabled(bool),
    CreateProfile {
        device: DeviceId,
        name: ProfileName,
        source: ProfileSource,
    },
    SelectProfile {
        device: DeviceId,
        profile: ProfileId,
    },
    EditProfile {
        profile: ProfileId,
        edit: ProfileEdit,
    },
    RefreshAudio,
}

/// An edit addressed to the profile that originated the interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum ProfileEdit {
    RenameControl {
        filter: FilterId,
        name: FilterControlName,
    },
    EditFilter {
        filter: FilterId,
        frequency: FrequencyHz,
        gain: GainDb,
    },
    SetControlGain {
        filter: FilterId,
        gain: GainDb,
    },
    PreviewControlGain {
        filter: FilterId,
        gain: GainDb,
    },
    FinishControlGain,
    CancelControlGain,
    SaveDraft,
    ResetDraft,
}
