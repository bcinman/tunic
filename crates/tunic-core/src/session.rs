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
pub use command::{Command, ProfileEdit, ProfileSource};
pub use persistence::{MemoryPersistence, Persistence, PersistenceError};
pub use profile::{
    Attribution, FilterControl, FilterControlName, FilterControlNameError, Preset, PresetId,
    PresetIdError, PresetOrigin, PresetRevision, PresetRevisionError, PresetSummary, Profile,
    ProfileError, ProfileId, ProfileIdError, ProfileName, ProfileNameError,
};
pub use state::{DeviceId, DeviceIdError, DeviceProfiles, State};

use std::sync::Arc;

use crate::{Chain, Controller, FilterId, FrequencyHz, GainDb, SampleRateHz, Telemetry};

pub type ChangeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

pub struct Connection {
    pub device_id: DeviceId,
    pub device_name: String,
    pub controller: Controller,
}

/// Owns native routes. Resolve the stable device identity, then obtain its chain
/// from the callback before starting that exact route. Never start a new device
/// with the previous device's processing chain.
/// An unchanged route returns `None`; an error tears down the previous route.
pub trait Platform {
    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), String>;
    fn refresh_default_output(
        &mut self,
        chain_for_device: &dyn Fn(&DeviceId) -> Chain,
    ) -> Result<Option<Connection>, String>;
}

const SYSTEM_OUTPUT: &str = "system-output";
const UNAVAILABLE_DEVICE: &str = "System Output (audio unavailable)";

