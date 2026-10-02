//! The on-disk format, shared by build-time validation and lazy runtime decoding.

use serde::Deserialize;
use std::collections::HashSet;
use tunic_core as core;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    summary: Summary,
    attribution: Attribution,
    equalizer: Equalizer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    id: String,
    revision: String,
    brand: String,
    model: String,
    variant: Option<String>,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attribution {
    provider: String,
    measurement_source: Option<String>,
    source_url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Equalizer {
    preamp_gain_db: f64,
    filters: Vec<Filter>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    id: u32,
    control_name: Option<String>,
    kind: FilterKind,
    frequency_hz: f64,
    gain_db: f64,
    quality_factor: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

fn text(value: String, field: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be blank"));
    }
    Ok(value)
}

pub(crate) fn decode(json: &str) -> Result<core::Preset, String> {
    let document: Document = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let summary = document.summary;
    let attribution = document.attribution;
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut controls = Vec::new();
    let filters = document
        .equalizer
        .filters
        .into_iter()
        .map(|filter| {
            let id = core::FilterId::try_new(filter.id)
                .map_err(|_| String::from("invalid filter id"))?;
            if !ids.insert(id) {
                return Err(format!("duplicate filter id {id}"));
            }
            let control_name = filter
                .control_name
                .map(|name| {
                    let name = core::FilterControlName::try_new(text(name, "filter control name")?)
                        .map_err(|_| String::from("invalid filter control name"))?;
                    if !names.insert(name.clone()) {
                        return Err(format!("duplicate filter control name {name}"));
                    }
                    Ok(name)
                })
                .transpose()?;
            if let Some(name) = control_name {
                controls.push(core::FilterControl::new(id, name));
            }
            Ok(core::Filter {
                id,
                parameters: core::FilterParameters {
                    kind: match filter.kind {
                        FilterKind::Peaking => core::FilterKind::Peaking,
                        FilterKind::LowShelf => core::FilterKind::LowShelf,
                        FilterKind::HighShelf => core::FilterKind::HighShelf,
                    },
                    frequency: core::FrequencyHz::try_new(filter.frequency_hz)
                        .map_err(|_| String::from("invalid filter frequency"))?,
                    gain: core::GainDb::try_new(filter.gain_db)
                        .map_err(|_| String::from("invalid filter gain"))?,
                    quality_factor: core::QualityFactor::try_new(filter.quality_factor)
                        .map_err(|_| String::from("invalid filter Q"))?,
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(core::Preset {
        summary: core::PresetSummary {
            id: core::PresetId::try_new(text(summary.id, "preset id")?)
                .map_err(|_| String::from("invalid preset id"))?,
            revision: core::PresetRevision::try_new(text(summary.revision, "revision")?)
                .map_err(|_| String::from("invalid revision"))?,
            brand: text(summary.brand, "brand")?,
            model: text(summary.model, "model")?,
            variant: summary.variant.map(|v| text(v, "variant")).transpose()?,
            target: text(summary.target, "target")?,
        },
        attribution: core::Attribution {
            provider: text(attribution.provider, "provider")?,
            measurement_source: attribution
                .measurement_source
                .map(|v| text(v, "measurement source"))
                .transpose()?,
            source_url: text(attribution.source_url, "source URL")?,
        },
        chain: core::Chain {
            equalizer: core::Equalizer {
                preamp: core::GainDb::try_new(document.equalizer.preamp_gain_db)
                    .map_err(|_| String::from("invalid preamp gain"))?,
                filters,
            },
        },
        controls,
    })
}
