use boltffi::{data, error, export};
use tunic_core as core;

#[error]
#[derive(Debug)]
pub enum EngineError {
    InvalidInput { message: String },
    Unavailable { message: String },
    Closed,
}

impl EngineError {
    pub(crate) fn invalid(error: impl std::fmt::Debug) -> Self {
        Self::InvalidInput {
            message: format!("{error:?}"),
        }
    }
}

#[data]
#[derive(Clone, Debug)]
pub enum EngineCommand {
    UseFlat,
    UsePreset {
        id: String,
    },
    EditFilter {
        filter: u32,
        frequency: f64,
        gain: f64,
    },
    SetControlGain {
        filter: u32,
        gain: f64,
    },
    SaveDraft,
    ResetDraft,
    ClearSelection,
}

impl TryFrom<EngineCommand> for core::Command {
    type Error = EngineError;

    fn try_from(command: EngineCommand) -> Result<Self, Self::Error> {
        Ok(match command {
            EngineCommand::UseFlat => Self::UseFlat,
            EngineCommand::UsePreset { id } => {
                Self::UsePreset(core::PresetId::try_new(id).map_err(EngineError::invalid)?)
            }
            EngineCommand::EditFilter {
                filter,
                frequency,
                gain,
            } => Self::EditFilter {
                filter: core::FilterId::try_new(filter).map_err(EngineError::invalid)?,
                frequency: core::FrequencyHz::try_new(frequency).map_err(EngineError::invalid)?,
                gain: core::GainDb::try_new(gain).map_err(EngineError::invalid)?,
            },
            EngineCommand::SetControlGain { filter, gain } => Self::SetControlGain {
                filter: core::FilterId::try_new(filter).map_err(EngineError::invalid)?,
                gain: core::GainDb::try_new(gain).map_err(EngineError::invalid)?,
            },
            EngineCommand::SaveDraft => Self::SaveDraft,
            EngineCommand::ResetDraft => Self::ResetDraft,
            EngineCommand::ClearSelection => Self::ClearSelection,
        })
    }
}

#[data]
#[derive(Clone, Copy, Debug)]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[data]
#[derive(Clone, Debug)]
pub struct Filter {
    pub id: u32,
    pub kind: FilterKind,
    pub frequency: f64,
    pub gain: f64,
    pub quality_factor: f64,
}

#[data]
#[derive(Clone, Debug)]
pub struct Chain {
    pub preamp: f64,
    pub filters: Vec<Filter>,
}

impl From<core::Chain> for Chain {
    fn from(chain: core::Chain) -> Self {
        Self {
            preamp: chain.equalizer.preamp.into_inner(),
            filters: chain
                .equalizer
                .filters
                .into_iter()
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
        }
    }
}

impl TryFrom<Chain> for core::Chain {
    type Error = EngineError;

    fn try_from(chain: Chain) -> Result<Self, Self::Error> {
        Ok(Self {
            equalizer: core::Equalizer {
                preamp: core::GainDb::try_new(chain.preamp).map_err(EngineError::invalid)?,
                filters: chain
                    .filters
                    .into_iter()
                    .map(|filter| {
                        Ok(core::Filter {
                            id: core::FilterId::try_new(filter.id).map_err(EngineError::invalid)?,
                            parameters: core::FilterParameters {
                                kind: match filter.kind {
                                    FilterKind::Peaking => core::FilterKind::Peaking,
                                    FilterKind::LowShelf => core::FilterKind::LowShelf,
                                    FilterKind::HighShelf => core::FilterKind::HighShelf,
                                },
                                frequency: core::FrequencyHz::try_new(filter.frequency)
                                    .map_err(EngineError::invalid)?,
                                gain: core::GainDb::try_new(filter.gain)
                                    .map_err(EngineError::invalid)?,
                                quality_factor: core::QualityFactor::try_new(filter.quality_factor)
                                    .map_err(EngineError::invalid)?,
                            },
                        })
                    })
                    .collect::<Result<_, EngineError>>()?,
            },
        })
    }
}

