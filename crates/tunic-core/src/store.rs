use crate::{Chain, DeviceProfileSelection, ProfileId, ProfileName, ProfileRevision};

/// Persistence boundary for Tunic's durable product state.
///
/// Implementations load and atomically replace the complete durable state.
/// Domain validation and command handling remain the backend's responsibility.
pub trait Store: Send {
    fn load(&mut self) -> Result<Option<DurableState>, StoreError>;
    fn save(&mut self, state: &DurableState) -> Result<(), StoreError>;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Unavailable { message: String },
    Corrupt { message: String },
}
