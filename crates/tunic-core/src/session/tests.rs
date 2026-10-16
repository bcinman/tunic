use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use super::*;
use crate::{
    AudioFormat, Equalizer, Filter, FilterKind, FilterParameters, Processor, QualityFactor,
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
    fn list(&self, _: &PresetQuery) -> Vec<PresetSummary> {
        self.iter().map(|p| p.summary.clone()).collect()
    }
    fn get(&self, id: &PresetId) -> Option<Preset> {
        self.iter().find(|p| &p.summary.id == id).cloned()
    }
}

#[derive(Clone, Default)]
struct TestPersistence {
    state: Arc<Mutex<Option<State>>>,
    fail: Arc<AtomicBool>,
    saves: Arc<AtomicUsize>,
}

impl Persistence for TestPersistence {
    fn load(&mut self) -> Result<Option<State>, PersistenceError> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn save(&mut self, state: &State) -> Result<(), PersistenceError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(PersistenceError::Unavailable {
                message: "save failed".into(),
            });
        }
        *self.state.lock().unwrap() = Some(state.clone());
        self.saves.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn preset() -> Preset {
    Preset {
        summary: PresetSummary {
            id: PresetId::try_new("reference").unwrap(),
            revision: PresetRevision::try_new("source-2").unwrap(),
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
        chain: Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(-3.0).unwrap(),
                filters: vec![Filter {
                    id: FilterId::try_new(3).unwrap(),
                    parameters: FilterParameters {
                        kind: FilterKind::Peaking,
                        frequency: FrequencyHz::try_new(1000.0).unwrap(),
                        gain: GainDb::try_new(2.0).unwrap(),
                        quality_factor: QualityFactor::try_new(0.7).unwrap(),
                    },
                }],
            },
        },
        controls: vec![FilterControl::new(
            FilterId::try_new(3).unwrap(),
            FilterControlName::try_new("Tone").unwrap(),
        )],
    }
}

fn use_preset() -> Command {
    create_preset("Reference", "reference")
}
fn create_preset(name: &str, id: &str) -> Command {
    Command::CreateProfile {
        device: DeviceId::try_new(SYSTEM_OUTPUT).unwrap(),
        name: ProfileName::try_new(name).unwrap(),
        source: ProfileSource::Preset(PresetId::try_new(id).unwrap()),
    }
}
fn flat() -> Command {
    Command::CreateProfile {
        device: DeviceId::try_new(SYSTEM_OUTPUT).unwrap(),
        name: ProfileName::try_new("Flat").unwrap(),
        source: ProfileSource::Flat,
    }
}
fn select_first(session: &Session) -> Command {
    Command::SelectProfile {
        device: session.device_id().clone(),
        profile: session.profiles()[0].id().clone(),
    }
}
fn adjust(gain: f64) -> ProfileEdit {
    ProfileEdit::SetControlGain {
        filter: FilterId::try_new(3).unwrap(),
        gain: GainDb::try_new(gain).unwrap(),
    }
}
fn edit(frequency: f64, gain: f64) -> ProfileEdit {
    ProfileEdit::EditFilter {
        filter: FilterId::try_new(3).unwrap(),
        frequency: FrequencyHz::try_new(frequency).unwrap(),
        gain: GainDb::try_new(gain).unwrap(),
    }
}

impl Session {
    fn execute_edit(&mut self, edit: ProfileEdit) -> Result<(), SessionError> {
        self.execute(Command::EditProfile {
            profile: self.selected_profile().unwrap().id().clone(),
            edit,
        })
    }
}

struct TestPlatform {
    watch_error: Option<String>,
    steps: VecDeque<Result<Option<f64>, String>>,
    chains: Arc<Mutex<Vec<Chain>>>,
    processor: Option<Processor>,
}

fn platform(steps: impl IntoIterator<Item = Result<Option<f64>, String>>) -> TestPlatform {
    TestPlatform {
        watch_error: None,
        steps: steps.into_iter().collect(),
        chains: Arc::default(),
        processor: None,
    }
}

impl Platform for TestPlatform {
    fn watch_default_output(&mut self, _: ChangeHandler) -> Result<(), String> {
        self.watch_error.clone().map_or(Ok(()), Err)
    }
    fn refresh_default_output(
        &mut self,
        chain_for_device: &dyn Fn(&DeviceId) -> Chain,
    ) -> Result<Option<Connection>, String> {
        let device_id = DeviceId::try_new(SYSTEM_OUTPUT).unwrap();
        let chain = chain_for_device(&device_id);
        self.chains.lock().unwrap().push(chain.clone());
        match self.steps.pop_front().expect("unexpected route refresh") {
            Ok(Some(rate)) => {
                let (processor, controller) = Processor::new(
                    AudioFormat {
                        sample_rate: SampleRateHz::try_new(rate).unwrap(),
                        maximum_frame_count: NonZeroUsize::new(64).unwrap(),
                    },
                    chain.clone(),
                    false,
                )
                .map_err(|error| format!("{error:?}"))?;
                self.processor = Some(processor);
                Ok(Some(Connection {
                    device_id,
                    device_name: format!("Output {rate}"),
                    controller,
                }))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.processor = None;
                Err(error)
            }
        }
    }
}

