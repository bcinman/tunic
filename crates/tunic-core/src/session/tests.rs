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
    Command::UsePreset(preset().summary.id)
}
fn adjust(gain: f64) -> Command {
    Command::SetControlGain {
        filter: FilterId::try_new(3).unwrap(),
        gain: GainDb::try_new(gain).unwrap(),
    }
}
fn edit(frequency: f64, gain: f64) -> Command {
    Command::EditFilter {
        filter: FilterId::try_new(3).unwrap(),
        frequency: FrequencyHz::try_new(frequency).unwrap(),
        gain: GainDb::try_new(gain).unwrap(),
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
    fn refresh_default_output(&mut self, chain: &Chain) -> Result<Option<Connection>, String> {
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
    session.execute(use_preset()).unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
    session.execute(Command::UseFlat).unwrap();
    assert_eq!(session.active_chain(), Chain::default());
    assert_eq!(session.state().profiles.len(), 2);
}

#[test]
fn drafts_compose_base_and_adjustment_and_save_only_changes() {
    let store = TestPersistence::default();
    let mut session = Session::new(store.clone(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    let saved = session.selected_profile().unwrap().clone();
    session.execute(adjust(4.0)).unwrap();
    session.execute(edit(800.0, 5.0)).unwrap();
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
    assert!(session.execute(Command::SaveDraft).is_err());
    assert_eq!(session.selected_profile(), Some(&saved));
    assert_eq!(session.draft(), Some(&draft));
    store.fail.store(false, Ordering::Relaxed);
    session.execute(Command::SaveDraft).unwrap();
    assert_eq!(session.selected_profile(), Some(&draft));
    assert!(session.draft().is_none());
    assert_eq!(store.saves.load(Ordering::Relaxed), 2);
    session.execute(adjust(4.0)).unwrap();
    session.execute(Command::SaveDraft).unwrap();
    assert_eq!(store.saves.load(Ordering::Relaxed), 2);
    session.execute(adjust(-2.0)).unwrap();
    session.execute(Command::ResetDraft).unwrap();
    assert_eq!(session.selected_profile(), Some(&draft));
    assert_eq!(session.active_chain(), draft.effective_chain());
}

#[test]
fn failed_selection_preserves_draft_and_saved_state() {
    for command in [Command::ClearSelection, Command::UseFlat] {
        let store = TestPersistence::default();
        let mut session = Session::new(store.clone(), vec![preset()]).unwrap();
        session.execute(use_preset()).unwrap();
        session.execute(adjust(4.0)).unwrap();
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
        assert_eq!(session.active_chain(), Chain::default());
    }
}

#[test]
fn saved_profiles_restore_without_catalog_and_other_device_selections_survive() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    session.execute(adjust(4.0)).unwrap();
    session.execute(Command::SaveDraft).unwrap();
    let mut state = session.state().clone();
    let other = DeviceProfileSelection {
        device: DeviceId::try_new("other-output").unwrap(),
        profile: state.profiles[0].id().clone(),
    };
    state.selections.push(other.clone());
    let mut restored =
        Session::new(MemoryPersistence::new(state.clone()), Vec::<Preset>::new()).unwrap();
    assert_eq!(restored.state(), &state);
    restored.execute(Command::ClearSelection).unwrap();
    assert_eq!(restored.state().selections, vec![other]);
    restored.execute(use_preset()).unwrap();
    assert_eq!(
        restored.active_chain().equalizer.filters[0]
            .parameters
            .gain
            .into_inner(),
        6.0
    );
    assert_eq!(restored.state().profiles, state.profiles);
}

#[test]
fn corrupt_restored_state_is_rejected() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()]).unwrap();
    session.execute(use_preset()).unwrap();
    let valid = session.state().clone();
    let mut duplicate_id = valid.clone();
    duplicate_id.profiles.push(valid.profiles[0].clone());
    let mut duplicate_name = valid.clone();
    duplicate_name.profiles.push(
        Profile::new(
            ProfileId::try_new("other").unwrap(),
            valid.profiles[0].name().clone(),
            Chain::default(),
            vec![],
            None,
        )
        .unwrap(),
    );
    let mut duplicate_selection = valid.clone();
    duplicate_selection
        .selections
        .push(valid.selections[0].clone());
    let mut missing_profile = valid;
    missing_profile.profiles.clear();
    for state in [
        duplicate_id,
        duplicate_name,
        duplicate_selection,
        missing_profile,
    ] {
        assert!(matches!(
            Session::new(MemoryPersistence::new(state), Vec::<Preset>::new()),
            Err(SessionError::PersistenceFailed(
                PersistenceError::Corrupt { .. }
            ))
        ));
    }
}

#[test]
fn invalid_preset_requests_do_not_save_or_discard_drafts() {
    let store = TestPersistence::default();
    let mut duplicate = preset();
    duplicate.summary.id = PresetId::try_new("duplicate-name").unwrap();
    let mut session = Session::new(store.clone(), vec![preset(), duplicate]).unwrap();
    session.execute(use_preset()).unwrap();
    session.execute(adjust(4.0)).unwrap();
    let draft = session.draft().cloned();
    assert!(matches!(
        session.execute(Command::UsePreset(PresetId::try_new("missing").unwrap())),
        Err(SessionError::PresetNotFound(_))
    ));
    assert!(matches!(
        session.execute(Command::UsePreset(
            PresetId::try_new("duplicate-name").unwrap()
        )),
        Err(SessionError::ProfileNameAlreadyExists(_))
    ));
    assert_eq!(session.draft(), draft.as_ref());
    assert_eq!(store.saves.load(Ordering::Relaxed), 1);
}

#[test]
fn rejected_publication_stays_visible_through_noops_and_unchanged_refresh() {
    for noop in [Command::SaveDraft, Command::ResetDraft] {
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
        assert_eq!(store.state.lock().unwrap().as_ref(), Some(session.state()));
        session.execute(noop).unwrap();
        assert!(session.action_error().is_none());
        assert_eq!(session.audio_error(), Some(error.as_str()));
        session.execute(Command::RefreshAudio).unwrap();
        assert_eq!(session.audio_error(), Some(error.as_str()));
        assert!(session.execute(use_preset()).is_err());
        session.execute(Command::ClearSelection).unwrap();
        assert!(session.audio_error().is_none());
        assert_eq!(session.active_chain(), Chain::default());
    }
}

#[test]
fn rejected_edit_preserves_live_draft_without_creating_persistent_audio_failure() {
    let mut session = Session::new(MemoryPersistence::default(), vec![preset()])
        .unwrap()
        .with_audio(platform([Ok(Some(48_000.0))]), Arc::new(|| {}));
    session.execute(use_preset()).unwrap();
    session.execute(adjust(4.0)).unwrap();
    let draft = session.draft().cloned();
    let applied = session.applied_chain().cloned();
    assert!(matches!(
        session.execute(edit(30_000.0, 8.0)),
        Err(SessionError::Audio(_))
    ));
    assert_eq!(session.draft(), draft.as_ref());
    assert_eq!(session.applied_chain(), applied.as_ref());
    assert!(session.audio_error().is_none());
    assert!(session.action_error().is_some());
    session.execute(Command::SaveDraft).unwrap();
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
            session.execute(edit(1000.0, 2.0)).unwrap();
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
    session.execute(adjust(4.0)).unwrap();
    assert!(
        session
            .execute(Command::UsePreset(PresetId::try_new("missing").unwrap()))
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
