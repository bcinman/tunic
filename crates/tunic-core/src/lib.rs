//! Portable product state and real-time audio processing for Tunic.

mod backend;
mod chain;
mod command;
mod processor;
mod profile;
mod state;
mod store;

pub use backend::{Backend, BackendError};
pub use chain::{
    Chain, Filter, FilterKind, FrequencyHz, FrequencyHzError, GainDb, GainDbError, QualityFactor,
    QualityFactorError,
};
pub use command::{Command, ProfileSource};
pub use processor::{
    AudioFormat, ChannelLevels, Processor, ProcessorError, SampleRateHz, SampleRateHzError,
    Spectrum, StereoLevels, Telemetry, TelemetryFrame,
};
pub use profile::{
    Preset, PresetId, PresetIdError, Profile, ProfileId, ProfileIdError, ProfileName,
    ProfileNameError, ProfileRevision,
};
pub use state::{DeviceId, DeviceIdError, DeviceProfileSelection, State};
pub use store::{DurableState, Store, StoreError, StoredProfile};