#[test]
fn processing_switch_bypasses_audio_and_survives_route_replacement() {
    struct Routes(VecDeque<Controller>);
    impl Platform for Routes {
        fn watch_default_output(&mut self, _: ChangeHandler) -> Result<(), String> {
            Ok(())
        }
        fn refresh_default_output(
            &mut self,
            chain_for_device: &dyn Fn(&DeviceId) -> Chain,
        ) -> Result<Option<Connection>, String> {
            let device_id = DeviceId::try_new(SYSTEM_OUTPUT).unwrap();
            let chain = chain_for_device(&device_id);
            let controller = self.0.pop_front().unwrap();
            controller.set_chain(chain.clone()).unwrap();
            Ok(Some(Connection {
                device_id,
                device_name: "Test output".into(),
                controller,
            }))
        }
    }
    let chain = preset().chain;
    let format = AudioFormat {
        sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
        maximum_frame_count: NonZeroUsize::new(64).unwrap(),
    };
    let (mut first, controller) = Processor::new(format, chain.clone(), false).unwrap();
    let (mut second, replacement) = Processor::new(format, chain, false).unwrap();
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    assert!(session.processing_enabled());
    let mut session = session.with_audio(Routes([controller, replacement].into()), Arc::new(|| {}));
    session
        .execute(Command::SetProcessingEnabled(false))
        .unwrap();
    assert!(!session.processing_enabled());
    let mut samples = [0.25, -0.5];
    first.process(&mut samples);
    assert_eq!(samples, [0.25, -0.5]);
    session.execute_edit(adjust(5.0)).unwrap();
    let desired = session.active_chain();
    session.execute(Command::RefreshAudio).unwrap();
    second.process(&mut samples);
    assert_eq!(samples, [0.25, -0.5]);
    session
        .execute(Command::SetProcessingEnabled(true))
        .unwrap();
    assert!(session.processing_enabled());
    assert_eq!(session.active_chain(), desired);
    second.process(&mut samples);
    assert_ne!(samples, [0.25, -0.5]);
}

#[test]
fn creation_selection_and_provenance_are_saved_atomically() {
    let store = TestPersistence::default();
    let source = preset();
    let mut session = Session::new(store.clone(), vec![source.clone()]).unwrap();
    assert_eq!(session.presets(), vec![source.summary.clone()]);
    store.fail.store(true, Ordering::Relaxed);
    assert!(matches!(
        session.execute(use_preset()),
        Err(SessionError::PersistenceFailed(_))
    ));
    assert_eq!(session.state(), &State::default());
    assert!(store.state.lock().unwrap().is_none());
    store.fail.store(false, Ordering::Relaxed);
    session.execute(use_preset()).unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
    assert_eq!(session.active_chain(), source.chain);
    let origin = session.selected_profile().unwrap().origin().unwrap();
    assert_eq!(origin.id, source.summary.id);
    assert_eq!(origin.revision, source.summary.revision);
    assert_eq!(origin.attribution, source.attribution);
    assert_eq!(store.state.lock().unwrap().as_ref(), Some(session.state()));
    session.execute(select_first(&session)).unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
    session.execute(flat()).unwrap();
    assert_eq!(session.active_chain().equalizer.filters.len(), 5);
    assert_eq!(session.profiles().len(), 2);
}

