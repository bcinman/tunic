use crate::{Chain, DeviceProfileSelection, ProfileId, ProfileName, ProfileRevision};

/// Persistence boundary for Tunic's durable product state.
///
/// Implementations load and atomically replace the complete durable state.
/// Domain validation and command handling remain the backend's responsibility.
pub trait Store: Send {
    fn load(&mut self) -> Result<Option<DurableState>, StoreError>;
    fn save(&mut self, state: &DurableState) -> Result<(), StoreError>;
}

/// An in-memory store for tests and non-persistent hosts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryStore {
    state: Option<DurableState>,
}

impl MemoryStore {
    #[must_use]
    pub fn new(state: DurableState) -> Self {
        Self { state: Some(state) }
    }
}

impl Store for MemoryStore {
    fn load(&mut self) -> Result<Option<DurableState>, StoreError> {
        Ok(self.state.clone())
    }

    fn save(&mut self, state: &DurableState) -> Result<(), StoreError> {
        self.state = Some(state.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DurableState {
    pub profiles: Vec<StoredProfile>,
    pub selections: Vec<DeviceProfileSelection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredProfile {
    pub id: ProfileId,
    pub name: ProfileName,
    pub chain: Chain,
    pub revision: ProfileRevision,
}

impl From<&crate::State> for DurableState {
    fn from(state: &crate::State) -> Self {
        Self {
            profiles: state
                .profiles
                .iter()
                .map(|profile| StoredProfile {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                    chain: profile.chain.clone(),
                    revision: profile.revision,
                })
                .collect(),
            selections: state.selections.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Unavailable { message: String },
    Corrupt { message: String },
}

#[cfg(test)]
mod tests {
    use super::{DurableState, MemoryStore, Store};

    #[test]
    fn memory_store_starts_empty_and_replaces_its_state() {
        let mut store = MemoryStore::default();
        assert_eq!(store.load().unwrap(), None);

        let state = DurableState::default();
        store.save(&state).unwrap();

        assert_eq!(store.load().unwrap(), Some(state));
    }
}
