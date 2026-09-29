//! Offline, immutable preset catalog. Browsing never decodes preset payloads.

mod format;

use tunic_core::{Preset, PresetCatalog, PresetId, PresetQuery, PresetRevision, PresetSummary};

struct Entry {
    id: &'static str,
    revision: &'static str,
    brand: &'static str,
    model: &'static str,
    variant: Option<&'static str>,
    target: &'static str,
    json: &'static str,
}

impl Entry {
    fn summary(&self) -> PresetSummary {
        PresetSummary {
            id: PresetId::try_new(self.id).expect("build-validated id"),
            revision: PresetRevision::try_new(self.revision).expect("build-validated revision"),
            brand: self.brand.into(),
            model: self.model.into(),
            variant: self.variant.map(str::to_owned),
            target: self.target.into(),
        }
    }
}

struct Brand {
    name: &'static str,
    models: &'static [Model],
}

struct Model {
    name: &'static str,
    indices: &'static [usize],
}

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

/// Zero-state handle to build-generated indexes and lazily decoded JSON payloads.
#[derive(Clone, Copy, Debug, Default)]
pub struct BundledCatalog;

fn brand(name: &str) -> Option<&'static Brand> {
    BRANDS
        .binary_search_by_key(&name, |brand| brand.name)
        .ok()
        .map(|i| &BRANDS[i])
}

impl PresetCatalog for BundledCatalog {
    fn brands(&self) -> Vec<String> {
        BRANDS.iter().map(|brand| brand.name.to_owned()).collect()
    }

    fn models(&self, name: &str) -> Vec<String> {
        brand(name)
            .into_iter()
            .flat_map(|brand| brand.models)
            .map(|model| model.name.to_owned())
            .collect()
    }

    fn list(&self, query: &PresetQuery) -> Vec<PresetSummary> {
        let brands = match query.brand.as_deref() {
            Some(name) => brand(name).map(std::slice::from_ref).unwrap_or_default(),
            None => BRANDS,
        };
        brands
            .iter()
            .flat_map(|brand| match query.model.as_deref() {
                Some(name) => brand
                    .models
                    .binary_search_by_key(&name, |model| model.name)
                    .ok()
                    .map(|i| &brand.models[i..=i])
                    .unwrap_or_default(),
                None => brand.models,
            })
            .flat_map(|model| model.indices)
            .map(|&i| ENTRIES[i].summary())
            .collect()
    }

