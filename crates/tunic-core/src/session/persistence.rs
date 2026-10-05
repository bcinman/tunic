//! Persistence boundary for complete session state snapshots.
//!
//! Persistence implementations own storage mechanics only. Session validation and
//! command policy remain outside this module.

use super::State;

/// Persistence boundary for Tunic's durable product state.
///
/// Implementations load and atomically replace the complete durable state.
/// Domain validation and command handling remain the session's responsibility.
pub trait Persistence: Send {
    fn load(&mut self) -> Result<Option<State>, PersistenceError>;
    fn save(&mut self, state: &State) -> Result<(), PersistenceError>;
}

/// An in-memory store for tests and non-persistent hosts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryPersistence {
    state: Option<State>,
}

impl MemoryPersistence {
    #[must_use]
    pub fn new(state: State) -> Self {
        Self { state: Some(state) }
    }
}

impl Persistence for MemoryPersistence {
    fn load(&mut self) -> Result<Option<State>, PersistenceError> {
        Ok(self.state.clone())
    }

    fn save(&mut self, state: &State) -> Result<(), PersistenceError> {
        self.state = Some(state.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistenceError {
    Unavailable { message: String },
    Corrupt { message: String },
}

#[cfg(test)]
mod tests {
    use super::{MemoryPersistence, Persistence};
    use crate::State;

    #[test]
    fn memory_store_starts_empty_and_replaces_its_state() {
        let mut store = MemoryPersistence::default();
        assert_eq!(store.load().unwrap(), None);

        let state = State::default();
        store.save(&state).unwrap();

        assert_eq!(store.load().unwrap(), Some(state));
    }
}
