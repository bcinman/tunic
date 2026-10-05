use super::*;

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
    let engine = Engine::new(false).unwrap();
    let initial = wait_for(&engine, |_| true);
    let preset = initial.presets.iter().find(|p| p.model == "HD650").unwrap();
    engine
        .enqueue(EngineCommand::UsePreset {
            id: preset.id.clone(),
        })
        .unwrap();
    let selected = wait_for(&engine, |s| s.preset_id.as_ref() == Some(&preset.id));
    let filter = selected.base_chain.filters[0].clone();
    engine
        .enqueue(EngineCommand::EditFilter {
            filter: filter.id,
            frequency: 123.0,
            gain: -3.25,
        })
        .unwrap();
    let edited = wait_for(&engine, |s| s.has_draft);
    assert_eq!(edited.base_chain.filters[0].frequency, 123.0);
    assert_eq!(edited.base_chain.filters[0].gain, -3.25);
    assert!(edited.accepted_chain.is_none());

    engine
        .enqueue(EngineCommand::UsePreset {
            id: "missing".into(),
        })
        .unwrap();
    let failed = wait_for(&engine, |s| s.action_error.is_some());
    assert!(failed.has_draft);
    assert_eq!(failed.base_chain.filters[0].gain, -3.25);
    engine.enqueue(EngineCommand::ResetDraft).unwrap();
    let reset = wait_for(&engine, |s| !s.has_draft && s.action_error.is_none());
    assert_eq!(reset.base_chain.filters[0].frequency, filter.frequency);
    assert_eq!(reset.base_chain.filters[0].gain, filter.gain);

    engine
        .enqueue(EngineCommand::EditFilter {
            filter: filter.id,
            frequency: 237.0,
            gain: 2.5,
        })
        .unwrap();
    wait_for(&engine, |s| s.has_draft);
    engine.enqueue(EngineCommand::SaveDraft).unwrap();
    let saved = wait_for(&engine, |s| !s.has_draft);
    assert_eq!(saved.base_chain.filters[0].gain, 2.5);
    assert_eq!(saved.base_chain.filters[0].frequency, 237.0);
    engine.shutdown();
    engine.shutdown();
    assert!(matches!(
        engine.enqueue(EngineCommand::UseFlat),
        Err(EngineError::Closed)
    ));
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
    let engine = Arc::new(Engine::new(false).unwrap());
    wait_for(&engine, |_| true);
    let shared = Arc::clone(&engine.shared);
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let engine = Arc::clone(&engine);
            thread::spawn(move || {
                for _ in 0..25 {
                    engine.enqueue(EngineCommand::UseFlat).unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let snapshot = wait_for(&engine, |s| s.revision == 101);
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
    let engine = Engine::new(false).unwrap();
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
            chain: &tunic_core::Chain,
        ) -> Result<Option<Connection>, String> {
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
