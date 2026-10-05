//! Portable product state and real-time audio processing for Tunic.

mod chain;
mod processor;
mod session;

pub use chain::{
    Chain, Equalizer, Filter, FilterId, FilterIdError, FilterKind, FilterParameters, FrequencyHz,
    FrequencyHzError, GainDb, GainDbError, QualityFactor, QualityFactorError,
};
pub use processor::{
    AudioFormat, ChannelLevels, Controller, FrequencyResponse, Processor, ProcessorError,
    SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT, SampleRateHz,
    SampleRateHzError, Spectrum, StereoLevels, Telemetry, TelemetryFrame, spectrum_frequency_hz,
};
pub use session::{
    Attribution, ChangeHandler, Command, Connection, DeviceId, DeviceIdError,
    DeviceProfileSelection, FilterControl, FilterControlName, FilterControlNameError,
    MemoryPersistence, Persistence, PersistenceError, Platform, Preset, PresetCatalog, PresetId,
    PresetIdError, PresetOrigin, PresetQuery, PresetRevision, PresetRevisionError, PresetSummary,
    Profile, ProfileError, ProfileId, ProfileIdError, ProfileName, ProfileNameError, Session,
    SessionError, State,
};
