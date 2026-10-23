//! Shared building blocks for every Logbox crate: configuration, the event
//! schema, error types and storage helpers.

pub mod config;
pub mod error;
pub mod telemetry;

pub use config::Config;
pub use error::{Error, Result};
