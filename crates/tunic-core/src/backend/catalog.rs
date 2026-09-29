//! Preset catalog boundary.

use super::{Preset, PresetId, PresetSummary};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PresetQuery {
    pub brand: Option<String>,
    pub model: Option<String>,
}

/// An immutable catalog of validated presets. Invalid catalog data is a
/// programming defect, not a recoverable lookup failure.
pub trait PresetCatalog {
    fn brands(&self) -> Vec<String>;
    fn models(&self, brand: &str) -> Vec<String>;
    fn list(&self, query: &PresetQuery) -> Vec<PresetSummary>;
    /// Returns `None` when the ID is not in the catalog.
    fn get(&self, id: &PresetId) -> Option<Preset>;
}
