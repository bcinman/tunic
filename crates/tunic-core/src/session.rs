//! Application state and persistence/audio orchestration.
//!
//! Session owns the only editable draft. Candidates become saved state only
//! after persistence accepts them; audio publication is a separate effect.

mod catalog;
mod command;
mod persistence;
mod profile;
mod state;
#[cfg(test)]
mod tests;

pub use catalog::{PresetCatalog, PresetQuery};
pub use command::Command;
pub use persistence::{MemoryPersistence, Persistence, PersistenceError};
pub use profile::{
    Attribution, FilterControl, FilterControlName, FilterControlNameError, Preset, PresetId,
    PresetIdError, PresetOrigin, PresetRevision, PresetRevisionError, PresetSummary, Profile,
    ProfileError, ProfileId, ProfileIdError, ProfileName, ProfileNameError,
};
pub use state::{DeviceId, DeviceIdError, DeviceProfileSelection, State};

use std::{collections::HashSet, sync::Arc};

use crate::{Chain, Controller, FilterId, FrequencyHz, GainDb, SampleRateHz, Telemetry};

pub type ChangeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct Connection {
    pub device_name: String,
    pub controller: Controller,
}

/// Owns native routes. Each new connection must already be running `active_chain`.
/// An unchanged route returns `None`; an error tears down the previous route.
pub trait Platform {
    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), String>;
    fn refresh_default_output(
        &mut self,
        active_chain: &Chain,
    ) -> Result<Option<Connection>, String>;
}

const SYSTEM_OUTPUT: &str = "system-output";
const UNAVAILABLE_DEVICE: &str = "System Output (audio unavailable)";
const FLAT_PROFILE: &str = "flat";

/// UI-independent application boundary for Tunic.
///
/// # Integrating a frontend
///
/// The host constructs one session with a [`Persistence`] implementation and a
/// [`PresetCatalog`], optionally attaching audio with [`Self::with_audio`]. Share
/// that session between views of the same application; creating one per view
/// creates independent state and drafts. Session contains no GPUI or other UI
/// framework types, notification mechanism, or event loop.
///
/// Serialize application actions through [`Self::execute`]. Commands are
/// synchronous and may perform persistence or platform I/O, so run them on the
/// host's application owner, never on the real-time audio callback. Native
/// callbacks should enqueue work for that owner rather than reenter Session.
/// After every command, including an error, reread state and notify your views.
/// An error does not necessarily mean nothing changed.
///
/// Keep focus, dragging, coordinate conversion, layout, and animation in the
/// frontend. Session owns profile selection, edits, save/reset, DSP publication,
/// and audio lifecycle. Do not duplicate those workflows in a UI adapter or
/// publish chains directly through a separate controller.
///
/// # Reading and editing
///
/// - [`Self::state`] and [`Self::selected_profile`] expose saved state, not edits.
/// - [`Self::editing_profile`] overlays the current draft on the saved profile;
///   use it for editable controls. [`Self::draft`] indicates an unsaved edit
///   session, not necessarily a value different from the saved profile.
/// - [`Self::base_chain`] is the editable EQ base. [`Self::active_chain`] is the
///   desired chain including named gain adjustments, not proof it is audible.
/// - [`Self::applied_chain`] is the last controller-accepted chain; it can differ
///   from the desired chain or be absent when audio is disconnected.
///
/// Submit intent through [`Command`], not a cloned/replaced [`Profile`]. Session
/// owns the only draft, so there are no profile revision counters or external
/// snapshot-save API. Base EQ edits preview without saving. Named adjustments
/// persist immediately, updating any base EQ draft without saving its base.
/// Save persists the base EQ draft without republishing audio; Reset restores
/// the saved base with current named adjustments. Successful selection
/// persistence discards the previous draft.
///
/// # Failures and audio lifecycle
///
/// Failed persistence preserves saved state and the draft. A rejected edit or
/// reset preserves the previous draft. Selection and named adjustments persist
/// *before* audio publication, so DSP rejection can leave new settings saved while the
/// previous chain remains applied. Display both [`Self::action_error`] and
/// [`Self::audio_error`]: a successful no-op may clear the last action error but
/// does not resolve an outstanding audio failure.
///
/// The [`ChangeHandler`] passed to [`Self::with_audio`] should wake the host to
/// execute [`Command::RefreshAudio`]. Check [`Self::audio_retry_needed`] after
/// initialization and refresh; schedule delayed retries while it is true.
/// Watcher-installation failure is not retryable through RefreshAudio: the host
/// must successfully attach audio again. Session owns the attached platform;
/// the platform implementation owns and tears down native resources.
///
/// Subscribe through [`Self::subscribe_telemetry`]. When
/// [`Self::audio_generation`] changes, drop the old subscription, subscribe
/// again, and reset frontend animation state. Telemetry polling and display
/// smoothing remain frontend concerns. Session exposes no automatic redraws.
pub struct Session {
    persistence: Box<dyn Persistence>,
    state: State,
    catalog: Box<dyn PresetCatalog>,
    device: DeviceId,
    device_name: String,
    controller: Option<Controller>,
    draft: Option<Profile>,
    presets: Vec<PresetSummary>,
    // Present only after successful watcher installation.
    platform: Option<Box<dyn Platform>>,
    audio_error: Option<String>,
    publication_error: Option<String>,
    action_error: Option<SessionError>,
    audio_retry_needed: bool,
    audio_generation: u64,
    applied_chain: Option<Chain>,
}

