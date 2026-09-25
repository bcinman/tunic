//! Portable equalizer definitions and real-time processing.

mod equalizer;
mod exchange;
mod graph;

pub use equalizer::{
    Equalizer, EqualizerError, Filter, FilterKind, FrequencyHz, GainDb, QualityFactor,
};
pub use exchange::{GraphProcessor, GraphPublisher};
pub use graph::PreparedGraph;