#[data]
#[derive(Clone, Debug)]
pub struct Control {
    pub filter: u32,
    pub name: String,
    pub gain: f64,
}

#[data]
#[derive(Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub brand: String,
    pub model: String,
    pub variant: Option<String>,
}

#[data]
#[derive(Clone, Debug)]
pub struct StateSnapshot {
    /// Notification sequence, not a profile revision or audio acknowledgement.
    pub revision: u64,
    pub device_name: String,
    pub profile_name: Option<String>,
    pub preset_id: Option<String>,
    pub has_draft: bool,
    pub base_chain: Chain,
    pub desired_chain: Chain,
    /// Last controller-accepted chain; not an acknowledgement from the callback.
    pub accepted_chain: Option<Chain>,
    pub controls: Vec<Control>,
    pub presets: Vec<Preset>,
    pub sample_rate: f64,
    pub audio_generation: u64,
    pub action_error: Option<String>,
    pub audio_error: Option<String>,
}

impl StateSnapshot {
    pub(crate) fn read(session: &core::Session, revision: u64) -> Self {
        let profile = session.editing_profile();
        Self {
            revision,
            device_name: session.device_name().to_owned(),
            profile_name: profile.map(|p| p.name().to_string()),
            preset_id: profile.and_then(|p| p.origin()).map(|o| o.id.to_string()),
            has_draft: session.draft().is_some(),
            base_chain: session.base_chain().into(),
            desired_chain: session.active_chain().into(),
            accepted_chain: session.applied_chain().cloned().map(Into::into),
            controls: profile
                .map(|p| {
                    p.controls()
                        .iter()
                        .map(|c| Control {
                            filter: c.target().into_inner(),
                            name: c.name().to_string(),
                            gain: c.gain_adjustment().into_inner(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            presets: session
                .presets()
                .into_iter()
                .map(|p| Preset {
                    id: p.id.to_string(),
                    brand: p.brand,
                    model: p.model,
                    variant: p.variant,
                })
                .collect(),
            sample_rate: session.sample_rate().into_inner(),
            audio_generation: session.audio_generation(),
            action_error: session.action_error().map(|error| format!("{error:?}")),
            audio_error: session.audio_error().map(str::to_owned),
        }
    }
}

#[data]
#[derive(Clone, Debug)]
pub struct TelemetryFrame {
    pub sequence: u64,
    pub left_peak: f32,
    pub left_rms: f32,
    pub right_peak: f32,
    pub right_rms: f32,
    /// Linear amplitudes, logarithmically spaced from 20 Hz to 20 kHz.
    pub spectrum: Vec<f32>,
}

impl From<core::TelemetryFrame> for TelemetryFrame {
    fn from(frame: core::TelemetryFrame) -> Self {
        Self {
            sequence: frame.sequence,
            left_peak: frame.levels.left.peak,
            left_rms: frame.levels.left.rms,
            right_peak: frame.levels.right.peak,
            right_rms: frame.levels.right.rms,
            spectrum: frame.spectrum.points.to_vec(),
        }
    }
}

#[data]
#[derive(Clone, Debug)]
pub struct TelemetryBatch {
    pub audio_generation: u64,
    pub frames: Vec<TelemetryFrame>,
}

/// Uses the exact processing coefficients. Like core's FrequencyResponse,
/// this excludes preamp gain. Supply all graph frequencies in one call.
#[export]
pub fn frequency_response(
    chain: Chain,
    sample_rate: f64,
    frequencies: Vec<f64>,
) -> Result<Vec<f64>, EngineError> {
    let response = core::FrequencyResponse::new(
        &chain.try_into()?,
        core::SampleRateHz::try_new(sample_rate).map_err(EngineError::invalid)?,
    )
    .map_err(EngineError::invalid)?;
    frequencies
        .into_iter()
        .map(|frequency| {
            response
                .db_at(core::FrequencyHz::try_new(frequency).map_err(EngineError::invalid)?)
                .map_err(EngineError::invalid)
        })
        .collect()
}