impl Session {
    /// Loads and validates saved state without opening an audio route.
    pub fn new(
        mut persistence: impl Persistence + 'static,
        catalog: impl PresetCatalog + 'static,
    ) -> Result<Self, SessionError> {
        let state = persistence
            .load()
            .map_err(SessionError::PersistenceFailed)?
            .unwrap_or_default();
        validate_state(&state)?;
        let presets = catalog.list(&PresetQuery::default());
        Ok(Self {
            persistence: Box::new(persistence),
            state,
            catalog: Box::new(catalog),
            device: DeviceId::try_new(SYSTEM_OUTPUT).expect("static device ID is valid"),
            device_name: UNAVAILABLE_DEVICE.into(),
            controller: None,
            draft: None,
            presets,
            platform: None,
            audio_error: None,
            publication_error: None,
            action_error: None,
            audio_retry_needed: false,
            audio_generation: 0,
            applied_chain: None,
        })
    }

    /// Attaches a platform, installs its watcher, and attempts the initial route.
    ///
    /// Replaces any previously attached platform. Inspect [`Self::audio_error`]
    /// and [`Self::audio_retry_needed`] on the returned session; initialization
    /// failure does not prevent offline editing. `notify` may run on a native
    /// thread and must only schedule work on the session's owner.
    #[must_use]
    pub fn with_audio(
        mut self,
        mut platform: impl Platform + 'static,
        notify: ChangeHandler,
    ) -> Self {
        self.platform = None;
        if self.controller.take().is_some() {
            self.audio_generation += 1;
        }
        self.device_name = UNAVAILABLE_DEVICE.into();
        self.applied_chain = None;
        self.publication_error = None;
        self.audio_retry_needed = false;
        match platform.watch_default_output(notify) {
            Ok(()) => {
                self.platform = Some(Box::new(platform));
                let _ = self.refresh_audio();
            }
            Err(error) => self.audio_error = Some(error),
        }
        self
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn selected_profile(&self) -> Option<&Profile> {
        self.state.selected_profile(&self.device)
    }

    #[must_use]
    pub fn draft(&self) -> Option<&Profile> {
        self.draft.as_ref()
    }

    #[must_use]
    pub fn editing_profile(&self) -> Option<&Profile> {
        self.draft.as_ref().or_else(|| self.selected_profile())
    }

    #[must_use]
    pub fn active_chain(&self) -> Chain {
        self.editing_profile()
            .map_or_else(Chain::default, Profile::effective_chain)
    }

    #[must_use]
    pub fn base_chain(&self) -> Chain {
        self.editing_profile()
            .map_or_else(Chain::default, |profile| profile.base().clone())
    }

    #[must_use]
    pub fn sample_rate(&self) -> SampleRateHz {
        self.controller.as_ref().map_or_else(
            || SampleRateHz::try_new(48_000.0).expect("valid fallback"),
            Controller::sample_rate,
        )
    }

    #[must_use]
    pub fn presets(&self) -> Vec<PresetSummary> {
        self.presets.clone()
    }

    #[must_use]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Route failure or an unresolved failure to apply the desired chain.
    #[must_use]
    pub fn audio_error(&self) -> Option<&str> {
        self.audio_error
            .as_deref()
            .or(self.publication_error.as_deref())
    }

    /// Last non-refresh command failure; cleared by a successful non-refresh command.
    /// Audio refresh leaves this unchanged. Also read [`Self::audio_error`].
    #[must_use]
    pub fn action_error(&self) -> Option<&SessionError> {
        self.action_error.as_ref()
    }

    /// Subscribes to the current route, or returns `None` while disconnected.
    /// Replace this subscription whenever [`Self::audio_generation`] changes.
    #[must_use]
    pub fn subscribe_telemetry(&self) -> Option<Telemetry> {
        self.controller
            .as_ref()
            .map(Controller::subscribe_telemetry)
    }

    /// Changes when a route is installed or lost; not a profile revision.
    #[must_use]
    pub fn audio_generation(&self) -> u64 {
        self.audio_generation
    }

    /// Last chain accepted by the controller, not an acknowledgement from the audio callback.
    #[must_use]
    pub fn applied_chain(&self) -> Option<&Chain> {
        self.applied_chain.as_ref()
    }

    #[must_use]
    pub fn audio_retry_needed(&self) -> bool {
        self.audio_retry_needed
    }

    /// Selection is persisted before audio publication; an audio error does not roll it back.
    pub fn execute(&mut self, command: Command) -> Result<(), SessionError> {
        let result = match command {
            Command::RefreshAudio => return self.refresh_audio(),
            Command::UseFlat => self.use_profile(None),
            Command::UsePreset(id) => self.use_profile(Some(id)),
            Command::EditFilter {
                filter,
                frequency,
                gain,
            } => self.edit_filter(filter, frequency, gain),
            Command::SetControlGain { filter, gain } => self.set_control_gain(filter, gain),
            Command::SaveDraft => self.save_draft(),
            Command::ResetDraft => self.reset_draft(),
            Command::ClearSelection => self.clear_selection(),
        };
        self.action_error = result.as_ref().err().cloned();
        result
    }

    fn commit(&mut self, next: State) -> Result<(), SessionError> {
        if next != self.state {
            self.persistence
                .save(&next)
                .map_err(SessionError::PersistenceFailed)?;
            self.state = next;
        }
        Ok(())
    }

    fn use_profile(&mut self, preset: Option<PresetId>) -> Result<(), SessionError> {
        let id = ProfileId::try_new(preset.as_ref().map_or_else(
            || FLAT_PROFILE.to_owned(),
            |id| format!("preset-{}", id.as_ref()),
        ))
        .expect("generated profile ID is nonempty");
        let mut next = self.state.clone();
        if next.profile(&id).is_none() {
            let profile = match preset {
                Some(preset_id) => {
                    let preset = self
                        .catalog
                        .get(&preset_id)
                        .ok_or(SessionError::PresetNotFound(preset_id))?;
                    let name = ProfileName::try_new(format!(
                        "{} {}",
                        preset.summary.brand, preset.summary.model
                    ))
                    .expect("catalog brand and model produce a nonempty name");
                    let origin = PresetOrigin {
                        id: preset.summary.id,
                        revision: preset.summary.revision,
                        attribution: preset.attribution,
                    };
                    Profile::new(
                        id.clone(),
                        name,
                        preset.chain,
                        preset.controls,
                        Some(origin),
                    )
                }
                None => Profile::new(
                    id.clone(),
                    ProfileName::try_new("Flat").expect("valid name"),
                    Chain::default(),
                    Vec::new(),
                    None,
                ),
            }
            .map_err(SessionError::InvalidProfile)?;
            if next
                .profiles
                .iter()
                .any(|saved| saved.name() == profile.name())
            {
                return Err(SessionError::ProfileNameAlreadyExists(
                    profile.name().clone(),
                ));
            }
            next.profiles.push(profile);
        }
        if let Some(selection) = next
            .selections
            .iter_mut()
            .find(|selection| selection.device == self.device)
        {
            selection.profile = id;
        } else {
            next.selections.push(DeviceProfileSelection {
                device: self.device.clone(),
                profile: id,
            });
        }
        self.commit(next)?;
        self.draft = None;
        self.publish_active_chain()
    }

    fn clear_selection(&mut self) -> Result<(), SessionError> {
        let mut next = self.state.clone();
        next.selections
            .retain(|selection| selection.device != self.device);
        self.commit(next)?;
        self.draft = None;
        self.publish_active_chain()
    }

    fn edit_filter(
        &mut self,
        filter: FilterId,
        frequency: FrequencyHz,
        gain: GainDb,
    ) -> Result<(), SessionError> {
        let mut draft = self
            .editing_profile()
            .cloned()
            .ok_or(SessionError::NoSelectedProfile)?;
        let mut base = draft.base().clone();
        let target = base
            .equalizer
            .filters
            .iter_mut()
            .find(|candidate| candidate.id == filter)
            .ok_or(SessionError::InvalidProfile(ProfileError::FilterNotFound(
                filter,
            )))?;
        target.parameters.frequency = frequency;
        target.parameters.gain = gain;
        draft
            .replace_base(base)
            .map_err(SessionError::InvalidProfile)?;
        self.publish(draft.effective_chain())?;
        self.draft = Some(draft);
        self.publication_error = None;
        Ok(())
    }

    fn set_control_gain(&mut self, filter: FilterId, gain: GainDb) -> Result<(), SessionError> {
        let id = self
            .selected_profile()
            .ok_or(SessionError::NoSelectedProfile)?
            .id();
        let mut next = self.state.clone();
        let saved = next
            .profiles
            .iter_mut()
            .find(|saved| saved.id() == id)
            .expect("Selected profiles always belong to saved state");
        saved
            .adjust_filter_gain(filter, gain)
            .map_err(SessionError::InvalidProfile)?;
        let mut draft = self.draft.clone();
        if let Some(draft) = &mut draft {
            draft
                .adjust_filter_gain(filter, gain)
                .map_err(SessionError::InvalidProfile)?;
        }
        self.commit(next)?;
        self.draft = draft;
        self.publish_active_chain()
    }

    fn save_draft(&mut self) -> Result<(), SessionError> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        let mut next = self.state.clone();
        let saved = next
            .profiles
            .iter_mut()
            .find(|profile| profile.id() == draft.id())
            .expect("Session drafts always belong to a saved profile");
        *saved = draft.clone();
        self.commit(next)?;
        self.draft = None;
        // Saving must not republish a preview or reset processor history.
        Ok(())
    }

