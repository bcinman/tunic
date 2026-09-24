use std::fmt;
use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use tunic_dsp::Equalizer;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_PROFILE_ID: &str = "default";

pub(crate) struct StoredProfile {
    pub(crate) id: String,
    pub(crate) equalizer: Equalizer,
    pub(crate) revision: u64,
}

pub(crate) struct ProfileStore {
    connection: Connection,
}

impl ProfileStore {
    pub(crate) fn open(path: &Path) -> Result<Self, PersistenceError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                PersistenceError(format!(
                    "create data directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let connection = Connection::open(path).map_err(|error| {
            PersistenceError(format!("open profile database {}: {error}", path.display()))
        })?;
        Self::initialize(connection)
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, PersistenceError> {
        Self::initialize(Connection::open_in_memory().map_err(|error| {
            PersistenceError(format!("open in-memory profile database: {error}"))
        })?)
    }

    fn initialize(mut connection: Connection) -> Result<Self, PersistenceError> {
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(database_error("enable foreign keys"))?;
        let version = connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .map_err(database_error("read schema version"))?;
        match version {
            0 => Self::create_schema(&mut connection)?,
            SCHEMA_VERSION => {}
            version => {
                return Err(PersistenceError(format!(
                    "unsupported profile database schema version {version}"
                )));
            }
        }
        Ok(Self { connection })
    }

    fn create_schema(connection: &mut Connection) -> Result<(), PersistenceError> {
        let equalizer_json = Equalizer::identity()
            .to_canonical_json()
            .map_err(|error| PersistenceError(error.to_string()))?;
        let transaction = connection
            .transaction()
            .map_err(database_error("begin schema migration"))?;
        transaction
            .execute_batch(
                "CREATE TABLE profiles (
                    id             TEXT PRIMARY KEY,
                    name           TEXT NOT NULL CHECK (length(trim(name)) > 0),
                    equalizer_json TEXT NOT NULL,
                    revision       INTEGER NOT NULL CHECK (revision >= 0)
                );
                CREATE TABLE app_state (
                    singleton          INTEGER PRIMARY KEY CHECK (singleton = 1),
                    default_profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE RESTRICT
                );
                CREATE TABLE device_profile_assignments (
                    device_id TEXT PRIMARY KEY,
                    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE
                );",
            )
            .map_err(database_error("create profile schema"))?;
        transaction
            .execute(
                "INSERT INTO profiles (id, name, equalizer_json, revision)
                 VALUES (?1, 'Default', ?2, 0)",
                params![DEFAULT_PROFILE_ID, equalizer_json],
            )
            .map_err(database_error("create default profile"))?;
        transaction
            .execute(
                "INSERT INTO app_state (singleton, default_profile_id) VALUES (1, ?1)",
                [DEFAULT_PROFILE_ID],
            )
            .map_err(database_error("select default profile"))?;
        transaction
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(database_error("record schema version"))?;
        transaction
            .commit()
            .map_err(database_error("commit schema migration"))
    }

    pub(crate) fn load_default_profile(&self) -> Result<StoredProfile, PersistenceError> {
        self.load_profile(
            "SELECT p.id, p.equalizer_json, p.revision
             FROM profiles p
             JOIN app_state a ON a.default_profile_id = p.id
             WHERE a.singleton = 1",
            [],
        )
    }

    pub(crate) fn load_profile_for_device(
        &self,
        device_id: &str,
    ) -> Result<StoredProfile, PersistenceError> {
        self.load_profile(
            "SELECT p.id, p.equalizer_json, p.revision
             FROM profiles p
             WHERE p.id = COALESCE(
                 (SELECT profile_id FROM device_profile_assignments WHERE device_id = ?1),
                 (SELECT default_profile_id FROM app_state WHERE singleton = 1)
             )",
            [device_id],
        )
    }

    fn load_profile<P>(&self, sql: &str, parameters: P) -> Result<StoredProfile, PersistenceError>
    where
        P: rusqlite::Params,
    {
        let row = self
            .connection
            .query_row(sql, parameters, |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .optional()
            .map_err(database_error("load active profile"))?
            .ok_or_else(|| PersistenceError("profile database has no active profile".into()))?;
        let revision = u64::try_from(row.2)
            .map_err(|_| PersistenceError("profile revision is out of range".into()))?;
        let equalizer = Equalizer::parse_json(&row.1).map_err(|error| {
            PersistenceError(format!(
                "profile {} has an invalid equalizer: {error}",
                row.0
            ))
        })?;
        Ok(StoredProfile {
            id: row.0,
            equalizer,
            revision,
        })
    }

    pub(crate) fn save_profile(
        &mut self,
        profile_id: &str,
        equalizer: &Equalizer,
        expected_revision: u64,
    ) -> Result<u64, PersistenceError> {
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| PersistenceError("profile revision overflow".into()))?;
        let expected_revision = i64::try_from(expected_revision)
            .map_err(|_| PersistenceError("profile revision is out of range".into()))?;
        let next_revision_sql = i64::try_from(next_revision)
            .map_err(|_| PersistenceError("profile revision is out of range".into()))?;
        let equalizer_json = equalizer
            .to_canonical_json()
            .map_err(|error| PersistenceError(error.to_string()))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin profile save"))?;
        let changed = transaction
            .execute(
                "UPDATE profiles
                 SET equalizer_json = ?1, revision = ?2
                 WHERE id = ?3 AND revision = ?4",
                params![
                    equalizer_json,
                    next_revision_sql,
                    profile_id,
                    expected_revision
                ],
            )
            .map_err(database_error("save profile"))?;
        if changed != 1 {
            return Err(PersistenceError(format!(
                "profile {profile_id} changed since it was loaded"
            )));
        }
        transaction
            .commit()
            .map_err(database_error("commit profile save"))?;
        Ok(next_revision)
    }
}

