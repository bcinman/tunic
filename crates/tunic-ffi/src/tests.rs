use super::*;

fn create_flat(name: &str) -> EngineCommand {
    EngineCommand::CreateProfile {
        device: "system-output".into(),
        name: name.into(),
        source: ProfileSource::Flat,
    }
}

fn wait_for(engine: &Engine, predicate: impl Fn(&StateSnapshot) -> bool) -> StateSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = engine.snapshot().filter(&predicate) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "engine did not publish expected state"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn commands_preserve_drafts_and_publish_failures() {
    let directory = std::env::temp_dir().join(format!("tunic-ffi-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let database = directory
        .join("session.sqlite3")
        .to_str()
        .unwrap()
        .to_owned();
    let engine = Engine::new(false, Some(database.clone())).unwrap();
    let initial = wait_for(&engine, |_| true);
    let preset = initial.presets.iter().find(|p| p.model == "HD650").unwrap();
    engine
        .enqueue(EngineCommand::CreateProfile {
            device: initial.device_id.clone(),
            name: "Reference".into(),
            source: ProfileSource::Preset {
                id: preset.id.clone(),
            },
        })
        .unwrap();
    let selected = wait_for(&engine, |s| s.preset_id.as_ref() == Some(&preset.id));
    let filter = selected.base_chain.filters[0].clone();
    engine
        .enqueue(EngineCommand::EditFilter {
            profile: selected.selected_profile_id.clone().unwrap(),
            filter: filter.id,
            frequency: 123.0,
            gain: -3.25,
        })
        .unwrap();
    let edited = wait_for(&engine, |s| s.has_draft);
    assert_eq!(edited.base_chain.filters[0].frequency, 123.0);
    assert_eq!(edited.base_chain.filters[0].gain, -3.25);
    assert!(edited.accepted_chain.is_none());

    let failed_receipt = engine
        .enqueue(EngineCommand::CreateProfile {
            device: initial.device_id.clone(),
            name: "Missing".into(),
            source: ProfileSource::Preset {
                id: "missing".into(),
            },
        })
        .unwrap();
    let failed = wait_for(&engine, |s| s.processed_command >= failed_receipt);
    assert!(failed.action_error.is_some());
    assert!(failed.has_draft);
    assert_eq!(failed.base_chain.filters[0].gain, -3.25);
    engine
        .enqueue(EngineCommand::ResetDraft {
            profile: selected.selected_profile_id.clone().unwrap(),
        })
        .unwrap();
    let reset = wait_for(&engine, |s| !s.has_draft && s.action_error.is_none());
    assert_eq!(reset.base_chain.filters[0].frequency, filter.frequency);
    assert_eq!(reset.base_chain.filters[0].gain, filter.gain);

    engine
        .enqueue(EngineCommand::EditFilter {
            profile: selected.selected_profile_id.clone().unwrap(),
            filter: filter.id,
            frequency: 237.0,
            gain: 2.5,
        })
        .unwrap();
    wait_for(&engine, |s| s.has_draft);
    engine
        .enqueue(EngineCommand::SaveDraft {
            profile: selected.selected_profile_id.clone().unwrap(),
        })
        .unwrap();
    let saved = wait_for(&engine, |s| !s.has_draft);
    assert_eq!(saved.base_chain.filters[0].gain, 2.5);
    assert_eq!(saved.base_chain.filters[0].frequency, 237.0);
    engine.shutdown();
    engine.shutdown();
    assert!(matches!(
        engine.enqueue(create_flat("Closed")),
        Err(EngineError::Closed)
    ));
    let reopened = Engine::new(false, Some(database.clone())).unwrap();
    let restored = wait_for(&reopened, |_| true);
    assert_eq!(restored.preset_id, Some(preset.id.clone()));
    assert_eq!(restored.base_chain.filters[0].gain, 2.5);
    assert_eq!(restored.base_chain.filters[0].frequency, 237.0);
    assert!(!restored.has_draft);
    reopened.shutdown();
    std::fs::write(&database, "not a SQLite database").unwrap();
    assert!(matches!(
        Engine::new(false, Some(database)),
        Err(EngineError::Unavailable { .. })
    ));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn editing_commands_keep_profile_target_and_reject_stale_preview_completion() {
    let mut session = Session::new(MemoryPersistence::default(), BundledCatalog).unwrap();
    session
        .execute(create_flat("First").try_into().unwrap())
        .unwrap();
    let first = session.selected_profile().unwrap().id().clone();
    session
        .execute(create_flat("Second").try_into().unwrap())
        .unwrap();
    let second = session.selected_profile().unwrap().id().to_string();
    session
        .execute(
            EngineCommand::EditFilter {
                profile: second.clone(),
                filter: 1,
                frequency: 150.0,
                gain: 2.0,
            }
            .try_into()
            .unwrap(),
        )
        .unwrap();
    session
        .execute(
            EngineCommand::PreviewControlGain {
                profile: second.clone(),
                filter: 1,
                gain: 3.0,
            }
            .try_into()
            .unwrap(),
        )
        .unwrap();
    let state = session.state().clone();
    let chain = session.active_chain();
    let draft = session.draft().cloned();
    for foreign in [
        EngineCommand::EditFilter {
            profile: first.to_string(),
            filter: 1,
            frequency: 200.0,
            gain: 9.0,
        },
        EngineCommand::SetControlGain {
            profile: first.to_string(),
            filter: 1,
            gain: 9.0,
        },
        EngineCommand::PreviewControlGain {
            profile: first.to_string(),
            filter: 1,
            gain: 9.0,
        },
        EngineCommand::FinishControlGain {
            profile: first.to_string(),
        },
        EngineCommand::CancelControlGain {
            profile: first.to_string(),
        },
        EngineCommand::SaveDraft {
            profile: first.to_string(),
        },
        EngineCommand::ResetDraft {
            profile: first.to_string(),
        },
    ] {
        let command = Command::try_from(foreign).unwrap();
        assert!(matches!(&command, Command::EditProfile { profile, .. } if profile == &first));
        assert_eq!(
            session.execute(command),
            Err(tunic_core::SessionError::StaleProfile(first.clone()))
        );
        assert!(session.is_previewing_gain());
        assert_eq!(session.state(), &state);
        assert_eq!(session.active_chain(), chain);
        assert_eq!(session.draft(), draft.as_ref());
    }
    session
        .execute(
            EngineCommand::FinishControlGain { profile: second }
                .try_into()
                .unwrap(),
        )
        .unwrap();
    assert!(!session.is_previewing_gain());
    assert_eq!(session.active_chain(), chain);
    assert_eq!(
        session.selected_profile().unwrap().controls()[0]
            .gain_adjustment()
            .into_inner(),
        3.0
    );
    assert!(session.draft().is_some());
}

#[test]
fn validates_foreign_values_and_uses_processing_response() {
    for (filter, frequency, gain) in [
        (0, 100.0, 1.0),
        (1, f64::NAN, 1.0),
        (1, 100.0, f64::INFINITY),
    ] {
        assert!(
            Command::try_from(EngineCommand::EditFilter {
                profile: "profile-1".into(),
                filter,
                frequency,
                gain
            })
            .is_err()
        );
    }
    let chain = Chain {
        preamp: -9.0,
        filters: vec![Filter {
            id: 7,
            kind: FilterKind::Peaking,
            frequency: 1234.0,
            gain: 6.0,
            quality_factor: 0.8,
        }],
    };
    let points =
        frequency_response(chain.clone(), 48_000.0, vec![100.0, 1234.0, 10_000.0]).unwrap();
    assert!((points[1] - 6.0).abs() < 0.001);
    assert!(points[0] < 0.2 && points[2] < 0.2);
    assert!(frequency_response(chain.clone(), 0.0, vec![100.0]).is_err());
    assert!(frequency_response(chain, 48_000.0, vec![f64::NAN]).is_err());
}

#[test]
fn concurrent_callers_share_one_owner_and_drop_stops_it() {
    let engine = Arc::new(Engine::new(false, None).unwrap());
    wait_for(&engine, |_| true);
    let shared = Arc::clone(&engine.shared);
    let workers: Vec<_> = (0..4)
        .map(|worker| {
            let engine = Arc::clone(&engine);
            thread::spawn(move || {
                for index in 0..25 {
                    engine
                        .enqueue(create_flat(&format!("Flat {worker}-{index}")))
                        .unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let snapshot = wait_for(&engine, |s| s.revision == 101);
    assert_eq!(snapshot.processed_command, 100);
    assert!(snapshot.profile_name.is_some());
    assert!(snapshot.action_error.is_none());
    drop(engine);
    assert!(shared.updates.lock().unwrap().closed);
}

#[test]
fn telemetry_mapping_preserves_channel_order_and_cursor() {
    use tunic_core::{AudioFormat, Processor, SampleRateHz};
    let (mut processor, controller) = Processor::new(
        AudioFormat {
            sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
            maximum_frame_count: 4096.try_into().unwrap(),
        },
        tunic_core::Chain::default(),
        false,
    )
    .unwrap();
    let engine = Engine::new(false, None).unwrap();
    wait_for(&engine, |_| true);
    {
        let mut measurements = engine.shared.measurements.lock().unwrap();
        measurements.generation = 7;
        measurements.subscription = Some(controller.subscribe_telemetry());
    }
    let mut samples: Vec<f32> = (0..4096).flat_map(|_| [0.25, -0.5]).collect();
    processor.process(&mut samples);
    let batch = engine.poll_telemetry();
    assert_eq!(batch.audio_generation, 7);
    let frame = batch.frames.last().unwrap();
    assert_eq!(frame.left_peak, 0.25);
    assert_eq!(frame.right_peak, 0.5);
    assert!((frame.left_rms - 0.25).abs() < 0.0001);
    assert!((frame.right_rms - 0.5).abs() < 0.0001);
    assert_eq!(frame.spectrum.len(), 256);
    assert!(engine.poll_telemetry().frames.is_empty());
    engine.shutdown();
    assert!(engine.poll_telemetry().frames.is_empty());
}

#[test]
fn route_loss_retries_replaces_telemetry_and_tears_down_on_owner() {
    use std::collections::VecDeque;
    use tunic_core::{AudioFormat, ChangeHandler, Connection, Platform, Processor, SampleRateHz};

    struct TestPlatform {
        steps: VecDeque<Result<f64, String>>,
        processor: Option<Processor>,
        dropped: mpsc::Sender<thread::ThreadId>,
    }
    impl Platform for TestPlatform {
        fn watch_default_output(&mut self, _: ChangeHandler) -> Result<(), String> {
            Ok(())
        }
        fn refresh_default_output(
            &mut self,
            chain_for_device: &dyn Fn(&tunic_core::DeviceId) -> tunic_core::Chain,
        ) -> Result<Option<Connection>, String> {
            let device_id = tunic_core::DeviceId::try_new("system-output").unwrap();
            let chain = chain_for_device(&device_id);
            self.processor = None;
            let rate = self.steps.pop_front().expect("unexpected refresh")?;
            let (processor, controller) = Processor::new(
                AudioFormat {
                    sample_rate: SampleRateHz::try_new(rate).unwrap(),
                    maximum_frame_count: 64.try_into().unwrap(),
                },
                chain.clone(),
                false,
            )
            .unwrap();
            self.processor = Some(processor);
            Ok(Some(Connection {
                device_id,
                device_name: "Test output".into(),
                controller,
            }))
        }
    }
    impl Drop for TestPlatform {
        fn drop(&mut self) {
            self.dropped.send(thread::current().id()).unwrap();
        }
    }

    let (sender, receiver) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    let owner_shared = Arc::clone(&shared);
    let (dropped, teardown) = mpsc::channel();
    let worker = thread::spawn(move || {
        let platform = TestPlatform {
            steps: [Ok(48_000.0), Err("disconnected".into()), Ok(44_100.0)].into(),
            processor: None,
            dropped,
        };
        let session = Session::new(MemoryPersistence::default(), BundledCatalog)
            .unwrap()
            .with_audio(platform, Arc::new(|| {}));
        run(session, receiver, &owner_shared);
    });
    let owner = worker.thread().id();
    let engine = Engine {
        sender,
        command_sequence: Mutex::new(0),
        shared,
        worker: Mutex::new(Some(worker)),
    };
    wait_for(&engine, |s| s.audio_generation == 1);
    engine.set_telemetry_enabled(true).unwrap();
    wait_for(&engine, |s| s.revision == 2);
    assert!(
        engine
            .shared
            .measurements
            .lock()
            .unwrap()
            .subscription
            .is_some()
    );
    engine.sender.send(Message::Refresh).unwrap();
    let lost = wait_for(&engine, |s| s.audio_error.is_some());
    assert_eq!(lost.audio_generation, 2);
    assert!(lost.accepted_chain.is_none());
    assert!(!lost.can_create_profile);
    assert!(
        engine
            .shared
            .measurements
            .lock()
            .unwrap()
            .subscription
            .is_none()
    );
    let recovered = wait_for(&engine, |s| s.audio_generation == 3);
    assert_eq!(recovered.sample_rate, 44_100.0);
    assert!(recovered.audio_error.is_none());
    assert!(recovered.can_create_profile);
    let measurements = engine.shared.measurements.lock().unwrap();
    assert_eq!(measurements.generation, 3);
    assert!(measurements.subscription.is_some());
    drop(measurements);
    engine.set_telemetry_enabled(false).unwrap();
    wait_for(&engine, |s| s.revision > recovered.revision);
    assert!(
        engine
            .shared
            .measurements
            .lock()
            .unwrap()
            .subscription
            .is_none()
    );
    engine.shutdown();
    assert_eq!(
        teardown.recv_timeout(Duration::from_secs(1)).unwrap(),
        owner
    );
    assert!(engine.snapshot().is_none());
}

#[test]
fn profile_commands_publish_device_scoped_summaries_and_selection() {
    let engine = Engine::new(false, None).unwrap();
    let initial = wait_for(&engine, |_| true);
    assert_eq!(initial.device_id, "system-output");
    assert!(initial.can_create_profile);
    assert!(initial.profiles.is_empty());
    assert!(initial.selected_profile_id.is_none());
    let receipt = engine.enqueue(create_flat("First")).unwrap();
    let first = wait_for(&engine, |s| s.processed_command >= receipt);
    let first_id = first.selected_profile_id.unwrap();
    assert_eq!(first.profiles.len(), 1);
    assert_eq!(first.profiles[0].id, first_id);
    assert_eq!(first.profiles[0].name, "First");
    let receipt = engine.enqueue(create_flat("Second")).unwrap();
    let second = wait_for(&engine, |s| s.processed_command >= receipt);
    assert_eq!(second.profiles.len(), 2);
    assert_ne!(second.selected_profile_id.as_ref(), Some(&first_id));
    let receipt = engine
        .enqueue(EngineCommand::SelectProfile {
            device: second.device_id.clone(),
            profile: first_id.clone(),
        })
        .unwrap();
    let selected = wait_for(&engine, |s| s.processed_command >= receipt);
    assert_eq!(selected.selected_profile_id, Some(first_id.clone()));
    assert_eq!(selected.profile_name.as_deref(), Some("First"));
    assert!(selected.preset_id.is_none());
    let receipt = engine
        .enqueue(EngineCommand::CreateProfile {
            device: "stale-device".into(),
            name: "Third".into(),
            source: ProfileSource::Flat,
        })
        .unwrap();
    let rejected = wait_for(&engine, |s| s.processed_command >= receipt);
    assert!(rejected.action_error.is_some());
    assert_eq!(rejected.profiles.len(), 2);
    assert_eq!(rejected.selected_profile_id, Some(first_id));
    engine.shutdown();
}
