//! Typed catalog access for native applications; no JSON crosses this boundary.

use boltffi::*;
use tunic_core::{self as core, PresetCatalog as _};
use tunic_presets::BundledCatalog;

#[data]
pub struct PresetQuery {
    pub brand: Option<String>,
    pub model: Option<String>,
}

#[data]
pub struct PresetSummary {
    pub id: String,
    pub revision: String,
    pub brand: String,
    pub model: String,
    pub variant: Option<String>,
    pub target: String,
}

#[data]
pub struct Attribution {
    pub provider: String,
    pub measurement_source: Option<String>,
    pub source_url: String,
}

#[data]
pub enum AdjustableParameter {
    GainDb,
    FrequencyHz,
    QualityFactor,
}

#[data]
pub struct Adjustment {
    pub label: String,
    pub filter: u32,
    pub parameter: AdjustableParameter,
}

#[data]
pub struct Preset {
    pub summary: PresetSummary,
    pub attribution: Attribution,
    pub chain: crate::Chain,
    pub adjustments: Vec<Adjustment>,
}

#[error]
#[derive(Debug)]
pub enum CatalogError {
    InvalidPresetId,
}

/// Immutable, offline catalog. Construction and browsing do not decode payloads.
#[derive(Default)]
pub struct PresetCatalog;

#[export]
impl PresetCatalog {
    pub fn new() -> Self {
        Self
    }

    pub fn brands(&self) -> Vec<String> {
        BundledCatalog.brands()
    }

    pub fn models(&self, brand: String) -> Vec<String> {
        BundledCatalog.models(&brand)
    }

    /// Exact, case-sensitive brand/model filters. Omitted filters match all.
    pub fn list_presets(&self, query: PresetQuery) -> Vec<PresetSummary> {
        BundledCatalog
            .list(&core::PresetQuery {
                brand: query.brand,
                model: query.model,
            })
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn preset(&self, id: String) -> Result<Option<Preset>, CatalogError> {
        let id = core::PresetId::try_new(id).map_err(|_| CatalogError::InvalidPresetId)?;
        Ok(BundledCatalog.get(&id).map(Into::into))
    }
}

impl From<core::PresetSummary> for PresetSummary {
    fn from(summary: core::PresetSummary) -> Self {
        Self {
            id: summary.id.into_inner(),
            revision: summary.revision.into_inner(),
            brand: summary.brand,
            model: summary.model,
            variant: summary.variant,
            target: summary.target,
        }
    }
}

impl From<core::Preset> for Preset {
    fn from(preset: core::Preset) -> Self {
        Self {
            summary: preset.summary.into(),
            attribution: preset.attribution.into(),
            chain: preset.chain.into(),
            adjustments: preset.adjustments.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<core::Attribution> for Attribution {
    fn from(attribution: core::Attribution) -> Self {
        Self {
            provider: attribution.provider,
            measurement_source: attribution.measurement_source,
            source_url: attribution.source_url,
        }
    }
}

impl From<core::Adjustment> for Adjustment {
    fn from(adjustment: core::Adjustment) -> Self {
        Self {
            label: adjustment.label,
            filter: adjustment.filter.into_inner(),
            parameter: match adjustment.parameter {
                core::AdjustableParameter::GainDb => AdjustableParameter::GainDb,
                core::AdjustableParameter::FrequencyHz => AdjustableParameter::FrequencyHz,
                core::AdjustableParameter::QualityFactor => AdjustableParameter::QualityFactor,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_returns_typed_payloads_with_stable_filter_ids() {
        let catalog = PresetCatalog::new();
        assert_eq!(catalog.models("Sony".into()), ["MDR-7506"]);
        let summaries = catalog.list_presets(PresetQuery {
            brand: Some("Sony".into()),
            model: Some("MDR-7506".into()),
        });
        assert_eq!(summaries.len(), 1);
        let preset = catalog.preset(summaries[0].id.clone()).unwrap().unwrap();
        assert_eq!(preset.attribution.provider, "oratory1990");
        assert_eq!(preset.chain.preamp_gain_db, -5.0);
        assert_eq!(preset.adjustments[0].label, "Bass");
        assert_eq!(preset.adjustments[0].filter, 2);
        assert!(matches!(
            preset.adjustments[0].parameter,
            AdjustableParameter::GainDb
        ));
        assert_eq!(preset.chain.filters[1].id, 2);
        assert_eq!(preset.chain.filters[1].frequency_hz, 105.0);
        let chain: core::Chain = preset.chain.try_into().unwrap();
        let round_trip: core::Chain = crate::Chain::from(chain.clone()).try_into().unwrap();
        assert_eq!(round_trip, chain);
        assert!(catalog.preset("missing".into()).unwrap().is_none());
        assert!(matches!(
            catalog.preset(String::new()),
            Err(CatalogError::InvalidPresetId)
        ));
    }

    #[test]
    fn chain_conversion_rejects_invalid_and_duplicate_ids() {
        let make_chain = || {
            PresetCatalog::new()
                .preset("oratory1990/sony/mdr-7506/harman".into())
                .unwrap()
                .unwrap()
                .chain
        };
        let mut chain = make_chain();
        chain.filters[0].id = 0;
        assert!(matches!(
            core::Chain::try_from(chain),
            Err(crate::ProcessorError::InvalidFilterId)
        ));
        let mut chain = make_chain();
        chain.filters[0].id = chain.filters[1].id;
        assert!(matches!(
            core::Chain::try_from(chain),
            Err(crate::ProcessorError::DuplicateFilterId)
        ));
    }
}
