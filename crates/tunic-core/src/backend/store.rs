//! Persistence boundary for complete backend state snapshots.
//!
//! Store implementations own storage mechanics only. Backend validation and
//! command policy remain outside this module.

use super::State;

/// Persistence boundary for Tunic's durable product state.
///
/// Implementations load and atomically replace the complete durable state.
/// Domain validation and command handling remain the backend's responsibility.
pub trait Store: Send {
    fn load(&mut self) -> Result<Option<State>, StoreError>;
    fn save(&mut self, state: &State) -> Result<(), StoreError>;
}

/// An in-memory store for tests and non-persistent hosts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryStore {
    state: Option<State>,
}

impl MemoryStore {
    #[must_use]
    pub fn new(state: State) -> Self {
        Self { state: Some(state) }
    }
}

impl Store for MemoryStore {
    fn load(&mut self) -> Result<Option<State>, StoreError> {
        Ok(self.state.clone())
    }

    fn save(&mut self, state: &State) -> Result<(), StoreError> {
        self.state = Some(state.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Unavailable { message: String },
    Corrupt { message: String },
}

#[cfg(test)]
mod tests {
    use super::{MemoryStore, Store};
    use crate::State;

    #[test]
    fn memory_store_starts_empty_and_replaces_its_state() {
        let mut store = MemoryStore::default();
        assert_eq!(store.load().unwrap(), None);

        let state = State::default();
        store.save(&state).unwrap();

        assert_eq!(store.load().unwrap(), Some(state));
    }
}
