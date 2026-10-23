//! Tracing setup for the `logbox` binary.

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::config::LogFormat;

/// Installs the global tracing subscriber. `RUST_LOG` overrides `level`.
pub fn init(level: &str, format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let registry = tracing_subscriber::registry().with(filter);
    // `try_init` fails only if a subscriber is already installed (e.g. in tests).
    let _ = match format {
        LogFormat::Json => registry
            .with(fmt::layer().json().flatten_event(true))
            .try_init(),
        LogFormat::Pretty => registry.with(fmt::layer().compact()).try_init(),
    };
}
