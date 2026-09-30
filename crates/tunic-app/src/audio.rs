use std::error::Error;
use std::sync::Arc;

use tunic_core::{Chain, Controller};

pub type ChangeHandler = Arc<dyn Fn() + Send + Sync + 'static>;

/// A newly established system-audio route.
pub struct Connection {
    pub device_name: String,
    pub controller: Controller,
}

/// Owns a platform's native system-audio route and its lifecycle.
pub trait Platform {
    type Error: Error;

    /// Registers a callback for changes to the system's default output.
    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), Self::Error>;

    /// Ensures audio is connected to the current default output.
    ///
    /// The first call connects, an unchanged route returns `None`, and a
    /// changed route is rebuilt with `active_chain` before being returned.
    /// After an error, the next call retries from a disconnected state.
    fn refresh_default_output(
        &mut self,
        active_chain: &Chain,
    ) -> Result<Option<Connection>, Self::Error>;
}
