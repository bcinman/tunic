use super::SqlitePersistence;
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tunic_core::*;
use tunic_presets::BundledCatalog;

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "tunic-sqlite-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        Self(directory.join("session.sqlite3"))
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}

fn saved_state() -> State {
    let filters = [
        FilterKind::HighShelf,
        FilterKind::Peaking,
        FilterKind::LowShelf,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| Filter {
        id: FilterId::try_new(9 - index as u32).unwrap(),
        parameters: FilterParameters {
            kind,
            frequency: FrequencyHz::try_new(123.45678901234567 * (index + 1) as f64).unwrap(),
            gain: GainDb::try_new(-2.3 + index as f64).unwrap(),
            quality_factor: QualityFactor::try_new(0.7312345678901234).unwrap(),
        },
    })
    .collect();
    let mut profile = Profile::new(
        ProfileId::try_new("custom").unwrap(),
        ProfileName::try_new("Studio 🎧").unwrap(),
        Chain {
            equalizer: Equalizer {
                preamp: GainDb::try_new(-6.25).unwrap(),
                filters,
            },
        },
        vec![FilterControl::new(
            FilterId::try_new(8).unwrap(),
            FilterControlName::try_new("Tone").unwrap(),
        )],
        Some(PresetOrigin {
            id: PresetId::try_new("source").unwrap(),
            revision: PresetRevision::try_new("rev-3").unwrap(),
            attribution: Attribution {
                provider: "Provider".into(),
                measurement_source: Some("Measurement".into()),
                source_url: "https://example.com/source".into(),
            },
        }),
    )
    .unwrap();
    profile
        .adjust_filter_gain(
            FilterId::try_new(8).unwrap(),
            GainDb::try_new(3.75).unwrap(),
        )
        .unwrap();
    let flat = Profile::new(
        ProfileId::try_new("flat").unwrap(),
        ProfileName::try_new("Flat").unwrap(),
        Chain::default(),
        vec![],
        None,
    )
    .unwrap();
    State {
        selections: vec![
            DeviceProfileSelection {
                device: DeviceId::try_new("system-output").unwrap(),
                profile: profile.id().clone(),
            },
            DeviceProfileSelection {
                device: DeviceId::try_new("headphones").unwrap(),
                profile: flat.id().clone(),
            },
        ],
        profiles: vec![profile, flat],
    }
}

#[test]
fn disk_snapshot_round_trips_and_replacement_removes_old_state() {
    let database = Database::new();
    let expected = saved_state();
    let mut store = SqlitePersistence::open(&database.0).unwrap();
    assert_eq!(store.load().unwrap(), None);
    store.save(&expected).unwrap();
    drop(store);
    let mut reopened = SqlitePersistence::open(&database.0).unwrap();
    assert_eq!(reopened.load().unwrap(), Some(expected));
    reopened.save(&State::default()).unwrap();
    drop(reopened);
    assert_eq!(
        SqlitePersistence::open(&database.0)
            .unwrap()
            .load()
            .unwrap(),
        Some(State::default())
    );
}

#[test]
fn session_restores_saved_adjustments_but_not_unsaved_drafts() {
    let database = Database::new();
    SqlitePersistence::open(&database.0)
        .unwrap()
        .save(&saved_state())
        .unwrap();
    let mut session = Session::new(
        SqlitePersistence::open(&database.0).unwrap(),
        BundledCatalog,
    )
    .unwrap();
    session
        .execute(Command::SetControlGain {
            filter: FilterId::try_new(8).unwrap(),
            gain: GainDb::try_new(-4.5).unwrap(),
        })
        .unwrap();
    session.execute(Command::SaveDraft).unwrap();
    let saved = session.state().clone();
    session
        .execute(Command::SetControlGain {
            filter: FilterId::try_new(8).unwrap(),
            gain: GainDb::try_new(7.0).unwrap(),
        })
        .unwrap();
    drop(session);
    let restored = Session::new(
        SqlitePersistence::open(&database.0).unwrap(),
        BundledCatalog,
    )
    .unwrap();
    assert_eq!(restored.state(), &saved);
    assert_eq!(
        restored.selected_profile().unwrap().controls()[0]
            .gain_adjustment()
            .into_inner(),
        -4.5
    );
}

#[test]
fn failed_save_keeps_previous_snapshot_on_disk() {
    let database = Database::new();
    let expected = saved_state();
    let mut store = SqlitePersistence::open(&database.0).unwrap();
    store.save(&expected).unwrap();
    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_save BEFORE UPDATE ON snapshot
        BEGIN SELECT RAISE(ABORT, 'disk write rejected'); END;",
        )
        .unwrap();
    assert!(store.save(&State::default()).is_err());
    drop(store);
    assert_eq!(
        SqlitePersistence::open(&database.0)
            .unwrap()
            .load()
            .unwrap(),
        Some(expected)
    );
}

#[test]
fn invalid_payloads_and_domain_values_are_not_silently_reset() {
    let database = Database::new();
    let mut store = SqlitePersistence::open(&database.0).unwrap();
    let connection = Connection::open(&database.0).unwrap();
    store.save(&saved_state()).unwrap();
    let payload: String = connection
        .query_row("SELECT payload FROM snapshot", [], |row| row.get(0))
        .unwrap();
    for (pointer, value) in [
        ("/profiles/0/filters/0/frequency", serde_json::json!(0)),
        ("/profiles/0/controls/0/target", serde_json::json!(123)),
        ("/profiles/0/id", serde_json::json!("")),
        ("/selections/0/profile", serde_json::json!("missing")),
    ] {
        let mut document: serde_json::Value = serde_json::from_str(&payload).unwrap();
        *document.pointer_mut(pointer).unwrap() = value;
        connection
            .execute("UPDATE snapshot SET payload = ?1", [document.to_string()])
            .unwrap();
        assert!(matches!(
            Session::new(
                SqlitePersistence::open(&database.0).unwrap(),
                BundledCatalog
            ),
            Err(SessionError::PersistenceFailed(
                PersistenceError::Corrupt { .. }
            ))
        ));
    }
    connection
        .execute("UPDATE snapshot SET payload = 'broken json'", [])
        .unwrap();
    assert!(matches!(
        store.load(),
        Err(PersistenceError::Corrupt { .. })
    ));
    let unchanged: String = connection
        .query_row("SELECT payload FROM snapshot", [], |row| row.get(0))
        .unwrap();
    assert_eq!(unchanged, "broken json");
}

#[test]
fn newer_schema_is_rejected_without_modifying_database() {
    let database = Database::new();
    SqlitePersistence::open(&database.0)
        .unwrap()
        .save(&saved_state())
        .unwrap();
    let connection = Connection::open(&database.0).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    let before = std::fs::read(&database.0).unwrap();
    assert!(matches!(
        SqlitePersistence::open(&database.0),
        Err(PersistenceError::Unavailable { .. })
    ));
    assert_eq!(std::fs::read(&database.0).unwrap(), before);
}

#[test]
fn failed_initial_migration_leaves_existing_data_and_version_unchanged() {
    let database = Database::new();
    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE snapshot (existing TEXT);
        INSERT INTO snapshot VALUES ('preserve me');",
        )
        .unwrap();
    assert!(SqlitePersistence::open(&database.0).is_err());
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
    let existing: String = connection
        .query_row("SELECT existing FROM snapshot", [], |row| row.get(0))
        .unwrap();
    assert_eq!(existing, "preserve me");
}
