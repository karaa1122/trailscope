//! The `logbox` binary.

use std::path::PathBuf;

use anyhow::Context;
use axum::{Json, Router, routing::get};
use clap::{Parser, Subcommand};
use logbox_core::Config;
use serde_json::json;
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(name = "logbox", version, about = "Self-hosted API request logging")]
struct Cli {
    /// Path to logbox.toml (defaults to ./logbox.toml when present).
    #[arg(long, short, global = true, env = "LOGBOX_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Ingest, API, alerts and UI in one process (default).
    All,
    /// Only the ingest endpoints.
    Ingest,
    /// Only the API and UI.
    Api,
    /// Only the alert evaluator.
    Alerts,
    /// Apply ClickHouse and Postgres migrations, then exit.
    Migrate,
    /// Generate demo traffic.
    Demo,
    /// Probe the local `/healthz` endpoint (used by container healthchecks).
    #[command(hide = true)]
    Healthcheck,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref()).context("loading configuration")?;
    logbox_core::telemetry::init(&config.log.level, config.log.format);

    match cli.command.unwrap_or(Command::All) {
        Command::All | Command::Ingest | Command::Api => serve(&config).await,
        Command::Healthcheck => healthcheck(&config).await,
        Command::Alerts | Command::Migrate | Command::Demo => {
            tracing::warn!("not implemented yet");
            Ok(())
        }
    }
}

async fn serve(config: &Config) -> anyhow::Result<()> {
    let app = Router::new().route(
        "/healthz",
        get(|| async { Json(json!({ "status": "ok" })) }),
    );
    let listener = TcpListener::bind(&config.server.listen)
        .await
        .with_context(|| format!("binding {}", config.server.listen))?;
    tracing::info!(addr = %config.server.listen, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
}

/// Exits non-zero unless `GET /healthz` on the local listener answers 200.
async fn healthcheck(config: &Config) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let port = config
        .server
        .listen
        .rsplit(':')
        .next()
        .context("server.listen has no port")?;
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        stream
            .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await?;
        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).await?;
        Ok::<_, std::io::Error>(String::from_utf8_lossy(&buf[..n]).into_owned())
    };
    let head = tokio::time::timeout(std::time::Duration::from_secs(3), probe)
        .await
        .context("healthcheck timed out")??;
    anyhow::ensure!(head.contains(" 200 "), "unhealthy: {head}");
    Ok(())
}
