//! Portable real-time equalization and post-DSP analysis.

mod analyzer;
mod equalizer;
mod exchange;
mod graph;

pub use analyzer::{
    AnalysisFrame, Analyzer, ChannelLevels, SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ,
    SPECTRUM_POINT_COUNT, Spectrum, StereoLevels, spectrum_frequency_hz,
};
pub use equalizer::{
    Equalizer, EqualizerError, Filter, FilterKind, FrequencyHz, GainDb, QualityFactor,
};
pub use exchange::{GraphProcessor, GraphPublisher};
pub use graph::PreparedGraph;