#[test]
fn drafts_compose_base_and_adjustment_and_save_only_changes() {
    let store = TestPersistence::default();
    let mut session = Session::new(store.clone(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    session.execute_edit(adjust(4.0)).unwrap();
    assert!(session.draft().is_none());
    let saved = session.selected_profile().unwrap().clone();
    session.execute_edit(edit(800.0, 5.0)).unwrap();
    assert_eq!(
        session.active_chain().equalizer.filters[0]
            .parameters
            .gain
            .into_inner(),
        9.0
    );
    assert_eq!(
        session.base_chain().equalizer.filters[0]
            .parameters
            .frequency
            .into_inner(),
        800.0
    );
    assert_eq!(session.selected_profile(), Some(&saved));
    let draft = session.draft().unwrap().clone();
    store.fail.store(true, Ordering::Relaxed);
    assert!(session.execute_edit(ProfileEdit::SaveDraft).is_err());
    assert_eq!(session.selected_profile(), Some(&saved));
    assert_eq!(session.draft(), Some(&draft));
    store.fail.store(false, Ordering::Relaxed);
    session.execute_edit(ProfileEdit::SaveDraft).unwrap();
    assert_eq!(session.selected_profile(), Some(&draft));
    assert!(session.draft().is_none());
    assert_eq!(store.saves.load(Ordering::Relaxed), 3);
    session.execute_edit(adjust(4.0)).unwrap();
    session.execute_edit(ProfileEdit::SaveDraft).unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 3);
    session.execute_edit(edit(900.0, 1.0)).unwrap();
    session.execute_edit(adjust(-2.0)).unwrap();
    let saved = session.selected_profile().unwrap().clone();
    assert_eq!(saved.base(), draft.base());
    assert_eq!(saved.controls()[0].gain_adjustment().into_inner(), -2.0);
    assert_eq!(
        session.active_chain().equalizer.filters[0]
            .parameters
            .gain
            .into_inner(),
        -1.0
    );
    let restored = Session::new(store, Vec::<Preset>::new()).unwrap();
    assert_eq!(restored.selected_profile(), Some(&saved));
    session.execute_edit(ProfileEdit::ResetDraft).unwrap();
    assert_eq!(session.selected_profile(), Some(&saved));
    assert_eq!(session.active_chain(), saved.effective_chain());
    assert!(session.draft().is_none());
}

#[test]
fn failed_adjustment_save_preserves_saved_draft_and_audio_state() {
    let store = TestPersistence::default();
    let mut session = Session::new(store.clone(), vec![preset()])
        .unwrap()
        .with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
    session.execute(use_preset()).unwrap();
    session.execute_edit(edit(800.0, 5.0)).unwrap();
    let saved = session.state().clone();
    let draft = session.draft().cloned();
    let applied = session.applied_chain().cloned();
    store.fail.store(true, Ordering::Relaxed);
    assert!(matches!(
        session.execute_edit(adjust(-2.0)),
        Err(SessionError::PersistenceFailed(_))
    ));
    assert_eq!(session.state(), &saved);
    assert_eq!(session.draft(), draft.as_ref());
    assert_eq!(session.applied_chain(), applied.as_ref());
    assert_eq!(store.state.lock().unwrap().as_ref(), Some(&saved));
    store.fail.store(false, Ordering::Relaxed);
    session.execute_edit(adjust(-2.0)).unwrap();
    session.execute_edit(ProfileEdit::SaveDraft).unwrap();
    assert_eq!(
        session.selected_profile().unwrap().controls()[0]
            .gain_adjustment()
            .into_inner(),
        -2.0
    );
    assert_eq!(
        session.active_chain().equalizer.filters[0]
            .parameters
            .gain
            .into_inner(),
        3.0
    );
    assert_eq!(session.applied_chain(), Some(&session.active_chain()));
}

#[test]
fn gain_drag_previews_without_saving_and_commits_only_the_adjustment() {
    let store = TestPersistence::default();
    let mut session = Session::new(store.clone(), vec![preset()])
        .unwrap()
        .with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
    session.execute(use_preset()).unwrap();
    session.execute_edit(edit(800.0, 5.0)).unwrap();
    let saved = session.state().clone();
    for step in 0..120 {
        session
            .execute_edit(ProfileEdit::PreviewControlGain {
                filter: FilterId::try_new(3).unwrap(),
                gain: GainDb::try_new(f64::from(step) / 10.0 - 6.0).unwrap(),
            })
            .unwrap();
        assert_eq!(session.state(), &saved);
        assert_eq!(session.applied_chain(), Some(&session.active_chain()));
        assert!(session.is_previewing_gain());
    }
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
    let preview = session.active_chain();
    session
        .execute_edit(ProfileEdit::FinishControlGain)
        .unwrap();
    assert!(!session.is_previewing_gain());
    assert_eq!(store.saves.load(Ordering::Relaxed), 2);
    assert_eq!(session.active_chain(), preview);
    assert_eq!(session.applied_chain(), Some(&preview));
    assert_eq!(
        session.selected_profile().unwrap().base(),
        saved.devices()[0].profiles()[0].base()
    );
    assert!(session.draft().is_some());
    let restored = Session::new(store.clone(), vec![preset()]).unwrap();
    assert_eq!(restored.selected_profile(), session.selected_profile());
    assert!(restored.draft().is_none());
    session.execute_edit(ProfileEdit::ResetDraft).unwrap();
    assert_eq!(session.active_chain(), restored.active_chain());
    session
        .execute_edit(ProfileEdit::FinishControlGain)
        .unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 2);
}

#[test]
fn cancelling_or_failing_a_gain_commit_restores_audio_and_preserves_base_draft() {
    for fail in [false, true] {
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), vec![preset()])
            .unwrap()
            .with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
        session.execute(use_preset()).unwrap();
        session.execute_edit(adjust(1.5)).unwrap();
        session.execute_edit(edit(800.0, -4.0)).unwrap();
        let saved = session.state().clone();
        let draft = session.draft().cloned();
        let applied = session.applied_chain().cloned();
        session
            .execute_edit(ProfileEdit::PreviewControlGain {
                filter: FilterId::try_new(3).unwrap(),
                gain: GainDb::try_new(-5.0).unwrap(),
            })
            .unwrap();
        assert_ne!(session.applied_chain(), applied.as_ref());
        store.fail.store(fail, Ordering::Relaxed);
        let result = session.execute_edit(if fail {
            ProfileEdit::FinishControlGain
        } else {
            ProfileEdit::CancelControlGain
        });
        assert_eq!(result.is_err(), fail);
        assert!(!session.is_previewing_gain());
        assert_eq!(session.state(), &saved);
        assert_eq!(session.draft(), draft.as_ref());
        assert_eq!(session.applied_chain(), applied.as_ref());
        assert_eq!(store.saves.load(Ordering::Relaxed), 2);
        assert_eq!(session.action_error().is_some(), fail);
        // Dismissal can finish again after a release; it must not hide a failed save.
        session
            .execute_edit(ProfileEdit::FinishControlGain)
            .unwrap();
        assert_eq!(session.action_error().is_some(), fail);
    }
}

#[test]
fn selection_finishes_gain_preview_and_failed_finish_prevents_switching() {
    for fail in [false, true] {
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), vec![preset()]).unwrap();
        session.execute(use_preset()).unwrap();
        session
            .execute_edit(ProfileEdit::PreviewControlGain {
                filter: FilterId::try_new(3).unwrap(),
                gain: GainDb::try_new(4.0).unwrap(),
            })
            .unwrap();
        store.fail.store(fail, Ordering::Relaxed);
        assert_eq!(session.execute(flat()).is_err(), fail);
        assert!(!session.is_previewing_gain());
        store.fail.store(false, Ordering::Relaxed);
        session.execute(select_first(&session)).unwrap();
        assert_eq!(
            session.editing_profile().unwrap().controls()[0]
                .gain_adjustment()
                .into_inner(),
            if fail { 0.0 } else { 4.0 }
        );
    }
}

