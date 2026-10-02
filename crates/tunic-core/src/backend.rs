//! Command handling and durable state orchestration.
//!
//! The backend validates restored state, reduces commands into candidate states,
//! and makes a candidate authoritative only after the store accepts it.

mod catalog;
mod command;
mod profile;
mod state;
mod store;

pub use catalog::{PresetCatalog, PresetQuery};
pub use command::{Command, ProfileSource};
pub use profile::{
    Attribution, FilterControl, FilterControlName, FilterControlNameError, Preset, PresetId,
    PresetIdError, PresetOrigin, PresetRevision, PresetRevisionError, PresetSummary, Profile,
    ProfileError, ProfileId, ProfileIdError, ProfileName, ProfileNameError, ProfileRevision,
};
pub use state::{DeviceId, DeviceIdError, DeviceProfileSelection, State};
pub use store::{MemoryStore, Store, StoreError};

use std::collections::HashSet;

/// Applies product commands and owns Tunic's authoritative state.
pub struct Backend {
    store: Box<dyn Store>,
    state: State,
    catalog: Box<dyn PresetCatalog>,
}

impl Backend {
    pub fn new(
        mut store: impl Store + 'static,
        catalog: impl PresetCatalog + 'static,
    ) -> Result<Self, BackendError> {
        let state = store
            .load()
            .map_err(BackendError::StoreFailed)?
            .unwrap_or_default();
        validate_state(&state)?;
        Ok(Self {
            store: Box::new(store),
            state,
            catalog: Box::new(catalog),
        })
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn catalog(&self) -> &dyn PresetCatalog {
        self.catalog.as_ref()
    }

    pub fn execute(&mut self, command: Command) -> Result<State, BackendError> {
        let next = apply(&self.state, self.catalog.as_ref(), command)?;
        if next != self.state {
            self.store.save(&next).map_err(BackendError::StoreFailed)?;
            self.state = next;
        }
        Ok(self.state.clone())
    }
}

fn apply(
    state: &State,
    catalog: &dyn PresetCatalog,
    command: Command,
) -> Result<State, BackendError> {
    let mut next = state.clone();
    match command {
        Command::CreateProfile { id, name, source } => {
            if state.profile(&id).is_some() {
                return Err(BackendError::ProfileAlreadyExists(id));
            }
            ensure_name_available(state, &name, None)?;
            let profile = match source {
                ProfileSource::Flat => Profile::new(id, name, Default::default(), Vec::new(), None),
                ProfileSource::Preset(preset_id) => {
                    let preset = catalog
                        .get(&preset_id)
                        .ok_or(BackendError::PresetNotFound(preset_id))?;
                    let origin = PresetOrigin {
                        id: preset.summary.id.clone(),
                        revision: preset.summary.revision.clone(),
                        attribution: preset.attribution.clone(),
                    };
                    Profile::new(id, name, preset.chain, preset.controls, Some(origin))
                }
                ProfileSource::Copy(source) => {
                    let source = state
                        .profile(&source)
                        .ok_or(BackendError::ProfileNotFound(source))?;
                    Profile::new(
                        id,
                        name,
                        source.base().clone(),
                        source.controls().to_vec(),
                        source.origin().cloned(),
                    )
                }
            }
            .map_err(BackendError::InvalidProfile)?;
            next.profiles.push(profile);
        }
        Command::RenameProfile { profile, name } => {
            let current = state
                .profile(&profile)
                .ok_or_else(|| BackendError::ProfileNotFound(profile.clone()))?;
            ensure_name_available(state, &name, Some(&profile))?;
            if current.name() != &name {
                let revision = current
                    .revision()
                    .next()
                    .ok_or_else(|| BackendError::ProfileRevisionExhausted(profile.clone()))?;
                let candidate = next
                    .profiles
                    .iter_mut()
                    .find(|candidate| candidate.id() == &profile)
                    .expect("profile existence checked above");
                candidate.rename(name);
                candidate.set_revision(revision);
            }
        }
        Command::DeleteProfile(profile) => {
            let Some(position) = next
                .profiles
                .iter()
                .position(|candidate| candidate.id() == &profile)
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
        Command::SaveProfile(mut draft) => {
            let profile = draft.id().clone();
            let profile_state = next
                .profiles
                .iter_mut()
                .find(|candidate| candidate.id() == &profile)
                .ok_or_else(|| BackendError::ProfileNotFound(profile.clone()))?;
            if profile_state.revision() != draft.revision() {
                return Err(BackendError::ProfileRevisionMismatch {
                    profile,
                    expected: draft.revision(),
                    actual: profile_state.revision(),
                });
            }
            draft.validate().map_err(BackendError::InvalidProfile)?;
            if profile_state != &draft {
                let revision = profile_state
                    .revision()
                    .next()
                    .ok_or_else(|| BackendError::ProfileRevisionExhausted(profile.clone()))?;
                draft.set_revision(revision);
                *profile_state = draft;
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
        .any(|profile| profile.name() == name && except != Some(profile.id()))
    {
        Err(BackendError::ProfileNameAlreadyExists(name.clone()))
    } else {
        Ok(())
    }
}

fn validate_state(state: &State) -> Result<(), BackendError> {
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

fn corrupt(message: String) -> BackendError {
    BackendError::StoreFailed(StoreError::Corrupt { message })
}

/// A rejected command or failed backend operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    ProfileNotFound(ProfileId),
    ProfileAlreadyExists(ProfileId),
    PresetNotFound(PresetId),
    InvalidProfile(ProfileError),
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
        Attribution, Chain, Command, DeviceId, DeviceProfileSelection, Equalizer, Filter,
        FilterControl, FilterControlName, FilterId, FilterKind, FilterParameters, FrequencyHz,
        GainDb, Preset, PresetCatalog, PresetId, PresetQuery, PresetRevision, PresetSummary,
        Profile, ProfileId, ProfileName, ProfileRevision, ProfileSource, QualityFactor, State,
        Store, StoreError,
    };

    impl PresetCatalog for Vec<Preset> {
        fn brands(&self) -> Vec<String> {
            self.iter().map(|p| p.summary.brand.clone()).collect()
        }
        fn models(&self, brand: &str) -> Vec<String> {
            self.iter()
                .filter(|p| p.summary.brand == brand)
                .map(|p| p.summary.model.clone())
                .collect()
        }
        fn list(&self, _query: &PresetQuery) -> Vec<PresetSummary> {
            self.iter().map(|p| p.summary.clone()).collect()
        }
        fn get(&self, id: &PresetId) -> Option<Preset> {
            self.iter().find(|p| &p.summary.id == id).cloned()
        }
    }

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

    fn filter(id: u32, frequency: f64, gain: f64, quality: f64) -> Filter {
        Filter {
            id: FilterId::try_new(id).unwrap(),
            parameters: FilterParameters {
                kind: FilterKind::Peaking,
                frequency: FrequencyHz::try_new(frequency).unwrap(),
                gain: GainDb::try_new(gain).unwrap(),
                quality_factor: QualityFactor::try_new(quality).unwrap(),
            },
        }
    }

    fn control(id: u32, name: &str) -> FilterControl {
        FilterControl::new(
            FilterId::try_new(id).unwrap(),
            FilterControlName::try_new(name).unwrap(),
        )
    }

    fn preset(id: &str, gain: f64) -> Preset {
        Preset {
            summary: PresetSummary {
                id: preset_id(id),
                revision: PresetRevision::try_new("1").unwrap(),
                brand: "Tunic".into(),
                model: "Reference".into(),
                variant: None,
                target: "Flat".into(),
            },
            attribution: Attribution {
                provider: "Tunic".into(),
                measurement_source: None,
                source_url: "https://example.com".into(),
            },
            chain: chain(gain),
            controls: Vec::new(),
        }
    }

    fn profile(id: &str, name: &str) -> Profile {
        Profile::new(
            profile_id(id),
            profile_name(name),
            Chain::default(),
            Vec::new(),
            None,
        )
        .unwrap()
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
                ProfileSource::Preset(reference.summary.id),
            ))
            .unwrap();
        backend
            .execute(create(
                "copy",
                "Copy",
                ProfileSource::Copy(profile_id("preset")),
            ))
            .unwrap();

        assert_eq!(backend.state().profiles[0].base(), &Chain::default());
        assert_eq!(backend.state().profiles[1].base(), &chain(-4.0));
        assert_eq!(backend.state().profiles[2].base(), &chain(-4.0));
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
            backend
                .state()
                .profile(&profile_id("second"))
                .unwrap()
                .name(),
            &profile_name("Renamed")
        );
        assert_eq!(
            backend
                .state()
                .profile(&profile_id("second"))
                .unwrap()
                .revision(),
            ProfileRevision(1)
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
                .id(),
            &profile_id("movies")
        );
        assert_eq!(
            backend
                .state()
                .selected_profile(&device_id("headphones"))
                .unwrap()
                .id(),
            &profile_id("music")
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
        let mut source = preset("reference", 0.0);
        source.chain.equalizer.filters = vec![filter(1, 100.0, 2.0, 0.7)];
        source.controls = vec![control(1, "Bass")];
        let mut backend = Backend::new(store.clone(), vec![source.clone()]).unwrap();
        backend
            .execute(create(
                "music",
                "Music",
                ProfileSource::Preset(source.summary.id),
            ))
            .unwrap();

        let stale = backend.state().profiles[0].clone();
        let mut draft = stale.clone();
        draft
            .adjust_filter_gain(FilterId::try_new(1).unwrap(), GainDb::try_new(4.0).unwrap())
            .unwrap();
        backend
            .execute(Command::SaveProfile(draft.clone()))
            .unwrap();
        let saves_after_change = store.save_count.load(Ordering::Relaxed);
        draft.set_revision(ProfileRevision(1));
        backend.execute(Command::SaveProfile(draft)).unwrap();

        assert_eq!(
            backend
                .state()
                .profile(&profile_id("music"))
                .unwrap()
                .revision(),
            ProfileRevision(1)
        );
        assert_eq!(store.save_count.load(Ordering::Relaxed), saves_after_change);
        assert_eq!(
            backend.execute(Command::SaveProfile(stale)),
            Err(BackendError::ProfileRevisionMismatch {
                profile: profile_id("music"),
                expected: ProfileRevision(0),
                actual: ProfileRevision(1),
            })
        );
    }

    #[test]
    fn profile_revision_overflow_is_rejected() {
        let mut saved = profile("music", "Music");
        saved.set_revision(ProfileRevision(u64::MAX));
        let store = TestStore::with_state(State {
            profiles: vec![saved.clone()],
            selections: Vec::new(),
        });
        let mut backend = Backend::new(store, Vec::new()).unwrap();
        saved.replace_base(chain(-1.0)).unwrap();

        assert_eq!(
            backend.execute(Command::SaveProfile(saved)),
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
                .name(),
            &profile_name("Music")
        );
    }

    #[test]
    fn rejects_corrupt_stored_state() {
        let first = profile("first", "First");
        let duplicate_name = profile("second", "First");
        let invalid_states = [
            State {
                profiles: vec![first.clone(), first.clone()],
                selections: Vec::new(),
            },
            State {
                profiles: vec![first.clone(), duplicate_name],
                selections: Vec::new(),
            },
            State {
                profiles: vec![first.clone()],
                selections: vec![
                    DeviceProfileSelection {
                        device: device_id("speakers"),
                        profile: first.id().clone(),
                    },
                    DeviceProfileSelection {
                        device: device_id("speakers"),
                        profile: first.id().clone(),
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
    fn exposes_the_catalog() {
        let backend = Backend::new(TestStore::default(), vec![preset("reference", -3.0)]).unwrap();
        assert_eq!(backend.catalog().brands(), vec!["Tunic"]);
    }

    #[test]
    fn base_chain_updates_and_gain_adjustments_compose_independently() {
        let mut source = preset("reference", -4.0);
        source.chain.equalizer.filters =
            vec![filter(3, 105.0, 5.5, 0.71), filter(4, 1_000.0, -2.0, 1.2)];
        source.controls = vec![control(3, "Bass")];
        let mut backend = Backend::new(TestStore::default(), vec![source.clone()]).unwrap();
        backend
            .execute(create(
                "profile",
                "Profile",
                ProfileSource::Preset(source.summary.id),
            ))
            .unwrap();

        let mut draft = backend.state().profiles[0].clone();
        draft
            .adjust_filter_gain(FilterId::try_new(3).unwrap(), GainDb::try_new(4.0).unwrap())
            .unwrap();
        let mut updated_base = source.chain;
        updated_base.equalizer.preamp = GainDb::try_new(-6.0).unwrap();
        updated_base.equalizer.filters[0].parameters.gain = GainDb::try_new(6.0).unwrap();
        updated_base.equalizer.filters[0].parameters.quality_factor =
            QualityFactor::try_new(1.5).unwrap();
        updated_base.equalizer.filters[1].parameters.gain = GainDb::try_new(-4.0).unwrap();
        draft.replace_base(updated_base.clone()).unwrap();
        backend.execute(Command::SaveProfile(draft)).unwrap();

        let saved = &backend.state().profiles[0];
        assert_eq!(saved.base(), &updated_base);
        let effective = saved.effective_chain();
        assert_eq!(
            effective.equalizer.filters[0]
                .parameters
                .frequency
                .into_inner(),
            105.0
        );
        assert_eq!(
            effective.equalizer.filters[0].parameters.gain.into_inner(),
            10.0
        );
        assert_eq!(
            effective.equalizer.filters[0].parameters.quality_factor,
            updated_base.equalizer.filters[0].parameters.quality_factor
        );
        assert_eq!(
            effective.equalizer.filters[1],
            updated_base.equalizer.filters[1]
        );
    }

    #[test]
    fn reset_adjustment_preserves_the_exposed_control() {
        let mut source = preset("reference", 0.0);
        source.chain.equalizer.filters = vec![filter(1, 100.0, 2.0, 0.7)];
        source.controls = vec![control(1, "Bass")];
        let store = TestStore::default();
        let mut backend = Backend::new(store.clone(), vec![source.clone()]).unwrap();
        backend
            .execute(create(
                "profile",
                "Profile",
                ProfileSource::Preset(source.summary.id),
            ))
            .unwrap();
        let mut draft = backend.state().profiles[0].clone();
        draft
            .adjust_filter_gain(FilterId::try_new(1).unwrap(), GainDb::try_new(8.0).unwrap())
            .unwrap();
        draft
            .reset_adjustment(FilterId::try_new(1).unwrap())
            .unwrap();
        backend.execute(Command::SaveProfile(draft)).unwrap();

        let saved = &backend.state().profiles[0];
        assert_eq!(saved.controls().len(), 1);
        assert_eq!(saved.controls()[0].gain_adjustment(), GainDb::default());
        assert_eq!(saved.effective_chain(), source.chain);
    }

    #[test]
    fn profile_edits_are_atomic_and_saved_profiles_need_no_catalog_entry() {
        let mut source = preset("reference", -4.0);
        source.chain.equalizer.filters = vec![filter(3, 105.0, 5.5, 0.71)];
        source.controls = vec![control(3, "Bass")];
        let store = TestStore::default();
        let mut backend = Backend::new(store.clone(), vec![source.clone()]).unwrap();
        backend
            .execute(create(
                "profile",
                "Profile",
                ProfileSource::Preset(source.summary.id.clone()),
            ))
            .unwrap();
        let original = backend.state().clone();
        let origin = original.profiles[0].origin().unwrap();
        assert_eq!(origin.id, source.summary.id);
        assert_eq!(origin.revision, source.summary.revision);
        assert_eq!(origin.attribution, source.attribution);

        let mut draft = original.profiles[0].clone();
        draft
            .adjust_filter_gain(FilterId::try_new(3).unwrap(), GainDb::try_new(8.0).unwrap())
            .unwrap();
        store.fail_saves.store(true, Ordering::Relaxed);
        assert!(matches!(
            backend.execute(Command::SaveProfile(draft)),
            Err(BackendError::StoreFailed(_))
        ));
        assert_eq!(backend.state(), &original);
        assert_eq!(store.state(), Some(original.clone()));

        let mut restored =
            Backend::new(TestStore::with_state(original), Vec::<Preset>::new()).unwrap();
        restored
            .execute(create(
                "copy",
                "Copy",
                ProfileSource::Copy(profile_id("profile")),
            ))
            .unwrap();
        assert_eq!(
            restored.state().profiles[1].base(),
            restored.state().profiles[0].base()
        );
        assert_eq!(
            restored.state().profiles[1].controls(),
            restored.state().profiles[0].controls()
        );
        assert_eq!(
            restored.state().profiles[1].origin(),
            restored.state().profiles[0].origin()
        );
    }

    #[test]
    fn catalog_is_loaded_only_for_preset_creation_and_missing_ids_do_not_save() {
        struct EmptyCatalog;
        impl PresetCatalog for EmptyCatalog {
            fn brands(&self) -> Vec<String> {
                panic!("must not enumerate catalog")
            }
            fn models(&self, _: &str) -> Vec<String> {
                panic!("must not enumerate catalog")
            }
            fn list(&self, _: &PresetQuery) -> Vec<PresetSummary> {
                panic!("must not enumerate catalog")
            }
            fn get(&self, _: &PresetId) -> Option<Preset> {
                None
            }
        }
        let store = TestStore::default();
        let mut backend = Backend::new(store.clone(), EmptyCatalog).unwrap();
        backend
            .execute(create("flat", "Flat", ProfileSource::Flat))
            .unwrap();
        let before = backend.state().clone();
        assert!(matches!(
            backend.execute(create(
                "preset",
                "Preset",
                ProfileSource::Preset(preset_id("reference"))
            )),
            Err(BackendError::PresetNotFound(id)) if id == preset_id("reference")
        ));
        assert_eq!(backend.state(), &before);
        assert_eq!(store.save_count.load(Ordering::Relaxed), 1);
    }
}
