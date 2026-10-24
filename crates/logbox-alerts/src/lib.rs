//! Logbox alerts: rule evaluation, the real-time "Email on 5xx" rule, the
//! notification outbox and the channels it delivers to.

pub mod channels;
pub mod crypto;
pub mod email5xx;
pub mod mute;
pub mod outbox;
pub mod render;
pub mod rules;
pub mod smtp;
pub mod throttle;

use logbox_core::Config;
use logbox_core::ch::ClickHouse;
use logbox_core::live::LiveBus;
use sqlx::PgPool;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use crypto::Keys;
pub use outbox::Outbox;

/// Starts the rule evaluator, the 5xx email engine and the outbox worker.
pub fn start(
    config: Config,
    ch: ClickHouse,
    pool: PgPool,
    bus: LiveBus,
    keys: Keys,
    shutdown: CancellationToken,
) -> JoinSet<()> {
    let outbox = Outbox::new(pool.clone());
    let mut set = JoinSet::new();
    set.spawn(outbox::run_worker(
        config.clone(),
        pool.clone(),
        keys.clone(),
        outbox.clone(),
        shutdown.clone(),
    ));
    set.spawn(rules::run(
        config.clone(),
        ch.clone(),
        pool.clone(),
        outbox.clone(),
        shutdown.clone(),
    ));
    let engine = email5xx::Engine::new(config, ch, pool, keys, outbox);
    set.spawn(engine.run(bus, shutdown));
    tracing::info!("alerts started");
    set
}