#[test]
fn failed_selection_preserves_draft_and_saved_state() {
    for selection in [false, true] {
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), vec![preset()]).unwrap();
        session.execute(use_preset()).unwrap();
        let command = if selection {
            session.execute(flat()).unwrap();
            let target = session.selected_profile().unwrap().id().clone();
            session.execute(select_first(&session)).unwrap();
            Command::SelectProfile {
                device: session.device_id().clone(),
                profile: target,
            }
        } else {
            flat()
        };
        session.execute_edit(edit(800.0, 4.0)).unwrap();
        let draft = session.draft().cloned();
        let saved = session.state().clone();
        store.fail.store(true, Ordering::Relaxed);
        assert!(matches!(
            session.execute(command.clone()),
            Err(SessionError::PersistenceFailed(_))
        ));
        assert_eq!(session.state(), &saved);
        assert_eq!(session.draft(), draft.as_ref());
        assert_eq!(store.state.lock().unwrap().as_ref(), Some(&saved));
        store.fail.store(false, Ordering::Relaxed);
        session.execute(command).unwrap();
        assert!(session.draft().is_none());
        assert_eq!(session.active_chain().equalizer.filters.len(), 5);
    }
}

#[test]
fn saved_profiles_restore_without_catalog_and_other_device_selections_survive() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    session.execute_edit(adjust(4.0)).unwrap();
    session.execute_edit(ProfileEdit::SaveDraft).unwrap();
    let state = session.state().clone();
    let restored =
        Session::new(MemoryPersistence::new(state.clone()), Vec::<Preset>::new()).unwrap();
    assert_eq!(restored.state(), &state);
    assert_eq!(
        restored.active_chain().equalizer.filters[0]
            .parameters
            .gain
            .into_inner(),
        6.0
    );
    assert_eq!(restored.profiles(), session.profiles());
}

#[test]
fn corrupt_restored_state_is_rejected() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    let valid = session.state().clone();
    let profile = valid.devices()[0].profiles()[0].clone();
    let duplicate_name = Profile::new(
        ProfileId::try_new("other").unwrap(),
        profile.name().clone(),
        Chain::default(),
        vec![],
        None,
    )
    .unwrap();
    for profiles in [
        vec![profile.clone(), profile.clone()],
        vec![profile.clone(), duplicate_name],
        vec![],
    ] {
        assert!(
            DeviceProfiles::new(
                DeviceId::try_new(SYSTEM_OUTPUT).unwrap(),
                profiles,
                profile.id().clone()
            )
            .is_err()
        );
    }
    assert!(State::new(vec![valid.devices()[0].clone(), valid.devices()[0].clone()]).is_err());
}

#[test]
fn invalid_preset_requests_do_not_save_or_discard_drafts() {
    let store = TestPersistence::default();
    let mut duplicate = preset();
    duplicate.summary.id = PresetId::try_new("duplicate-name").unwrap();
    let mut session = Session::new(store.clone(), vec![preset(), duplicate]).unwrap();
    session.execute(use_preset()).unwrap();
    session.execute_edit(edit(800.0, 4.0)).unwrap();
    let draft = session.draft().cloned();
    assert!(matches!(
        session.execute(create_preset("Missing", "missing")),
        Err(SessionError::PresetNotFound(_))
    ));
    assert!(matches!(
        session.execute(create_preset("Reference", "duplicate-name")),
        Err(SessionError::ProfileNameAlreadyExists(_))
    ));
    assert_eq!(session.draft(), draft.as_ref());
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
}

#[test]
fn rejected_publication_stays_visible_through_noops_and_unchanged_refresh() {
    for noop in [ProfileEdit::SaveDraft, ProfileEdit::ResetDraft] {
        let mut source = preset();
        source.chain.equalizer.preamp = GainDb::try_new(1000.0).unwrap();
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), vec![source.clone()])
            .unwrap()
            .with_audio(platform([Ok(Some(48_000.0)), Ok(None)]), Arc::new(|| {}));
        assert!(matches!(
            session.execute(use_preset()),
            Err(SessionError::Audio(_))
        ));
        let error = session.audio_error().unwrap().to_owned();
        assert!(error.contains("PreampOutOfRange"));
        assert_eq!(session.applied_chain(), Some(&Chain::default()));
        assert_eq!(session.active_chain(), source.chain);
        // Named adjustments remain saved even when the audio route rejects the chain.
        assert!(matches!(
            session.execute_edit(adjust(4.0)),
            Err(SessionError::Audio(_))
        ));
        assert_eq!(
            session.selected_profile().unwrap().controls()[0]
                .gain_adjustment()
                .into_inner(),
            4.0
        );
        assert!(session.draft().is_none());
        assert_eq!(session.applied_chain(), Some(&Chain::default()));
        assert_eq!(store.state.lock().unwrap().as_ref(), Some(session.state()));
        session.execute_edit(noop).unwrap();
        assert!(session.action_error().is_none());
        assert_eq!(session.audio_error(), Some(error.as_str()));
        session.execute(Command::RefreshAudio).unwrap();
        assert_eq!(session.audio_error(), Some(error.as_str()));
        assert!(session.execute(use_preset()).is_err());
        session.execute(flat()).unwrap();
        assert!(session.audio_error().is_none());
        assert_eq!(session.active_chain().equalizer.filters.len(), 5);
    }
}

