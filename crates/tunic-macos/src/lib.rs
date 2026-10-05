//! Minimal macOS system-output route for Tunic's portable processor.

mod devices;
mod platform;
mod route;

use std::fmt;

use objc2_core_audio::{
    AudioObjectID, AudioObjectPropertyAddress, kAudioObjectPropertyElementMain,
};
use tunic_core::{Chain, Controller};

use crate::devices::{DefaultOutputWatcher, default_output_id, device_name, sample_rate};
use crate::route::Route;

pub use crate::platform::MacosPlatform;

/// Owns the Core Audio resources that keep system-output processing active.
struct AudioSession {
    output_id: AudioObjectID,
    device_name: String,
    _route: Route,
}

impl AudioSession {
    /// Starts processing the current default output with `initial_chain`.
    fn start(initial_chain: Chain) -> Result<(Self, Controller), Error> {
        let output = default_output_id()?;
        let device_name = device_name(output)?;
        let (route, controller) = Route::start(output, sample_rate(output)?, initial_chain)?;
        Ok((
            Self {
                output_id: output,
                device_name,
                _route: route,
            },
            controller,
        ))
    }

    #[must_use]
    fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Whether this session still targets the system's default output.
    fn is_current_default_output(&self) -> Result<bool, Error> {
        Ok(self.output_id == default_output_id()?)
    }
}

#[derive(Debug)]
struct Error(String);

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for Error {}

pub(crate) const fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

pub(crate) fn check_status(operation: &str, status: i32) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(status_error(operation, status))
    }
}

pub(crate) fn status_error(operation: &str, status: i32) -> Error {
    let bytes = (status as u32).to_be_bytes();
    let detail = if bytes.iter().all(u8::is_ascii_graphic) {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    };
    Error::new(format!("{operation} failed with OSStatus {detail}"))
}

pub(crate) fn with_cleanup_error(primary: Error, cleanup: Result<(), Error>) -> Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => Error::new(format!("{primary}; cleanup also failed: {cleanup}")),
    }
}
