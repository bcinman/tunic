//! Version 1 storage DTOs, deliberately separate from domain/runtime types.

use serde::{Deserialize, Serialize};
use tunic_core as core;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    profiles: Vec<Profile>,
    selections: Vec<Selection>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    device: String,
    profile: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    id: String,
    name: String,
    preamp: f64,
    filters: Vec<Filter>,
    controls: Vec<Control>,
    origin: Option<Origin>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    id: u32,
    kind: FilterKind,
    frequency: f64,
    gain: f64,
    quality_factor: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    target: u32,
    name: String,
    gain_adjustment: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Origin {
    id: String,
    revision: String,
    provider: String,
    measurement_source: Option<String>,
    source_url: String,
}

pub(super) fn encode(state: &core::State) -> Result<String, core::PersistenceError> {
    let snapshot = Snapshot {
        profiles: state
            .profiles
            .iter()
            .map(|profile| Profile {
                id: profile.id().to_string(),
                name: profile.name().to_string(),
                preamp: profile.base().equalizer.preamp.into_inner(),
                filters: profile
                    .base()
                    .equalizer
                    .filters
                    .iter()
                    .map(|filter| Filter {
                        id: filter.id.into_inner(),
                        kind: match filter.parameters.kind {
                            core::FilterKind::Peaking => FilterKind::Peaking,
                            core::FilterKind::LowShelf => FilterKind::LowShelf,
                            core::FilterKind::HighShelf => FilterKind::HighShelf,
                        },
                        frequency: filter.parameters.frequency.into_inner(),
                        gain: filter.parameters.gain.into_inner(),
                        quality_factor: filter.parameters.quality_factor.into_inner(),
                    })
                    .collect(),
                controls: profile
                    .controls()
                    .iter()
                    .map(|control| Control {
                        target: control.target().into_inner(),
                        name: control.name().to_string(),
                        gain_adjustment: control.gain_adjustment().into_inner(),
                    })
                    .collect(),
                origin: profile.origin().map(|origin| Origin {
                    id: origin.id.to_string(),
                    revision: origin.revision.to_string(),
                    provider: origin.attribution.provider.clone(),
                    measurement_source: origin.attribution.measurement_source.clone(),
                    source_url: origin.attribution.source_url.clone(),
                }),
            })
            .collect(),
        selections: state
            .selections
            .iter()
            .map(|selection| Selection {
                device: selection.device.to_string(),
                profile: selection.profile.to_string(),
            })
            .collect(),
    };
    serde_json::to_string(&snapshot).map_err(corrupt)
}

pub(super) fn decode(payload: &str) -> Result<core::State, core::PersistenceError> {
    let snapshot: Snapshot = serde_json::from_str(payload).map_err(corrupt)?;
    Ok(core::State {
        profiles: snapshot
            .profiles
            .into_iter()
            .map(Profile::restore)
            .collect::<Result<_, _>>()?,
        selections: snapshot
            .selections
            .into_iter()
            .map(|selection| {
                Ok(core::DeviceProfileSelection {
                    device: core::DeviceId::try_new(selection.device).map_err(corrupt)?,
                    profile: core::ProfileId::try_new(selection.profile).map_err(corrupt)?,
                })
            })
            .collect::<Result<_, core::PersistenceError>>()?,
    })
}

impl Profile {
    fn restore(self) -> Result<core::Profile, core::PersistenceError> {
        let filters = self
            .filters
            .into_iter()
            .map(|filter| {
                Ok(core::Filter {
                    id: core::FilterId::try_new(filter.id).map_err(corrupt)?,
                    parameters: core::FilterParameters {
                        kind: match filter.kind {
                            FilterKind::Peaking => core::FilterKind::Peaking,
                            FilterKind::LowShelf => core::FilterKind::LowShelf,
                            FilterKind::HighShelf => core::FilterKind::HighShelf,
                        },
                        frequency: core::FrequencyHz::try_new(filter.frequency).map_err(corrupt)?,
                        gain: core::GainDb::try_new(filter.gain).map_err(corrupt)?,
                        quality_factor: core::QualityFactor::try_new(filter.quality_factor)
                            .map_err(corrupt)?,
                    },
                })
            })
            .collect::<Result<_, core::PersistenceError>>()?;
        let controls = self
            .controls
            .iter()
            .map(|control| {
                Ok(core::FilterControl::new(
                    core::FilterId::try_new(control.target).map_err(corrupt)?,
                    core::FilterControlName::try_new(control.name.clone()).map_err(corrupt)?,
                ))
            })
            .collect::<Result<_, core::PersistenceError>>()?;
        let origin = self
            .origin
            .map(|origin| {
                Ok(core::PresetOrigin {
                    id: core::PresetId::try_new(origin.id).map_err(corrupt)?,
                    revision: core::PresetRevision::try_new(origin.revision).map_err(corrupt)?,
                    attribution: core::Attribution {
                        provider: origin.provider,
                        measurement_source: origin.measurement_source,
                        source_url: origin.source_url,
                    },
                })
            })
            .transpose()?;
        let mut profile = core::Profile::new(
            core::ProfileId::try_new(self.id).map_err(corrupt)?,
            core::ProfileName::try_new(self.name).map_err(corrupt)?,
            core::Chain {
                equalizer: core::Equalizer {
                    preamp: core::GainDb::try_new(self.preamp).map_err(corrupt)?,
                    filters,
                },
            },
            controls,
            origin,
        )
        .map_err(corrupt)?;
        for control in self.controls {
            profile
                .adjust_filter_gain(
                    core::FilterId::try_new(control.target).map_err(corrupt)?,
                    core::GainDb::try_new(control.gain_adjustment).map_err(corrupt)?,
                )
                .map_err(corrupt)?;
        }
        Ok(profile)
    }
}

fn corrupt(error: impl std::fmt::Debug) -> core::PersistenceError {
    core::PersistenceError::Corrupt {
        message: format!("Invalid session snapshot: {error:?}"),
    }
}