#[test]
fn rejected_edit_preserves_live_draft_without_creating_persistent_audio_failure() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()])
        .unwrap()
        .with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
    session.execute(use_preset()).unwrap();
    session.execute_edit(adjust(4.0)).unwrap();
    let draft = session.draft().cloned();
    let applied = session.applied_chain().cloned();
    assert!(matches!(
        session.execute_edit(edit(30_000.0, 8.0)),
        Err(SessionError::Audio(_))
    ));
    assert_eq!(session.draft(), draft.as_ref());
    assert_eq!(session.applied_chain(), applied.as_ref());
    assert!(session.audio_error().is_none());
    assert!(session.action_error().is_some());
    session.execute_edit(ProfileEdit::SaveDraft).unwrap();
    assert_eq!(session.applied_chain(), applied.as_ref());
}

#[test]
fn successful_edit_or_reconnection_clears_publication_failure() {
    for reconnect in [false, true] {
        let mut source = preset();
        source.chain.equalizer.filters[0].parameters.frequency =
            FrequencyHz::try_new(30_000.0).unwrap();
        let mut session = Session::new(MemoryPersistence::default(), vec![source])
            .unwrap()
            .with_audio(
                platform([Ok(Some(48_000.0)), Ok(Some(96_000.0))]),
                Arc::new(|| {}),
            );
        assert!(session.execute(use_preset()).is_err());
        if reconnect {
            session.execute(Command::RefreshAudio).unwrap();
        } else {
            session.execute_edit(edit(1000.0, 2.0)).unwrap();
        }
        assert!(session.audio_error().is_none());
        assert_eq!(session.applied_chain(), Some(&session.active_chain()));
    }
}

#[test]
fn failed_watcher_cannot_be_bypassed_by_refresh_commands() {
    let mut unavailable = platform([]);
    unavailable.watch_error = Some("watcher unavailable".into());
    let calls = Arc::clone(&unavailable.chains);
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()])
        .unwrap()
        .with_audio(unavailable, Arc::new(|| {}));
    for _ in 0..2 {
        assert_eq!(
            session.execute(Command::RefreshAudio),
            Err(SessionError::Audio("watcher unavailable".into()))
        );
        assert_eq!(session.audio_error(), Some("watcher unavailable"));
        assert!(!session.audio_retry_needed());
        assert_eq!(session.audio_generation(), 0);
        assert!(session.applied_chain().is_none());
        assert!(session.subscribe_telemetry().is_none());
        assert!(!session.can_create_profile());
        assert_eq!(
            session.execute(flat()),
            Err(SessionError::DeviceUnavailable)
        );
    }
    assert!(calls.lock().unwrap().is_empty());
    let session = session.with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
    assert!(session.audio_error().is_none());
    assert!(session.subscribe_telemetry().is_some());
}

#[test]
fn initial_connection_failure_retries_after_successful_watcher_installation() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()])
        .unwrap()
        .with_audio(
            platform([Err("route unavailable".into()), Ok(Some(48_000.0))]),
            Arc::new(|| {}),
        );
    assert!(session.audio_retry_needed());
    assert_eq!(session.audio_error(), Some("route unavailable"));
    session.execute(Command::RefreshAudio).unwrap();
    assert!(!session.audio_retry_needed());
    assert!(session.audio_error().is_none());
    assert_eq!(session.audio_generation(), 1);
}

#[test]
fn lost_route_reconnects_with_draft_and_preserves_action_errors() {
    let output = platform([
        Ok(Some(48_000.0)),
        Err("route lost".into()),
        Ok(Some(96_000.0)),
    ]);
    let chains = Arc::clone(&output.chains);
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()])
        .unwrap()
        .with_audio(output, Arc::new(|| {}));
    session.execute(use_preset()).unwrap();
    session.execute_edit(edit(800.0, 4.0)).unwrap();
    assert!(
        session
            .execute(create_preset("Missing", "missing"))
            .is_err()
    );
    let error = session.action_error().cloned();
    let desired = session.active_chain();
    assert!(session.execute(Command::RefreshAudio).is_err());
    assert_eq!(session.audio_generation(), 2);
    assert!(session.audio_retry_needed());
    assert!(session.subscribe_telemetry().is_none());
    assert!(session.applied_chain().is_none());
    session.execute(Command::RefreshAudio).unwrap();
    assert_eq!(session.audio_generation(), 3);
    assert!(!session.audio_retry_needed());
    assert_eq!(session.sample_rate().into_inner(), 96_000.0);
    assert_eq!(chains.lock().unwrap().last(), Some(&desired));
    assert_eq!(session.applied_chain(), Some(&desired));
    assert_eq!(session.action_error(), error.as_ref());
}

struct DeviceRoutes {
    steps: VecDeque<Result<DeviceId, String>>,
    initial_chains: Arc<Mutex<Vec<(DeviceId, Chain)>>>,
    processor: Option<Processor>,
}

