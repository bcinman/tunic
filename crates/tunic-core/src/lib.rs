//! Portable product state and real-time audio processing for Tunic.

mod backend;
mod chain;
mod processor;

pub use backend::{
    AdjustableParameter, Adjustment, Attribution, Backend, BackendError, Command, DeviceId,
    DeviceIdError, DeviceProfileSelection, MemoryStore, Preset, PresetCatalog, PresetId,
    PresetIdError, PresetOrigin, PresetQuery, PresetRevision, PresetRevisionError, PresetSummary,
    Profile, ProfileId, ProfileIdError, ProfileName, ProfileNameError, ProfileRevision,
    ProfileSource, State, Store, StoreError,
};
pub use chain::{
    Chain, Equalizer, Filter, FilterId, FilterIdError, FilterKind, FrequencyHz, FrequencyHzError,
    GainDb, GainDbError, QualityFactor, QualityFactorError,
};
pub use processor::{
    AudioFormat, ChannelLevels, Controller, Processor, ProcessorError, SPECTRUM_MAX_FREQUENCY_HZ,
    SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT, SampleRateHz, SampleRateHzError, Spectrum,
    StereoLevels, Telemetry, TelemetryFrame, spectrum_frequency_hz,
};
