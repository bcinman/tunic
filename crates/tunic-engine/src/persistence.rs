use std::fmt;
use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use tunic_dsp::Equalizer;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_PROFILE_ID: &str = "default";

pub(crate) struct StoredProfile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) equalizer: Equalizer,
    pub(crate) revision: u64,
}

pub(crate) struct StoredAssignment {
    pub(crate) device_id: String,
    pub(crate) profile_id: String,
}

pub(crate) struct StoredCatalog {
    pub(crate) profiles: Vec<StoredProfile>,
    pub(crate) default_profile_id: String,
    pub(crate) assignments: Vec<StoredAssignment>,
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
            "SELECT p.id, p.name, p.equalizer_json, p.revision
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
            "SELECT p.id, p.name, p.equalizer_json, p.revision
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
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .optional()
            .map_err(database_error("load active profile"))?
            .ok_or_else(|| PersistenceError("profile database has no active profile".into()))?;
        let revision = u64::try_from(row.3)
            .map_err(|_| PersistenceError("profile revision is out of range".into()))?;
        let equalizer = Equalizer::parse_json(&row.2).map_err(|error| {
            PersistenceError(format!(
                "profile {} has an invalid equalizer: {error}",
                row.0
            ))
        })?;
        Ok(StoredProfile {
            id: row.0,
            name: row.1,
            equalizer,
            revision,
        })
    }

    pub(crate) fn load_profile_by_id(
        &self,
        profile_id: &str,
    ) -> Result<StoredProfile, PersistenceError> {
        self.load_profile(
            "SELECT id, name, equalizer_json, revision FROM profiles WHERE id = ?1",
            [profile_id],
        )
    }

    pub(crate) fn default_profile_id(&self) -> Result<String, PersistenceError> {
        self.connection
            .query_row(
                "SELECT default_profile_id FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(database_error("load default profile selection"))
    }

    pub(crate) fn catalog(&self) -> Result<StoredCatalog, PersistenceError> {
        read_catalog(&self.connection)
    }

    pub(crate) fn create_and_select_profile(
        &mut self,
        name: &str,
        equalizer: &Equalizer,
        active_device_id: &str,
    ) -> Result<(StoredProfile, StoredCatalog), PersistenceError> {
        let name = validate_name(name)?;
        self.ensure_name_available(name, None)?;
        let id = self.available_profile_id(name)?;
        let equalizer_json = equalizer
            .to_canonical_json()
            .map_err(|error| PersistenceError(error.to_string()))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin profile creation"))?;
        transaction
            .execute(
                "INSERT INTO profiles (id, name, equalizer_json, revision)
                 VALUES (?1, ?2, ?3, 0)",
                params![id, name, equalizer_json],
            )
            .map_err(database_error("create profile"))?;
        transaction
            .execute(
                "UPDATE app_state SET default_profile_id = ?1 WHERE singleton = 1",
                [&id],
            )
            .map_err(database_error("select created profile"))?;
        transaction
            .execute(
                "DELETE FROM device_profile_assignments WHERE device_id = ?1",
                [active_device_id],
            )
            .map_err(database_error("clear active device profile assignment"))?;
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit profile creation"))?;
        Ok((
            StoredProfile {
                id,
                name: name.to_owned(),
                equalizer: equalizer.clone(),
                revision: 0,
            },
            catalog,
        ))
    }

    pub(crate) fn rename_profile(
        &mut self,
        profile_id: &str,
        name: &str,
    ) -> Result<StoredCatalog, PersistenceError> {
        let name = validate_name(name)?;
        self.ensure_name_available(name, Some(profile_id))?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin profile rename"))?;
        let changed = transaction
            .execute(
                "UPDATE profiles SET name = ?1 WHERE id = ?2",
                params![name, profile_id],
            )
            .map_err(database_error("rename profile"))?;
        changed_profile(changed, profile_id)?;
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit profile rename"))?;
        Ok(catalog)
    }

    pub(crate) fn select_default_profile(
        &mut self,
        profile_id: &str,
        active_device_id: &str,
    ) -> Result<StoredCatalog, PersistenceError> {
        self.load_profile_by_id(profile_id)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin profile selection"))?;
        transaction
            .execute(
                "UPDATE app_state SET default_profile_id = ?1 WHERE singleton = 1",
                [profile_id],
            )
            .map_err(database_error("select default profile"))?;
        transaction
            .execute(
                "DELETE FROM device_profile_assignments WHERE device_id = ?1",
                [active_device_id],
            )
            .map_err(database_error("clear active device profile assignment"))?;
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit profile selection"))?;
        Ok(catalog)
    }

    pub(crate) fn assign_profile(
        &mut self,
        device_id: &str,
        profile_id: &str,
    ) -> Result<StoredCatalog, PersistenceError> {
        self.load_profile_by_id(profile_id)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin device profile assignment"))?;
        transaction
            .execute(
                "INSERT INTO device_profile_assignments (device_id, profile_id)
                 VALUES (?1, ?2)
                 ON CONFLICT(device_id) DO UPDATE SET profile_id = excluded.profile_id",
                params![device_id, profile_id],
            )
            .map_err(database_error("assign profile to device"))?;
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit device profile assignment"))?;
        Ok(catalog)
    }

    pub(crate) fn delete_profile(
        &mut self,
        profile_id: &str,
    ) -> Result<StoredCatalog, PersistenceError> {
        if self.default_profile_id()? == profile_id {
            return Err(PersistenceError(
                "cannot delete the default profile; select another profile first".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin profile deletion"))?;
        let changed = transaction
            .execute("DELETE FROM profiles WHERE id = ?1", [profile_id])
            .map_err(database_error("delete profile"))?;
        changed_profile(changed, profile_id)?;
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit profile deletion"))?;
        Ok(catalog)
    }

    fn ensure_name_available(
        &self,
        name: &str,
        except_profile_id: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let existing = self
            .connection
            .query_row(
                "SELECT id FROM profiles WHERE name = ?1 COLLATE NOCASE",
                [name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("check profile name"))?;
        if existing
            .as_deref()
            .is_some_and(|id| Some(id) != except_profile_id)
        {
            return Err(PersistenceError(format!(
                "a profile named '{name}' already exists"
            )));
        }
        Ok(())
    }

    fn available_profile_id(&self, name: &str) -> Result<String, PersistenceError> {
        let base = profile_id_base(name);
        for suffix in 1_u64.. {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            let exists = self
                .connection
                .query_row("SELECT 1 FROM profiles WHERE id = ?1", [&candidate], |_| {
                    Ok(())
                })
                .optional()
                .map_err(database_error("check profile identifier"))?
                .is_some();
            if !exists {
                return Ok(candidate);
            }
        }
        unreachable!("profile identifier suffixes are unbounded")
    }

    pub(crate) fn save_profile(
        &mut self,
        profile_id: &str,
        equalizer: &Equalizer,
        expected_revision: u64,
    ) -> Result<(u64, StoredCatalog), PersistenceError> {
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
        let catalog = read_catalog(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit profile save"))?;
        Ok((next_revision, catalog))
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

fn read_catalog(connection: &Connection) -> Result<StoredCatalog, PersistenceError> {
    let mut profile_statement = connection
        .prepare(
            "SELECT id, name, equalizer_json, revision
             FROM profiles
             ORDER BY name COLLATE NOCASE, id",
        )
        .map_err(database_error("prepare profile catalog"))?;
    let profile_rows = profile_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(database_error("read profile catalog"))?;
    let profiles = profile_rows
        .map(|row| {
            let (id, name, equalizer_json, revision) =
                row.map_err(database_error("read profile catalog row"))?;
            let revision = u64::try_from(revision)
                .map_err(|_| PersistenceError("profile revision is out of range".into()))?;
            let equalizer = Equalizer::parse_json(&equalizer_json).map_err(|error| {
                PersistenceError(format!("profile {id} has an invalid equalizer: {error}"))
            })?;
            Ok(StoredProfile {
                id,
                name,
                equalizer,
                revision,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(profile_statement);

    let default_profile_id = connection
        .query_row(
            "SELECT default_profile_id FROM app_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(database_error("read default profile from catalog"))?;

    let mut assignment_statement = connection
        .prepare(
            "SELECT device_id, profile_id
             FROM device_profile_assignments
             ORDER BY device_id",
        )
        .map_err(database_error("prepare profile assignment catalog"))?;
    let assignments = assignment_statement
        .query_map([], |row| {
            Ok(StoredAssignment {
                device_id: row.get(0)?,
                profile_id: row.get(1)?,
            })
        })
        .map_err(database_error("read profile assignment catalog"))?
        .map(|row| row.map_err(database_error("read profile assignment catalog row")))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(StoredCatalog {
        profiles,
        default_profile_id,
        assignments,
    })
}

fn validate_name(name: &str) -> Result<&str, PersistenceError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(PersistenceError("profile name cannot be empty".into()))
    } else {
        Ok(trimmed)
    }
}

fn changed_profile(changed: usize, profile_id: &str) -> Result<(), PersistenceError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(PersistenceError(format!(
            "profile {profile_id} does not exist"
        )))
    }
}

fn profile_id_base(name: &str) -> String {
    let id = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let id = id.trim_matches('-');
    if id.is_empty() {
        "profile".into()
    } else {
        id.into()
    }
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
        assert_eq!(initial.name, "Default");
        assert_eq!(initial.equalizer, Equalizer::identity());
        assert_eq!(initial.revision, 0);

        assert_eq!(store.save_profile("default", &equalizer, 0).unwrap().0, 1);
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

    #[test]
    fn creates_renames_selects_assigns_and_deletes_profiles() {
        let mut store = ProfileStore::open_in_memory().unwrap();
        let equalizer = test_equalizer(6.0);

        store.assign_profile("device-1", "default").unwrap();
        store.assign_profile("device-2", "default").unwrap();
        let (created, created_catalog) = store
            .create_and_select_profile("  Studio Phones  ", &equalizer, "device-1")
            .unwrap();
        assert_eq!(created.id, "studio-phones");
        assert_eq!(created.name, "Studio Phones");
        assert_eq!(created_catalog.assignments.len(), 1);
        assert_eq!(created_catalog.assignments[0].device_id, "device-2");
        assert_eq!(store.default_profile_id().unwrap(), "studio-phones");
        assert_eq!(
            store.load_profile_for_device("device-1").unwrap().id,
            "studio-phones"
        );

        store.rename_profile("studio-phones", "Headphones").unwrap();
        assert_eq!(
            store.load_profile_by_id("studio-phones").unwrap().name,
            "Headphones"
        );
        assert!(store.rename_profile("studio-phones", "Default").is_err());

        store.assign_profile("device-1", "studio-phones").unwrap();
        store.assign_profile("device-2", "studio-phones").unwrap();
        let selected = store.select_default_profile("default", "device-1").unwrap();
        assert_eq!(selected.assignments.len(), 1);
        assert_eq!(selected.assignments[0].device_id, "device-2");

        store.assign_profile("device-1", "studio-phones").unwrap();
        let assignments = store.catalog().unwrap().assignments;
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].device_id, "device-1");
        assert_eq!(assignments[0].profile_id, "studio-phones");
        assert_eq!(assignments[1].device_id, "device-2");
        assert_eq!(assignments[1].profile_id, "studio-phones");

        store.delete_profile("studio-phones").unwrap();
        let catalog = store.catalog().unwrap();
        assert!(catalog.assignments.is_empty());
        assert_eq!(catalog.profiles.len(), 1);
        assert!(store.delete_profile("default").is_err());
    }

    #[test]
    fn catalog_validation_failure_rolls_back_profile_creation() {
        let mut store = ProfileStore::open_in_memory().unwrap();
        store
            .connection
            .execute(
                "INSERT INTO profiles (id, name, equalizer_json, revision)
                 VALUES ('broken', 'Broken', '{}', 0)",
                [],
            )
            .unwrap();

        assert!(
            store
                .create_and_select_profile("New Profile", &Equalizer::identity(), "device-1")
                .is_err()
        );
        let created = store
            .connection
            .query_row(
                "SELECT count(*) FROM profiles WHERE id = 'new-profile'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(created, 0);
        assert_eq!(store.default_profile_id().unwrap(), "default");
    }

    fn test_equalizer(gain_db: f64) -> Equalizer {
        Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ))
    }
}