impl DeviceRoutes {
    fn new(ids: &[&str]) -> Self {
        Self {
            steps: ids
                .iter()
                .map(|id| Ok(DeviceId::try_new(*id).unwrap()))
                .collect(),
            initial_chains: Arc::default(),
            processor: None,
        }
    }
}

impl Platform for DeviceRoutes {
    fn watch_default_output(&mut self, _: ChangeHandler) -> Result<(), String> {
        Ok(())
    }

    fn refresh_default_output(
        &mut self,
        chain_for_device: &dyn Fn(&DeviceId) -> Chain,
    ) -> Result<Option<Connection>, String> {
        self.processor = None;
        let device_id = self.steps.pop_front().expect("unexpected refresh")?;
        let chain = chain_for_device(&device_id);
        self.initial_chains
            .lock()
            .unwrap()
            .push((device_id.clone(), chain.clone()));
        let (processor, controller) = Processor::new(
            AudioFormat {
                sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
                maximum_frame_count: NonZeroUsize::new(64).unwrap(),
            },
            chain,
            false,
        )
        .map_err(|error| format!("{error:?}"))?;
        self.processor = Some(processor);
        Ok(Some(Connection {
            device_id,
            device_name: "Output".into(),
            controller,
        }))
    }
}

fn create_on(session: &Session, name: &str, source: ProfileSource) -> Command {
    Command::CreateProfile {
        device: session.device_id().clone(),
        name: ProfileName::try_new(name).unwrap(),
        source,
    }
}

#[test]
fn failed_initial_route_cannot_create_profiles_for_the_placeholder_device() {
    let store = TestPersistence::default();
    let mut routes = DeviceRoutes::new(&["real-headphones"]);
    routes
        .steps
        .push_front(Err("audio permission unavailable".into()));
    let mut session = Session::new(store.clone(), vec![preset()])
        .unwrap()
        .with_audio(routes, Arc::new(|| {}));
    assert!(session.audio_error().is_some());
    assert!(!session.can_create_profile());
    assert_eq!(
        session.execute(create_on(&session, "Custom", ProfileSource::Flat)),
        Err(SessionError::DeviceUnavailable)
    );
    assert!(session.state().devices().is_empty());
    assert_eq!(store.saves.load(Ordering::Relaxed), 0);

    session.execute(Command::RefreshAudio).unwrap();
    assert!(session.can_create_profile());
    session
        .execute(create_on(&session, "Custom", ProfileSource::Flat))
        .unwrap();
    assert_eq!(session.state().devices().len(), 1);
    assert_eq!(
        session.state().devices()[0].device_id().to_string(),
        "real-headphones"
    );
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
}

#[test]
fn independent_preset_copies_and_device_recall_survive_routes_and_restart() {
    let store = TestPersistence::default();
    let routes = DeviceRoutes::new(&["headphones", "speakers", "headphones", "speakers"]);
    let chains = Arc::clone(&routes.initial_chains);
    let source = preset();
    let mut session = Session::new(store.clone(), vec![source.clone()])
        .unwrap()
        .with_audio(routes, Arc::new(|| {}));
    assert!(session.profiles().is_empty());
    assert!(session.selected_profile().is_none());
    session
        .execute(create_on(
            &session,
            "Reference",
            ProfileSource::Preset(source.summary.id.clone()),
        ))
        .unwrap();
    let first_id = session.selected_profile().unwrap().id().clone();
    session.execute_edit(adjust(4.0)).unwrap();
    let first = session.selected_profile().unwrap().clone();
    session
        .execute(create_on(
            &session,
            "Second copy",
            ProfileSource::Preset(source.summary.id.clone()),
        ))
        .unwrap();
    let second_id = session.selected_profile().unwrap().id().clone();
    assert_ne!(first_id, second_id);
    assert_eq!(session.active_chain(), source.chain);
    assert_eq!(
        session.selected_profile().unwrap().controls(),
        source.controls
    );
    assert_eq!(session.selected_profile().unwrap().origin(), first.origin());
    session
        .execute(Command::SelectProfile {
            device: session.device_id().clone(),
            profile: first_id.clone(),
        })
        .unwrap();
    session.execute(Command::RefreshAudio).unwrap();
    assert!(session.selected_profile().is_none());
    assert_eq!(session.active_chain(), Chain::default());
    // Names are scoped to a device, not globally reserved.
    session
        .execute(create_on(
            &session,
            "Reference",
            ProfileSource::Preset(source.summary.id.clone()),
        ))
        .unwrap();
    let speakers_id = session.selected_profile().unwrap().id().clone();
    assert_ne!(speakers_id, first_id);
    session.execute_edit(adjust(-2.0)).unwrap();
    let speakers = session.selected_profile().unwrap().clone();
    session.execute(Command::RefreshAudio).unwrap();
    assert_eq!(session.selected_profile(), Some(&first));
    session.execute_edit(edit(700.0, 5.0)).unwrap();
    session.execute(Command::RefreshAudio).unwrap();
    assert_eq!(session.selected_profile(), Some(&speakers));
    assert!(session.draft().is_none());
    assert_eq!(
        chains
            .lock()
            .unwrap()
            .iter()
            .map(|(_, chain)| chain.clone())
            .collect::<Vec<_>>(),
        vec![
            Chain::default(),
            Chain::default(),
            first.effective_chain(),
            speakers.effective_chain()
        ]
    );
    drop(session);

    let routes = DeviceRoutes::new(&["headphones", "speakers"]);
    let chains = Arc::clone(&routes.initial_chains);
    let mut restored = Session::new(store, Vec::<Preset>::new())
        .unwrap()
        .with_audio(routes, Arc::new(|| {}));
    assert_eq!(restored.selected_profile(), Some(&first));
    assert_eq!(restored.profiles().len(), 2);
    restored.execute(Command::RefreshAudio).unwrap();
    assert_eq!(restored.selected_profile(), Some(&speakers));
    assert_eq!(
        chains
            .lock()
            .unwrap()
            .iter()
            .map(|(_, chain)| chain.clone())
            .collect::<Vec<_>>(),
        vec![first.effective_chain(), speakers.effective_chain()]
    );
    restored
        .execute(create_on(
            &restored,
            "New after restart",
            ProfileSource::Flat,
        ))
        .unwrap();
    let new_id = restored.selected_profile().unwrap().id();
    assert!(![first_id, second_id, speakers_id].contains(new_id));
}

