use crate::{DeviceId, PresetId, ProfileId, ProfileName, State, Store, StoreError};

/// Applies product commands and owns Tunic's authoritative state.
///
/// The backend is deliberately opaque until its command and persistence
/// behavior is implemented.
pub struct Backend {
    _store: Box<dyn Store>,
    _state: State,
}

/// A rejected command or failed backend operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    ProfileNotFound(ProfileId),
    PresetNotFound(PresetId),
    ProfileNameAlreadyExists {
        device: DeviceId,
        name: ProfileName,
    },
    ProfileDeviceMismatch {
        profile: ProfileId,
        profile_device: DeviceId,
        requested_device: DeviceId,
    },
    StoreFailed(StoreError),
}
