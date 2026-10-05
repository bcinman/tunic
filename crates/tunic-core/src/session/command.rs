//! Product intents accepted by the session.
//!
//! Commands describe requested state changes without performing persistence,
//! device I/O, or real-time processor control.

use super::PresetId;
use crate::{FilterId, FrequencyHz, GainDb};

/// An intended change to Tunic's product state.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    UseFlat,
    UsePreset(PresetId),
    EditFilter {
        filter: FilterId,
        frequency: FrequencyHz,
        gain: GainDb,
    },
    SetControlGain {
        filter: FilterId,
        gain: GainDb,
    },
    SaveDraft,
    ResetDraft,
    ClearSelection,
    RefreshAudio,
}
