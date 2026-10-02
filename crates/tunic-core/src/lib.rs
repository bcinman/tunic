//! Portable product state and real-time audio processing for Tunic.

mod backend;
mod chain;
mod processor;

pub use backend::{
    Attribution, Backend, BackendError, Command, DeviceId, DeviceIdError, DeviceProfileSelection,
    FilterControl, FilterControlName, FilterControlNameError, MemoryStore, Preset, PresetCatalog,
    PresetId, PresetIdError, PresetOrigin, PresetQuery, PresetRevision, PresetRevisionError,
    PresetSummary, Profile, ProfileError, ProfileId, ProfileIdError, ProfileName, ProfileNameError,
    ProfileRevision, ProfileSource, State, Store, StoreError,
};
pub use chain::{
    Chain, Equalizer, Filter, FilterId, FilterIdError, FilterKind, FilterParameters, FrequencyHz,
    FrequencyHzError, GainDb, GainDbError, QualityFactor, QualityFactorError,
};
pub use processor::{
    AudioFormat, ChannelLevels, Controller, FrequencyResponse, Processor, ProcessorError,
    SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT, SampleRateHz,
    SampleRateHzError, Spectrum, StereoLevels, Telemetry, TelemetryFrame, spectrum_frequency_hz,
};