#[derive(Debug)]
pub(crate) struct PersistenceError(String);

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for PersistenceError {}

fn database_error(context: &'static str) -> impl FnOnce(rusqlite::Error) -> PersistenceError {
    move |error| PersistenceError(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};

    use super::ProfileStore;

    #[test]
    fn creates_an_identity_default_profile_and_saves_it_optimistically() {
        let mut store = ProfileStore::open_in_memory().unwrap();
        let initial = store.load_default_profile().unwrap();
        let equalizer = test_equalizer(4.5);

        assert_eq!(initial.id, "default");
        assert_eq!(initial.equalizer, Equalizer::identity());
        assert_eq!(initial.revision, 0);

        assert_eq!(store.save_profile("default", &equalizer, 0).unwrap(), 1);
        let saved = store.load_default_profile().unwrap();
        assert_eq!(saved.equalizer, equalizer);
        assert_eq!(saved.revision, 1);
        assert!(
            store
                .save_profile("default", &Equalizer::identity(), 0)
                .is_err()
        );
    }

    #[test]
    fn resolves_a_device_assignment_before_the_default_profile() {
        let store = ProfileStore::open_in_memory().unwrap();
        let equalizer = test_equalizer(-3.0);
        store
            .connection
            .execute(
                "INSERT INTO profiles (id, name, equalizer_json, revision)
                 VALUES ('headphones', 'Headphones', ?1, 7)",
                [equalizer.to_canonical_json().unwrap()],
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO device_profile_assignments (device_id, profile_id)
                 VALUES (?1, 'headphones')",
                params!["device-1"],
            )
            .unwrap();

        let assigned = store.load_profile_for_device("device-1").unwrap();
        let fallback = store.load_profile_for_device("unassigned").unwrap();

        assert_eq!(assigned.id, "headphones");
        assert_eq!(assigned.equalizer, equalizer);
        assert_eq!(assigned.revision, 7);
        assert_eq!(fallback.id, "default");
    }

    fn test_equalizer(gain_db: f64) -> Equalizer {
        Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ))
    }
}
