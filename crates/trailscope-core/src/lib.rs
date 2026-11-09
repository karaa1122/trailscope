//! Shared building blocks for every Trailscope crate: configuration, the event
//! schema, error types and storage helpers.

pub mod admin;
pub mod ch;
pub mod config;
pub mod error;
pub mod live;
pub mod pg;
pub mod project;
pub mod rows;
pub mod secrets;
pub mod telemetry;
pub mod testing;
pub mod wire;

pub use config::Config;
pub use error::{Error, Result};
