use std::fmt;

/// Errors shared across Logbox crates.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn config(msg: impl fmt::Display) -> Self {
        Self::Config(msg.to_string())
    }
}

impl From<figment::Error> for Error {
    fn from(err: figment::Error) -> Self {
        Self::Config(err.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
