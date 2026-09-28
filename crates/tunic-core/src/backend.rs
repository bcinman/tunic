//! Command handling and durable state orchestration.
//!
//! The backend validates restored state, reduces commands into candidate states,
//! and makes a candidate authoritative only after the store accepts it.

use std::collections::HashSet;

use crate::{
    Command, DeviceProfileSelection, Preset, PresetId, Profile, ProfileId, ProfileName,
    ProfileRevision, ProfileSource, State, Store, StoreError,
};

/// Applies product commands and owns Tunic's authoritative state.
pub struct Backend {
    store: Box<dyn Store>,
    state: State,
    presets: Vec<Preset>,
}

impl Backend {
    pub fn new(
        mut store: impl Store + 'static,
        presets: Vec<Preset>,
    ) -> Result<Self, BackendError> {
        validate_presets(&presets)?;
        let state = store
            .load()
            .map_err(BackendError::StoreFailed)?
            .unwrap_or_default();
        validate_state(&state)?;
        Ok(Self {
            store: Box::new(store),
            state,
            presets,
        })
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn execute(&mut self, command: Command) -> Result<State, BackendError> {
        let next = apply(&self.state, &self.presets, command)?;
        if next != self.state {
            self.store.save(&next).map_err(BackendError::StoreFailed)?;
            self.state = next;
        }
        Ok(self.state.clone())
    }
}

fn apply(state: &State, presets: &[Preset], command: Command) -> Result<State, BackendError> {
    let mut next = state.clone();
    match command {
        Command::CreateProfile { id, name, source } => {
            if state.profile(&id).is_some() {
                return Err(BackendError::ProfileAlreadyExists(id));
            }
            ensure_name_available(state, &name, None)?;
            let chain = match source {
                ProfileSource::Flat => Default::default(),
                ProfileSource::Preset(preset) => presets
                    .iter()
                    .find(|candidate| candidate.id == preset)
                    .ok_or(BackendError::PresetNotFound(preset))?
                    .chain
                    .clone(),
                ProfileSource::Copy(profile) => state
                    .profile(&profile)
                    .ok_or(BackendError::ProfileNotFound(profile))?
                    .chain
                    .clone(),
            };
            next.profiles.push(Profile {
                id,
                name,
                chain,
                revision: ProfileRevision::default(),
            });
        }
        Command::RenameProfile { profile, name } => {
            if state.profile(&profile).is_none() {
                return Err(BackendError::ProfileNotFound(profile));
            }
            ensure_name_available(state, &name, Some(&profile))?;
            next.profiles
                .iter_mut()
                .find(|candidate| candidate.id == profile)
                .expect("profile existence checked above")
                .name = name;
        }
        Command::DeleteProfile(profile) => {
            let Some(position) = next
                .profiles
                .iter()
                .position(|candidate| candidate.id == profile)
            else {
                return Err(BackendError::ProfileNotFound(profile));
            };
            next.profiles.remove(position);
            next.selections
                .retain(|selection| selection.profile != profile);
        }
        Command::SelectProfile { device, profile } => {
            if state.profile(&profile).is_none() {
                return Err(BackendError::ProfileNotFound(profile));
            }
            if let Some(selection) = next
                .selections
                .iter_mut()
                .find(|selection| selection.device == device)
            {
                selection.profile = profile;
            } else {
                next.selections
                    .push(DeviceProfileSelection { device, profile });
            }
        }
        Command::ClearProfile(device) => {
            next.selections
                .retain(|selection| selection.device != device);
        }
        Command::UpdateProfile {
            profile,
            chain,
            expected_revision,
        } => {
            let profile_state = next
                .profiles
                .iter_mut()
                .find(|candidate| candidate.id == profile)
                .ok_or_else(|| BackendError::ProfileNotFound(profile.clone()))?;
            if profile_state.revision != expected_revision {
                return Err(BackendError::ProfileRevisionMismatch {
                    profile,
                    expected: expected_revision,
                    actual: profile_state.revision,
                });
            }
            if profile_state.chain != chain {
                profile_state.revision = profile_state
                    .revision
                    .next()
                    .ok_or_else(|| BackendError::ProfileRevisionExhausted(profile.clone()))?;
                profile_state.chain = chain;
            }
        }
    }
    Ok(next)
}

fn ensure_name_available(
    state: &State,
    name: &ProfileName,
    except: Option<&ProfileId>,
) -> Result<(), BackendError> {
    if state
        .profiles
        .iter()
        .any(|profile| &profile.name == name && except != Some(&profile.id))
    {
        Err(BackendError::ProfileNameAlreadyExists(name.clone()))
    } else {
        Ok(())
    }
}

fn validate_presets(presets: &[Preset]) -> Result<(), BackendError> {
    let mut ids = HashSet::new();
    for preset in presets {
        if !ids.insert(preset.id.clone()) {
            return Err(BackendError::PresetAlreadyExists(preset.id.clone()));
        }
    }
    Ok(())
}

fn validate_state(state: &State) -> Result<(), BackendError> {
    let mut profile_ids = HashSet::new();
    let mut profile_names = HashSet::new();
    for profile in &state.profiles {
        if !profile_ids.insert(profile.id.clone()) {
            return Err(corrupt(format!("duplicate profile id '{}'", profile.id)));
        }
        if !profile_names.insert(profile.name.clone()) {
            return Err(corrupt(format!(
                "duplicate profile name '{}'",
                profile.name
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

fn corrupt(message: String) -> BackendError {
    BackendError::StoreFailed(StoreError::Corrupt { message })
}

/// A rejected command or failed backend operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    ProfileNotFound(ProfileId),
    ProfileAlreadyExists(ProfileId),
    PresetNotFound(PresetId),
    PresetAlreadyExists(PresetId),
    ProfileNameAlreadyExists(ProfileName),
    ProfileRevisionMismatch {
        profile: ProfileId,
        expected: ProfileRevision,
        actual: ProfileRevision,
    },
    ProfileRevisionExhausted(ProfileId),
    StoreFailed(StoreError),
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use super::{Backend, BackendError};
    use crate::{
        Chain, Command, DeviceId, DeviceProfileSelection, Equalizer, GainDb, Preset, PresetId,
        Profile, ProfileId, ProfileName, ProfileRevision, ProfileSource, State, Store, StoreError,
    };

    #[derive(Clone, Default)]
    struct TestStore {
        state: Arc<Mutex<Option<State>>>,
        fail_saves: Arc<AtomicBool>,
        save_count: Arc<AtomicUsize>,
    }

    impl TestStore {
        fn with_state(state: State) -> Self {
            Self {
                state: Arc::new(Mutex::new(Some(state))),
                ..Self::default()
            }
        }

        fn state(&self) -> Option<State> {
            self.state.lock().unwrap().clone()
        }
    }

    impl Store for TestStore {
        fn load(&mut self) -> Result<Option<State>, StoreError> {
            Ok(self.state())
        }

        fn save(&mut self, state: &State) -> Result<(), StoreError> {
            if self.fail_saves.load(Ordering::Relaxed) {
                return Err(StoreError::Unavailable {
                    message: "save failed".into(),
                });
            }
            *self.state.lock().unwrap() = Some(state.clone());
            self.save_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn profile_id(value: &str) -> ProfileId {
        ProfileId::try_new(value).unwrap()
    }

    fn preset_id(value: &str) -> PresetId {
        PresetId::try_new(value).unwrap()
    }

    fn profile_name(value: &str) -> ProfileName {
        ProfileName::try_new(value).unwrap()
    }

    fn device_id(value: &str) -> DeviceId {
        DeviceId::try_new(value).unwrap()
    }

    fn chain(gain: f64) -> Chain {
        Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(gain).unwrap(),
                filters: Vec::new(),
            },
        }
    }

    fn preset(id: &str, gain: f64) -> Preset {
        Preset {
            id: preset_id(id),
            brand: "Tunic".into(),
            model: "Reference".into(),
            chain: chain(gain),
        }
    }

    fn profile(id: &str, name: &str) -> Profile {
        Profile {
            id: profile_id(id),
            name: profile_name(name),
            chain: Chain::default(),
            revision: ProfileRevision::default(),
        }
    }

    fn create(id: &str, name: &str, source: ProfileSource) -> Command {
        Command::CreateProfile {
            id: profile_id(id),
            name: profile_name(name),
            source,
        }
    }

    #[test]
    fn creates_flat_preset_and_copied_profiles_and_persists_the_snapshot() {
        let store = TestStore::default();
        let reference = preset("reference", -4.0);
        let mut backend = Backend::new(store.clone(), vec![reference.clone()]).unwrap();

        backend
            .execute(create("flat", "Flat", ProfileSource::Flat))
            .unwrap();
        backend
            .execute(create(
                "preset",
                "Preset",
                ProfileSource::Preset(reference.id),
            ))
            .unwrap();
        backend
            .execute(create(
                "copy",
                "Copy",
                ProfileSource::Copy(profile_id("preset")),
            ))
            .unwrap();

        assert_eq!(backend.state().profiles[0].chain, Chain::default());
        assert_eq!(backend.state().profiles[1].chain, chain(-4.0));
        assert_eq!(backend.state().profiles[2].chain, chain(-4.0));
        assert_eq!(store.state().as_ref(), Some(backend.state()));
        assert_eq!(store.save_count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn profile_ids_and_names_are_globally_unique() {
        let mut backend = Backend::new(TestStore::default(), Vec::new()).unwrap();
        backend
            .execute(create("first", "First", ProfileSource::Flat))
            .unwrap();
        backend
            .execute(create("second", "Second", ProfileSource::Flat))
            .unwrap();

        assert_eq!(
            backend.execute(create("first", "Third", ProfileSource::Flat)),
            Err(BackendError::ProfileAlreadyExists(profile_id("first")))
        );
        assert_eq!(
            backend.execute(create("third", "First", ProfileSource::Flat)),
            Err(BackendError::ProfileNameAlreadyExists(profile_name(
                "First"
            )))
        );
        assert_eq!(
            backend.execute(Command::RenameProfile {
                profile: profile_id("second"),
                name: profile_name("First"),
            }),
            Err(BackendError::ProfileNameAlreadyExists(profile_name(
                "First"
            )))
        );

        backend
            .execute(Command::RenameProfile {
                profile: profile_id("second"),
                name: profile_name("Renamed"),
            })
            .unwrap();
        assert_eq!(
            backend.state().profile(&profile_id("second")).unwrap().name,
            profile_name("Renamed")
        );
    }

    #[test]
    fn profiles_can_be_selected_reused_replaced_and_cleared() {
        let mut backend = Backend::new(TestStore::default(), Vec::new()).unwrap();
        backend
            .execute(create("music", "Music", ProfileSource::Flat))
            .unwrap();
        backend
            .execute(create("movies", "Movies", ProfileSource::Flat))
            .unwrap();

        for device in ["speakers", "headphones"] {
            backend
                .execute(Command::SelectProfile {
                    device: device_id(device),
                    profile: profile_id("music"),
                })
                .unwrap();
        }
        backend
            .execute(Command::SelectProfile {
                device: device_id("speakers"),
                profile: profile_id("movies"),
            })
            .unwrap();

        assert_eq!(
            backend
                .state()
                .selected_profile(&device_id("speakers"))
                .unwrap()
                .id,
            profile_id("movies")
        );
        assert_eq!(
            backend
                .state()
                .selected_profile(&device_id("headphones"))
                .unwrap()
                .id,
            profile_id("music")
        );

        backend
            .execute(Command::ClearProfile(device_id("speakers")))
            .unwrap();
        assert!(
            backend
                .state()
                .selected_profile(&device_id("speakers"))
                .is_none()
        );
    }

    #[test]
    fn deleting_a_profile_clears_every_selection_of_it() {
        let mut backend = Backend::new(TestStore::default(), Vec::new()).unwrap();
        backend
            .execute(create("music", "Music", ProfileSource::Flat))
            .unwrap();
        for device in ["speakers", "headphones"] {
            backend
                .execute(Command::SelectProfile {
                    device: device_id(device),
                    profile: profile_id("music"),
                })
                .unwrap();
        }

        backend
            .execute(Command::DeleteProfile(profile_id("music")))
            .unwrap();

        assert!(backend.state().profiles.is_empty());
        assert!(backend.state().selections.is_empty());
    }

    #[test]
    fn updates_require_the_current_revision_and_only_changes_advance_it() {
        let store = TestStore::default();
        let mut backend = Backend::new(store.clone(), Vec::new()).unwrap();
        backend
            .execute(create("music", "Music", ProfileSource::Flat))
            .unwrap();

        backend
            .execute(Command::UpdateProfile {
                profile: profile_id("music"),
                chain: chain(-3.0),
                expected_revision: ProfileRevision(0),
            })
            .unwrap();
        let saves_after_change = store.save_count.load(Ordering::Relaxed);
        backend
            .execute(Command::UpdateProfile {
                profile: profile_id("music"),
                chain: chain(-3.0),
                expected_revision: ProfileRevision(1),
            })
            .unwrap();

        assert_eq!(
            backend
                .state()
                .profile(&profile_id("music"))
                .unwrap()
                .revision,
            ProfileRevision(1)
        );
        assert_eq!(store.save_count.load(Ordering::Relaxed), saves_after_change);
        assert_eq!(
            backend.execute(Command::UpdateProfile {
                profile: profile_id("music"),
                chain: chain(-6.0),
                expected_revision: ProfileRevision(0),
            }),
            Err(BackendError::ProfileRevisionMismatch {
                profile: profile_id("music"),
                expected: ProfileRevision(0),
                actual: ProfileRevision(1),
            })
        );
    }

    #[test]
    fn profile_revision_overflow_is_rejected() {
        let store = TestStore::with_state(State {
            profiles: vec![Profile {
                revision: ProfileRevision(u64::MAX),
                ..profile("music", "Music")
            }],
            selections: Vec::new(),
        });
        let mut backend = Backend::new(store, Vec::new()).unwrap();

        assert_eq!(
            backend.execute(Command::UpdateProfile {
                profile: profile_id("music"),
                chain: chain(-3.0),
                expected_revision: ProfileRevision(u64::MAX),
            }),
            Err(BackendError::ProfileRevisionExhausted(profile_id("music")))
        );
    }

    #[test]
    fn a_failed_save_does_not_publish_the_candidate_state() {
        let store = TestStore::default();
        store.fail_saves.store(true, Ordering::Relaxed);
        let mut backend = Backend::new(store, Vec::new()).unwrap();

        assert_eq!(
            backend.execute(create("music", "Music", ProfileSource::Flat)),
            Err(BackendError::StoreFailed(StoreError::Unavailable {
                message: "save failed".into(),
            }))
        );
        assert!(backend.state().profiles.is_empty());
    }

    #[test]
    fn restores_valid_state() {
        let state = State {
            profiles: vec![profile("music", "Music")],
            selections: vec![DeviceProfileSelection {
                device: device_id("speakers"),
                profile: profile_id("music"),
            }],
        };

        let backend = Backend::new(TestStore::with_state(state), Vec::new()).unwrap();

        assert_eq!(
            backend
                .state()
                .selected_profile(&device_id("speakers"))
                .unwrap()
                .name,
            profile_name("Music")
        );
    }

    #[test]
    fn rejects_corrupt_stored_state() {
        let first = profile("first", "First");
        let second = profile("second", "Second");
        let invalid_states = [
            State {
                profiles: vec![first.clone(), Profile { ..first.clone() }],
                selections: Vec::new(),
            },
            State {
                profiles: vec![
                    first.clone(),
                    Profile {
                        name: first.name.clone(),
                        ..second.clone()
                    },
                ],
                selections: Vec::new(),
            },
            State {
                profiles: vec![first.clone()],
                selections: vec![
                    DeviceProfileSelection {
                        device: device_id("speakers"),
                        profile: first.id.clone(),
                    },
                    DeviceProfileSelection {
                        device: device_id("speakers"),
                        profile: first.id.clone(),
                    },
                ],
            },
            State {
                profiles: vec![first],
                selections: vec![DeviceProfileSelection {
                    device: device_id("speakers"),
                    profile: profile_id("missing"),
                }],
            },
        ];

        for state in invalid_states {
            assert!(matches!(
                Backend::new(TestStore::with_state(state), Vec::new()),
                Err(BackendError::StoreFailed(StoreError::Corrupt { .. }))
            ));
        }
    }

    #[test]
    fn rejects_duplicate_preset_ids() {
        let result = Backend::new(
            TestStore::default(),
            vec![preset("reference", -3.0), preset("reference", -6.0)],
        );

        assert!(matches!(
            result,
            Err(BackendError::PresetAlreadyExists(id)) if id == preset_id("reference")
        ));
    }
}