    fn reset_draft(&mut self) -> Result<(), SessionError> {
        if self.draft.is_none() {
            return Ok(());
        }
        let chain = self
            .selected_profile()
            .map_or_else(Chain::default, Profile::effective_chain);
        self.publish(chain)?;
        self.draft = None;
        self.publication_error = None;
        Ok(())
    }

    fn publish_active_chain(&mut self) -> Result<(), SessionError> {
        let result = self.publish(self.active_chain());
        self.publication_error = match &result {
            Err(SessionError::Audio(message)) => Some(message.clone()),
            _ => None,
        };
        result
    }

    fn publish(&mut self, chain: Chain) -> Result<(), SessionError> {
        if self.applied_chain.as_ref() == Some(&chain) {
            return Ok(());
        }
        if let Some(controller) = &self.controller {
            controller
                .set_chain(chain.clone())
                .map_err(|error| SessionError::Audio(format!("publish chain: {error:?}")))?;
            self.applied_chain = Some(chain);
        }
        Ok(())
    }

    fn refresh_audio(&mut self) -> Result<(), SessionError> {
        let chain = self.active_chain();
        let platform = self.platform.as_mut().ok_or_else(|| {
            SessionError::Audio(
                self.audio_error
                    .clone()
                    .unwrap_or_else(|| "audio platform is unavailable".into()),
            )
        })?;
        match platform.refresh_default_output(&chain) {
            Ok(Some(connection)) => {
                self.device_name = connection.device_name;
                self.controller = Some(connection.controller);
                self.audio_error = None;
                self.publication_error = None;
                self.audio_retry_needed = false;
                self.audio_generation += 1;
                self.applied_chain = Some(chain);
                Ok(())
            }
            Ok(None) => {
                self.audio_error = None;
                self.audio_retry_needed = false;
                // An unchanged route does not acknowledge a rejected publication.
                Ok(())
            }
            Err(message) => {
                if self.controller.take().is_some() {
                    self.audio_generation += 1;
                }
                self.device_name = UNAVAILABLE_DEVICE.into();
                self.applied_chain = None;
                self.publication_error = None;
                self.audio_error = Some(message.clone());
                self.audio_retry_needed = true;
                Err(SessionError::Audio(message))
            }
        }
    }
}

