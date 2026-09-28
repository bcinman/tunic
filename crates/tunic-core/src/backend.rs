use crate::{State, Store};

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
pub struct BackendError {
    pub message: String,
}