    fn get(&self, id: &PresetId) -> Option<Preset> {
        IDS.binary_search_by_key(&id.as_ref(), |(id, _)| *id)
            .ok()
            .map(|i| {
                format::decode(ENTRIES[IDS[i].1].json).expect("build-validated preset payload")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tunic_core::{AdjustableParameter, AudioFormat, FilterKind, Processor, SampleRateHz};

    #[test]
    fn indexes_resolve_every_entry_and_intersect_exact_filters() {
        let catalog = BundledCatalog;
        assert_eq!(catalog.brands(), ["Sennheiser", "Sony"]);
        assert_eq!(catalog.models("Sennheiser"), ["HD650"]);
        assert!(catalog.models("sennheiser").is_empty());
        let all = catalog.list(&PresetQuery::default());
        assert_eq!(all.len(), ENTRIES.len());
        for summary in &all {
            assert_eq!(catalog.get(&summary.id).unwrap().summary, *summary);
            for query in [
                PresetQuery {
                    brand: Some(summary.brand.clone()),
                    model: None,
                },
                PresetQuery {
                    brand: None,
                    model: Some(summary.model.clone()),
                },
                PresetQuery {
                    brand: Some(summary.brand.clone()),
                    model: Some(summary.model.clone()),
                },
            ] {
                let expected = all
                    .iter()
                    .filter(|s| {
                        query.brand.as_ref().is_none_or(|b| b == &s.brand)
                            && query.model.as_ref().is_none_or(|m| m == &s.model)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(catalog.list(&query), expected);
            }
        }
        for (brand, model) in [
            (Some("Sony"), Some("HD650")),
            (Some("missing"), None),
            (None, Some("missing")),
        ] {
            assert!(
                catalog
                    .list(&PresetQuery {
                        brand: brand.map(str::to_owned),
                        model: model.map(str::to_owned),
                    })
                    .is_empty()
            );
        }
        assert!(
            catalog
                .get(&PresetId::try_new("missing").unwrap())
                .is_none()
        );
    }

    #[test]
    fn hd650_preserves_source_values_and_adjustment_references() {
        let preset = BundledCatalog
            .get(&PresetId::try_new("oratory1990/sennheiser/hd650/harman").unwrap())
            .unwrap();
        assert_eq!(preset.chain.equalizer.preamp.into_inner(), -9.3);
        assert_eq!(preset.chain.equalizer.filters.len(), 10);
        let bass = preset
            .chain
            .equalizer
            .filters
            .iter()
            .find(|f| f.id.into_inner() == 3)
            .unwrap();
        assert_eq!(bass.kind, FilterKind::LowShelf);
        assert_eq!(bass.frequency.into_inner(), 105.0);
        assert_eq!(bass.gain.into_inner(), 5.5);
        assert_eq!(bass.quality_factor.into_inner(), 0.71);
        assert_eq!(
            preset
                .adjustments
                .iter()
                .map(|a| (a.label.as_str(), a.filter.into_inner(), a.parameter))
                .collect::<Vec<_>>(),
            vec![
                ("Bass", 3, AdjustableParameter::GainDb),
                ("Warmth / muddiness", 4, AdjustableParameter::GainDb),
                (
                    "Midrange accuracy / shoutiness",
                    5,
                    AdjustableParameter::GainDb
                ),
                ("Treble", 6, AdjustableParameter::GainDb),
                ("Airiness", 10, AdjustableParameter::GainDb),
            ]
        );
    }

    #[test]
    fn every_preset_compiles_at_common_playback_rates() {
        for summary in BundledCatalog.list(&PresetQuery::default()) {
            let preset = BundledCatalog.get(&summary.id).unwrap();
            for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
                assert!(
                    Processor::new(
                        AudioFormat {
                            sample_rate: SampleRateHz::try_new(rate).unwrap(),
                            maximum_frame_count: std::num::NonZeroUsize::new(256).unwrap(),
                        },
                        preset.chain.clone(),
                        false
                    )
                    .is_ok(),
                    "{} at {rate}",
                    summary.id
                );
            }
        }
    }

    #[test]
    fn schema_rejects_typos_unsupported_values_and_broken_references() {
        let valid: serde_json::Value = serde_json::from_str(ENTRIES[0].json).unwrap();
        for (pointer, value) in [
            ("/summary/id", serde_json::json!(" ")),
            ("/summary/brand", serde_json::json!("")),
            ("/equalizer/filters/0/id", serde_json::json!(0)),
            ("/equalizer/filters/0/id", serde_json::json!(2)),
            ("/equalizer/filters/0/frequency_hz", serde_json::json!(0)),
            ("/equalizer/filters/0/quality_factor", serde_json::json!(-1)),
            ("/equalizer/filters/0/kind", serde_json::json!("notch")),
            ("/equalizer/preamp_gain_db", serde_json::json!("NaN")),
            ("/adjustments/0/filter", serde_json::json!(999)),
            ("/adjustments/0/parameter", serde_json::json!("slope")),
            ("/adjustments/0/label", serde_json::json!(" ")),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(format::decode(&invalid.to_string()).is_err(), "{pointer}");
        }
        for pointer in [
            "",
            "/summary",
            "/attribution",
            "/equalizer",
            "/equalizer/filters/0",
            "/adjustments/0",
        ] {
            let mut invalid = valid.clone();
            invalid
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("typo".into(), serde_json::json!(true));
            assert!(format::decode(&invalid.to_string()).is_err(), "{pointer}");
        }
    }
}