fn validate_state(state: &State) -> Result<(), SessionError> {
    let mut profile_ids = HashSet::new();
    let mut profile_names = HashSet::new();
    for profile in &state.profiles {
        profile
            .validate()
            .map_err(|error| corrupt(format!("{error:?}")))?;
        if !profile_ids.insert(profile.id().clone()) {
            return Err(corrupt(format!("duplicate profile id '{}'", profile.id())));
        }
        if !profile_names.insert(profile.name().clone()) {
            return Err(corrupt(format!(
                "duplicate profile name '{}'",
                profile.name()
            )));
        }
    }
    let mut selected_devices = HashSet::new();
    for selection in &state.selections {
        if !selected_devices.insert(selection.device.clone()) {
            return Err(corrupt(format!(
                "multiple profile selections for device '{}'",
                selection.device
            )));
        }
        if !profile_ids.contains(&selection.profile) {
            return Err(corrupt(format!(
                "device '{}' selects missing profile '{}'",
                selection.device, selection.profile
            )));
        }
    }
    Ok(())
}

fn corrupt(message: String) -> SessionError {
    SessionError::PersistenceFailed(PersistenceError::Corrupt { message })
}

/// A rejected command or failed session operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    NoSelectedProfile,
    Audio(String),
    PresetNotFound(PresetId),
    InvalidProfile(ProfileError),
    ProfileNameAlreadyExists(ProfileName),
    PersistenceFailed(PersistenceError),
}
