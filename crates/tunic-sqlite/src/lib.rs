//! SQLite storage for complete committed session snapshots.
//!
//! The schema version covers both SQL tables and the JSON snapshot format.
//! Append migrations; never edit a released migration. A format change must
//! migrate existing payloads as well as increment `user_version`. All pending
//! migrations and the version update commit in one exclusive transaction.
//! Newer databases are rejected without being downgraded or reset.
//! Migration SQL must not contain its own transaction control statements.
//!
//! Hosts choose the database path and create its parent directory. This adapter
//! performs blocking I/O and must not run on an audio callback. Like the core
//! snapshot contract, saves are last-writer-wins, not concurrent editor merges.

mod snapshot;

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use tunic_core::{Persistence, PersistenceError, State};

const MIGRATIONS: &[&str] = &[include_str!("migrations/001_snapshot.sql")];

pub struct SqlitePersistence {
    connection: Connection,
}

impl SqlitePersistence {
    /// Opens or creates a database and upgrades its schema before use.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        let mut connection = Connection::open(path).map_err(storage_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage_error)?;
        // Do not acknowledge a save before SQLite has synced its commit.
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage_error)?;
        // macOS needs F_FULLFSYNC to flush drive caches; ignored elsewhere.
        connection
            .pragma_update(None, "fullfsync", true)
            .map_err(storage_error)?;
        migrate(&mut connection, MIGRATIONS)?;
        Ok(Self { connection })
    }
}

impl Persistence for SqlitePersistence {
    fn load(&mut self) -> Result<Option<State>, PersistenceError> {
        let payload: Option<String> = self
            .connection
            .query_row("SELECT payload FROM snapshot WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        payload.as_deref().map(snapshot::decode).transpose()
    }

    fn save(&mut self, state: &State) -> Result<(), PersistenceError> {
        let payload = snapshot::encode(state)?;
        // A single statement is an atomic SQLite transaction, including replacement.
        self.connection
            .execute(
                "INSERT INTO snapshot (id, payload) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                [payload],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

fn migrate(connection: &mut Connection, migrations: &[&str]) -> Result<(), PersistenceError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Exclusive)
        .map_err(storage_error)?;
    let version: u32 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(storage_error)?;
    let version = version as usize;
    if version > migrations.len() {
        return Err(PersistenceError::Unavailable {
            message: format!(
                "Database schema version {version} is newer than supported version {}; update Tunic",
                migrations.len()
            ),
        });
    }
    if version == migrations.len() {
        return Ok(());
    }
    for migration in &migrations[version..] {
        transaction
            .execute_batch(migration)
            .map_err(storage_error)?;
    }
    transaction
        .pragma_update(None, "user_version", migrations.len() as u32)
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn storage_error(error: rusqlite::Error) -> PersistenceError {
    let message = error.to_string();
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            PersistenceError::Corrupt { message }
        }
        _ => PersistenceError::Unavailable { message },
    }
}

#[cfg(test)]
mod tests;