struct GainPreview {
    filter: FilterId,
    gain: GainDb,
    profile: Profile,
}

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
/// - [`Self::editing_profile`] overlays gain previews and base drafts on the saved profile;
///   use it for editable controls. [`Self::draft`] indicates an unsaved edit
///   session, not necessarily a value different from the saved profile.
/// - [`Self::base_chain`] is the editable EQ base. [`Self::active_chain`] is the
///   desired chain including named gain adjustments, not proof it is audible.
/// - [`Self::applied_chain`] is the last controller-accepted chain; it can differ
///   from the desired chain or be absent when audio is disconnected.
///
/// Submit intent through [`Command`], not a cloned/replaced [`Profile`].
/// Edits must carry the originating profile ID in [`Command::EditProfile`];
/// stale edits are rejected before touching previews or drafts.
/// Session owns the only draft, so there are no profile revision counters or external
/// snapshot-save API. Base EQ edits preview without saving. Named adjustments use
/// PreviewControlGain during interaction and FinishControlGain to persist once;
/// SetControlGain performs a discrete saved adjustment. Gain previews are separate
/// from base drafts. Finishing updates any base draft without saving its base.
/// Selection and base-edit commands finish pending gains first; CancelControlGain
/// restores the committed adjustment without saving.
/// Save persists the base EQ draft without republishing audio; Reset restores
/// the saved base with current named adjustments. Successful selection
/// persistence discards the previous draft.
///
/// # Failures and audio lifecycle
///
/// Failed persistence preserves saved state and the draft. A rejected edit or
/// reset preserves the previous draft. Failed gain saves discard the gain preview
/// and attempt to restore audio to the committed adjustment. Selection and discrete
/// named adjustments persist before audio publication, so DSP rejection can leave
/// new settings saved while the previous chain remains applied. Display both [`Self::action_error`] and
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
    gain_preview: Option<GainPreview>,
    presets: Vec<PresetSummary>,
    // Offline sessions deliberately use a synthetic device; audio-backed sessions
    // must not create profiles until a real route has established its identity.
    audio_requested: bool,
    // Present only after successful watcher installation.
    platform: Option<Box<dyn Platform>>,
    audio_error: Option<String>,
    publication_error: Option<String>,
    action_error: Option<SessionError>,
    audio_retry_needed: bool,
    audio_generation: u64,
    applied_chain: Option<Chain>,
    processing_enabled: bool,
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
        let presets = catalog.list(&PresetQuery::default());
        Ok(Self {
            persistence: Box::new(persistence),
            state,
            catalog: Box::new(catalog),
            device: DeviceId::try_new(SYSTEM_OUTPUT).expect("static device ID is valid"),
            device_name: UNAVAILABLE_DEVICE.into(),
            controller: None,
            draft: None,
            gain_preview: None,
            presets,
            audio_requested: false,
            platform: None,
            audio_error: None,
            publication_error: None,
            action_error: None,
            audio_retry_needed: false,
            audio_generation: 0,
            applied_chain: None,
            processing_enabled: true,
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
        self.audio_requested = true;
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

    pub fn device_id(&self) -> &DeviceId {
        &self.device
    }

    pub fn profiles(&self) -> &[Profile] {
        self.state
            .device(&self.device)
            .map_or(&[], DeviceProfiles::profiles)
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
        self.gain_preview
            .as_ref()
            .map(|preview| &preview.profile)
            .or(self.draft.as_ref())
            .or_else(|| self.selected_profile())
    }

    #[must_use]
    pub fn is_previewing_gain(&self) -> bool {
        self.gain_preview.is_some()
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

    #[must_use]
    pub fn processing_enabled(&self) -> bool {
        self.processing_enabled
    }

    #[must_use]
    pub fn can_create_profile(&self) -> bool {
        !self.audio_requested || self.controller.is_some()
    }

    /// Selection is persisted before audio publication; an audio error does not roll it back.
    pub fn execute(&mut self, command: Command) -> Result<(), SessionError> {
        if let Command::EditProfile { profile, edit } = &command {
            if self.selected_profile().map(Profile::id) != Some(profile) {
                let error = SessionError::StaleProfile(profile.clone());
                self.action_error = Some(error.clone());
                return Err(error);
            }
            if matches!(
                edit,
                ProfileEdit::FinishControlGain | ProfileEdit::CancelControlGain
            ) && self.gain_preview.is_none()
            {
                return Ok(());
            }
        }
        let refresh = matches!(command, Command::RefreshAudio);
        let result = self.execute_command(command);
        if !refresh {
            self.action_error = result.as_ref().err().cloned();
        }
        result
    }

    fn execute_command(&mut self, command: Command) -> Result<(), SessionError> {
        match &command {
            Command::CreateProfile { .. } if !self.can_create_profile() => {
                return Err(SessionError::DeviceUnavailable);
            }
            Command::CreateProfile { device, .. } | Command::SelectProfile { device, .. }
                if device != &self.device =>
            {
                return Err(SessionError::StaleDevice(device.clone()));
            }
            Command::SelectProfile { profile, .. }
                if !self.profiles().iter().any(|saved| saved.id() == profile) =>
            {
                return Err(SessionError::ProfileNotFound(profile.clone()));
            }
            _ => {}
        }
        if matches!(
            command,
            Command::CreateProfile { .. } | Command::SelectProfile { .. }
        ) {
            self.finish_control_gain()?;
        }
        match command {
            Command::SetProcessingEnabled(enabled) => {
                self.processing_enabled = enabled;
                if let Some(controller) = &self.controller {
                    controller.set_bypassed(!enabled);
                }
                Ok(())
            }
            Command::RefreshAudio => self.refresh_audio(),
            Command::CreateProfile { name, source, .. } => self.create_profile(name, source),
            Command::SelectProfile { profile, .. } => self.select_profile(profile),
            Command::EditProfile { edit, .. } => self.edit_profile(edit),
        }
    }

    fn edit_profile(&mut self, edit: ProfileEdit) -> Result<(), SessionError> {
        if matches!(
            edit,
            ProfileEdit::EditFilter { .. } | ProfileEdit::SaveDraft | ProfileEdit::ResetDraft
        ) {
            self.finish_control_gain()?;
        }
        match edit {
            ProfileEdit::EditFilter {
                filter,
                frequency,
                gain,
            } => self.edit_filter(filter, frequency, gain),
            ProfileEdit::SetControlGain { filter, gain } => self.set_control_gain(filter, gain),
            ProfileEdit::PreviewControlGain { filter, gain } => {
                self.preview_control_gain(filter, gain)
            }
            ProfileEdit::FinishControlGain => self.finish_control_gain(),
            ProfileEdit::CancelControlGain => self.cancel_control_gain(),
            ProfileEdit::SaveDraft => self.save_draft(),
            ProfileEdit::ResetDraft => self.reset_draft(),
        }
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

    fn create_profile(
        &mut self,
        name: ProfileName,
        source: ProfileSource,
    ) -> Result<(), SessionError> {
        if self
            .profiles()
            .iter()
            .any(|profile| profile.name() == &name)
        {
            return Err(SessionError::ProfileNameAlreadyExists(name));
        }
        let id = self.state.fresh_profile_id();
        let mut next = self.state.clone();
        let profile = match source {
            ProfileSource::Preset(preset_id) => {
                let preset = self
                    .catalog
                    .get(&preset_id)
                    .ok_or(SessionError::PresetNotFound(preset_id))?;
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
            ProfileSource::Flat => {
                Profile::new(id.clone(), name, flat_chain(), flat_controls(), None)
            }
        }
        .map_err(SessionError::InvalidProfile)?;
        if let Some(device) = next.device_mut(&self.device) {
            device.profiles.push(profile);
            device.selected = id;
        } else {
            next.insert(DeviceProfiles::new(self.device.clone(), vec![profile], id)?);
        }
        self.commit(next)?;
        self.draft = None;
        self.publish_active_chain()
    }

    fn select_profile(&mut self, id: ProfileId) -> Result<(), SessionError> {
        let mut next = self.state.clone();
        next.device_mut(&self.device)
            .ok_or(SessionError::NoSelectedProfile)?
            .selected = id;
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

    fn preview_control_gain(&mut self, filter: FilterId, gain: GainDb) -> Result<(), SessionError> {
        if self
            .gain_preview
            .as_ref()
            .is_some_and(|preview| preview.filter != filter)
        {
            self.finish_control_gain()?;
        }
        let mut profile = self
            .editing_profile()
            .cloned()
            .ok_or(SessionError::NoSelectedProfile)?;
        profile
            .adjust_filter_gain(filter, gain)
            .map_err(SessionError::InvalidProfile)?;
        self.publish(profile.effective_chain())?;
        self.gain_preview = Some(GainPreview {
            filter,
            gain,
            profile,
        });
        self.publication_error = None;
        Ok(())
    }

    fn finish_control_gain(&mut self) -> Result<(), SessionError> {
        let Some(preview) = &self.gain_preview else {
            return Ok(());
        };
        self.set_control_gain(preview.filter, preview.gain)
    }

    fn cancel_control_gain(&mut self) -> Result<(), SessionError> {
        if self.gain_preview.take().is_some() {
            self.publish_active_chain()?;
        }
        Ok(())
    }

    fn set_control_gain(&mut self, filter: FilterId, gain: GainDb) -> Result<(), SessionError> {
        self.gain_preview = None;
        let result = self.save_control_gain(filter, gain);
        if result.is_err() {
            // A failed save rolls audio back to the committed adjustment, keeping base edits.
            let _ = self.publish_active_chain();
        }
        result
    }

    fn save_control_gain(&mut self, filter: FilterId, gain: GainDb) -> Result<(), SessionError> {
        let id = self
            .selected_profile()
            .ok_or(SessionError::NoSelectedProfile)?
            .id();
        let mut next = self.state.clone();
        let saved = next
            .device_mut(&self.device)
            .expect("selected device exists")
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
            .device_mut(&self.device)
            .expect("selected device exists")
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
        if let Err(error) = self.finish_control_gain() {
            // Refresh is normally not an action, but a failed gain commit must
            // remain visible and must not prevent a later route retry.
            self.action_error = Some(error.clone());
            self.audio_retry_needed = self.platform.is_some();
            return Err(error);
        }
        let current_chain = self.active_chain();
        let state = &self.state;
        let current_device = &self.device;
        let chain_for_device = |device: &DeviceId| {
            if device == current_device {
                current_chain.clone()
            } else {
                state
                    .selected_profile(device)
                    .map_or_else(Chain::default, Profile::effective_chain)
            }
        };
        let platform = self.platform.as_mut().ok_or_else(|| {
            SessionError::Audio(
                self.audio_error
                    .clone()
                    .unwrap_or_else(|| "audio platform is unavailable".into()),
            )
        })?;
        match platform.refresh_default_output(&chain_for_device) {
            Ok(Some(connection)) => {
                let chain = chain_for_device(&connection.device_id);
                if connection.device_id != self.device {
                    self.draft = None;
                }
                self.device = connection.device_id;
                connection.controller.set_bypassed(!self.processing_enabled);
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

const FLAT_BANDS: [(&str, f64); 5] = [
    ("Bass", 100.0),
    ("Warmth", 300.0),
    ("Mids", 1000.0),
    ("Presence", 3000.0),
    ("Treble", 8000.0),
];

fn flat_chain() -> Chain {
    use crate::{Equalizer, Filter, FilterKind, FilterParameters, QualityFactor};
    Chain {
        equalizer: Equalizer {
            preamp: GainDb::default(),
            filters: FLAT_BANDS
                .iter()
                .enumerate()
                .map(|(index, (_, frequency))| Filter {
                    id: FilterId::try_new(index as u32 + 1).expect("static ID"),
                    parameters: FilterParameters {
                        kind: match index {
                            0 => FilterKind::LowShelf,
                            4 => FilterKind::HighShelf,
                            _ => FilterKind::Peaking,
                        },
                        frequency: FrequencyHz::try_new(*frequency).expect("static frequency"),
                        gain: GainDb::default(),
                        quality_factor: QualityFactor::try_new(std::f64::consts::FRAC_1_SQRT_2)
                            .expect("static Q"),
                    },
                })
                .collect(),
        },
    }
}

fn flat_controls() -> Vec<FilterControl> {
    FLAT_BANDS
        .iter()
        .enumerate()
        .map(|(index, (name, _))| {
            FilterControl::new(
                FilterId::try_new(index as u32 + 1).expect("static ID"),
                FilterControlName::try_new(*name).expect("static name"),
            )
        })
        .collect()
}

fn corrupt(message: String) -> SessionError {
    SessionError::PersistenceFailed(PersistenceError::Corrupt { message })
}

/// A rejected command or failed session operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    DeviceUnavailable,
    StaleDevice(DeviceId),
    StaleProfile(ProfileId),
    ProfileNotFound(ProfileId),
    NoSelectedProfile,
    Audio(String),
    PresetNotFound(PresetId),
    InvalidProfile(ProfileError),
    ProfileNameAlreadyExists(ProfileName),
    PersistenceFailed(PersistenceError),
}
