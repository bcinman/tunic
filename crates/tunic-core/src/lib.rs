//! Portable product state and real-time audio processing for Tunic.

mod analyzer;
mod backend;
mod chain;
mod command;
mod dsp;
mod exchange;
mod processor;
mod profile;
mod state;
mod store;
mod telemetry;

pub use backend::{Backend, BackendError};
pub use chain::{
    Chain, Equalizer, Filter, FilterKind, FrequencyHz, FrequencyHzError, GainDb, GainDbError,
    QualityFactor, QualityFactorError,
};
pub use command::{Command, ProfileSource};
pub use processor::{
    AudioFormat, Controller, Processor, ProcessorError, SampleRateHz, SampleRateHzError,
};
pub use profile::{
    Preset, PresetId, PresetIdError, Profile, ProfileId, ProfileIdError, ProfileName,
    ProfileNameError, ProfileRevision,
};
pub use state::{DeviceId, DeviceIdError, DeviceProfileSelection, State};
pub use store::{MemoryStore, Store, StoreError};
pub use telemetry::{
    ChannelLevels, SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT,
    Spectrum, StereoLevels, Telemetry, TelemetryFrame, spectrum_frequency_hz,
};
