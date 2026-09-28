//! Portable product state and real-time audio processing for Tunic.

mod backend;
mod chain;
mod command;
mod processor;
mod profile;
mod state;
mod store;

pub use backend::{Backend, BackendError};
pub use chain::{Chain, Filter, FilterKind, FrequencyHz, GainDb, QualityFactor};
pub use command::{Command, ProfileSource};
pub use processor::{
    AudioFormat, ChannelLevels, Processor, ProcessorError, Spectrum, StereoLevels, Telemetry,
    TelemetryFrame,
};
pub use profile::{
    EditRevision, Preset, PresetId, Profile, ProfileEdit, ProfileId, ProfileRevision,
};
pub use state::{DeviceId, DeviceProfileSelection, State};
pub use store::{DurableState, Store, StoreError, StoredProfile};