#[test]
fn delayed_profile_edits_preserve_new_device_preview_and_draft() {
    for delayed in [
        ProfileEdit::PreviewControlGain {
            filter: FilterId::try_new(1).unwrap(),
            gain: GainDb::try_new(9.0).unwrap(),
        },
        ProfileEdit::FinishControlGain,
        ProfileEdit::CancelControlGain,
        ProfileEdit::SaveDraft,
        ProfileEdit::ResetDraft,
        ProfileEdit::SetControlGain {
            filter: FilterId::try_new(1).unwrap(),
            gain: GainDb::try_new(9.0).unwrap(),
        },
        ProfileEdit::EditFilter {
            filter: FilterId::try_new(1).unwrap(),
            frequency: FrequencyHz::try_new(200.0).unwrap(),
            gain: GainDb::try_new(9.0).unwrap(),
        },
    ] {
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), Vec::<Preset>::new())
            .unwrap()
            .with_audio(DeviceRoutes::new(&["a", "b"]), Arc::new(|| {}));
        session
            .execute(create_on(&session, "Flat", ProfileSource::Flat))
            .unwrap();
        let a = session.selected_profile().unwrap().id().clone();
        session.execute(Command::RefreshAudio).unwrap();
        session
            .execute(create_on(&session, "Flat", ProfileSource::Flat))
            .unwrap();
        session
            .execute_edit(ProfileEdit::EditFilter {
                filter: FilterId::try_new(1).unwrap(),
                frequency: FrequencyHz::try_new(100.0).unwrap(),
                gain: GainDb::try_new(2.0).unwrap(),
            })
            .unwrap();
        session
            .execute_edit(ProfileEdit::PreviewControlGain {
                filter: FilterId::try_new(1).unwrap(),
                gain: GainDb::try_new(3.0).unwrap(),
            })
            .unwrap();
        let state = session.state().clone();
        let chain = session.active_chain();
        let draft = session.draft().cloned();
        let saves = store.saves.load(Ordering::Relaxed);
        assert_eq!(
            session.execute(Command::EditProfile {
                profile: a.clone(),
                edit: delayed.clone()
            }),
            Err(SessionError::StaleProfile(a.clone()))
        );
        assert_eq!(session.active_chain(), chain);
        assert_eq!(session.state(), &state);
        assert_eq!(session.draft(), draft.as_ref());
        assert!(session.is_previewing_gain());
        assert_eq!(store.saves.load(Ordering::Relaxed), saves);
        session
            .execute_edit(ProfileEdit::CancelControlGain)
            .unwrap();
        let chain = session.active_chain();
        assert_eq!(
            session.execute(Command::EditProfile {
                profile: a.clone(),
                edit: delayed
            }),
            Err(SessionError::StaleProfile(a))
        );
        assert_eq!(session.active_chain(), chain);
        assert_eq!(session.applied_chain(), Some(&chain));
        assert!(!session.is_previewing_gain());
        assert_eq!(session.state(), &state);
        assert_eq!(session.draft(), draft.as_ref());
        assert_eq!(store.saves.load(Ordering::Relaxed), saves);
    }
}

#[test]
fn stale_and_cross_device_commands_do_not_finish_gain_preview() {
    let store = TestPersistence::default();
    let mut session = Session::new(store.clone(), vec![preset()])
        .unwrap()
        .with_audio(DeviceRoutes::new(&["a", "b", "a"]), Arc::new(|| {}));
    session
        .execute(create_on(
            &session,
            "Reference",
            ProfileSource::Preset(preset().summary.id),
        ))
        .unwrap();
    let a = session.device_id().clone();
    let a_profile = session.selected_profile().unwrap().id().clone();
    session.execute(Command::RefreshAudio).unwrap();
    session
        .execute(create_on(
            &session,
            "Reference",
            ProfileSource::Preset(preset().summary.id),
        ))
        .unwrap();
    let b = session.device_id().clone();
    let b_profile = session.selected_profile().unwrap().id().clone();
    session.execute(Command::RefreshAudio).unwrap();
    session
        .execute_edit(ProfileEdit::PreviewControlGain {
            filter: FilterId::try_new(3).unwrap(),
            gain: GainDb::try_new(4.0).unwrap(),
        })
        .unwrap();
    let state = session.state().clone();
    let chain = session.active_chain();
    let saves = store.saves.load(Ordering::Relaxed);
    for command in [
        Command::CreateProfile {
            device: b.clone(),
            name: ProfileName::try_new("Wrong device").unwrap(),
            source: ProfileSource::Flat,
        },
        Command::SelectProfile {
            device: b,
            profile: b_profile.clone(),
        },
        Command::SelectProfile {
            device: a.clone(),
            profile: b_profile,
        },
        Command::SelectProfile {
            device: a,
            profile: ProfileId::try_new("missing").unwrap(),
        },
    ] {
        assert!(session.execute(command).is_err());
        assert!(session.is_previewing_gain());
        assert_eq!(session.state(), &state);
        assert_eq!(session.active_chain(), chain);
        assert_eq!(session.selected_profile().unwrap().id(), &a_profile);
        assert_eq!(store.saves.load(Ordering::Relaxed), saves);
    }
}

