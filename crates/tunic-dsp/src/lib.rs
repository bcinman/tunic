//! Portable equalizer definitions and real-time processing.

mod equalizer;
mod graph;

pub use equalizer::{
    Equalizer, EqualizerError, Filter, FilterKind, FrequencyHz, GainDb, QualityFactor,
};
pub use graph::PreparedGraph;
