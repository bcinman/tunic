//! GPUI application lifecycle and presentation, wiring `tunic-engine` to the
//! `tunic-macos` platform implementation.

mod application;
mod editor;
mod spectrum;

use std::fmt;
use std::path::PathBuf;

pub struct AppConfig {
    pub data_directory: PathBuf,
}

#[derive(Debug)]
pub struct AppError(String);

impl AppError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for AppError {}

pub fn default_data_directory() -> Result<PathBuf, AppError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| AppError::new("HOME is not set; a data directory is required"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Tunic"))
}

pub use application::run;