#[test]
fn route_refresh_finishes_gains_and_preserves_draft_until_successful_switch() {
    let store = TestPersistence::default();
    let mut routes = DeviceRoutes::new(&["a", "a", "b"]);
    routes
        .steps
        .insert(1, Err("temporarily unavailable".into()));
    let chains = Arc::clone(&routes.initial_chains);
    let mut session = Session::new(store.clone(), vec![preset()])
        .unwrap()
        .with_audio(routes, Arc::new(|| {}));
    session
        .execute(create_on(
            &session,
            "Reference",
            ProfileSource::Preset(preset().summary.id),
        ))
        .unwrap();
    session.execute_edit(edit(700.0, 5.0)).unwrap();
    session
        .execute_edit(ProfileEdit::PreviewControlGain {
            filter: FilterId::try_new(3).unwrap(),
            gain: GainDb::try_new(4.0).unwrap(),
        })
        .unwrap();
    let preview = session.active_chain();
    store.fail.store(true, Ordering::Relaxed);
    assert!(matches!(
        session.execute(Command::RefreshAudio),
        Err(SessionError::PersistenceFailed(_))
    ));
    assert_eq!(chains.lock().unwrap().len(), 1); // No route work after a failed gain save.
    assert!(session.draft().is_some());
    store.fail.store(false, Ordering::Relaxed);
    session
        .execute_edit(ProfileEdit::PreviewControlGain {
            filter: FilterId::try_new(3).unwrap(),
            gain: GainDb::try_new(4.0).unwrap(),
        })
        .unwrap();
    assert!(session.execute(Command::RefreshAudio).is_err());
    assert!(!session.is_previewing_gain());
    assert!(session.draft().is_some());
    assert_eq!(session.device_id(), &DeviceId::try_new("a").unwrap());
    assert_eq!(
        session.selected_profile().unwrap().controls()[0]
            .gain_adjustment()
            .into_inner(),
        4.0
    );
    session.execute(Command::RefreshAudio).unwrap();
    assert_eq!(session.applied_chain(), Some(&preview));
    assert!(session.draft().is_some());
    session.execute(Command::RefreshAudio).unwrap();
    assert!(session.draft().is_none());
    assert!(session.selected_profile().is_none());
    assert_eq!(session.applied_chain(), Some(&Chain::default()));
}

#[test]
fn flat_profile_has_neutral_response_and_editable_named_bands() {
    let mut session = Session::new(MemoryPersistence::default(), Vec::<Preset>::new()).unwrap();
    session.execute(flat()).unwrap();
    let profile = session.selected_profile().unwrap();
    assert!(profile.origin().is_none());
    assert_eq!(profile.base().equalizer.preamp, GainDb::default());
    assert_eq!(
        profile
            .controls()
            .iter()
            .map(|control| control.name().to_string())
            .collect::<Vec<_>>(),
        ["Bass", "Warmth", "Mids", "Presence", "Treble"]
    );
    assert_eq!(
        profile
            .base()
            .equalizer
            .filters
            .iter()
            .map(|filter| filter.parameters.frequency.into_inner())
            .collect::<Vec<_>>(),
        [100.0, 300.0, 1000.0, 3000.0, 8000.0]
    );
    let response =
        crate::FrequencyResponse::new(&session.active_chain(), session.sample_rate()).unwrap();
    for frequency in [20.0, 100.0, 300.0, 1000.0, 3000.0, 8000.0, 20000.0] {
        assert!(
            response
                .db_at(FrequencyHz::try_new(frequency).unwrap())
                .unwrap()
                .abs()
                < 1e-10
        );
    }
    let targets = profile
        .controls()
        .iter()
        .map(FilterControl::target)
        .collect::<Vec<_>>();
    for target in targets {
        session
            .execute_edit(ProfileEdit::SetControlGain {
                filter: target,
                gain: GainDb::try_new(3.0).unwrap(),
            })
            .unwrap();
        session
            .execute_edit(ProfileEdit::EditFilter {
                filter: target,
                frequency: FrequencyHz::try_new(500.0).unwrap(),
                gain: GainDb::try_new(-1.0).unwrap(),
            })
            .unwrap();
        session.execute_edit(ProfileEdit::SaveDraft).unwrap();
        let filter = session
            .active_chain()
            .equalizer
            .filters
            .into_iter()
            .find(|filter| filter.id == target)
            .unwrap();
        assert_eq!(filter.parameters.frequency.into_inner(), 500.0);
        assert_eq!(filter.parameters.gain.into_inner(), 2.0);
    }
}
