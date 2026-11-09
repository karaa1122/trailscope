//! The live bus: request summaries for the live tail, and server errors for
//! the real-time alert engine.
//!
//! Without Redis (the `all` mode default) both topics are in-process
//! `tokio::sync::broadcast` channels. With Redis configured, publishers send
//! to Redis only and every process forwards Redis messages into its local
//! broadcast, so subscribers see the same thing in both modes.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::config::{LiveConfig, RedisConfig};
use crate::error::{Error, Result};
use crate::rows::{ErrorSignal, LiveRequest};

const ERROR_CHANNEL_CAPACITY: usize = 65_536;

#[derive(Clone)]
pub struct LiveBus {
    requests: broadcast::Sender<Arc<LiveRequest>>,
    errors: broadcast::Sender<Arc<ErrorSignal>>,
    redis: Option<mpsc::Sender<(String, String)>>,
    channel: String,
}

impl LiveBus {
    /// In-process bus.
    pub fn local(cfg: &LiveConfig) -> Self {
        let (requests, _) = broadcast::channel(cfg.channel_capacity.max(16));
        let (errors, _) = broadcast::channel(ERROR_CHANNEL_CAPACITY);
        Self {
            requests,
            errors,
            redis: None,
            channel: String::new(),
        }
    }

    /// Bus backed by Redis pub/sub: publishes go to Redis, and a background
    /// task forwards Redis messages into the local broadcast channels.
    pub async fn redis(
        live: &LiveConfig,
        redis: &RedisConfig,
        shutdown: CancellationToken,
    ) -> Result<Self> {
        let client = redis::Client::open(redis.url.as_str())
            .map_err(|e| Error::Other(format!("redis: {e}")))?;
        let conn = client
            .get_connection_manager()
            .await
            .map_err(|e| Error::Other(format!("redis: {e}")))?;
        let mut bus = Self::local(live);
        bus.channel = redis.channel().to_string();

        let (tx, rx) = mpsc::channel(1024);
        bus.redis = Some(tx);
        tokio::spawn(publish_loop(conn, rx));
        tokio::spawn(subscribe_loop(client, bus.clone(), shutdown));
        Ok(bus)
    }

    fn topic(&self, name: &str) -> String {
        format!("{}:{name}", self.channel)
    }

    pub fn subscribe_requests(&self) -> broadcast::Receiver<Arc<LiveRequest>> {
        self.requests.subscribe()
    }

    pub fn subscribe_errors(&self) -> broadcast::Receiver<Arc<ErrorSignal>> {
        self.errors.subscribe()
    }

    pub fn publish_requests(&self, items: Vec<LiveRequest>) {
        if items.is_empty() {
            return;
        }
        match &self.redis {
            Some(tx) => send_redis(tx, self.topic("requests"), &items),
            None => fan_out(&self.requests, items),
        }
    }

    pub fn publish_errors(&self, items: Vec<ErrorSignal>) {
        if items.is_empty() {
            return;
        }
        match &self.redis {
            Some(tx) => send_redis(tx, self.topic("errors"), &items),
            None => fan_out(&self.errors, items),
        }
    }
}

fn fan_out<T>(tx: &broadcast::Sender<Arc<T>>, items: Vec<T>) {
    if tx.receiver_count() == 0 {
        return;
    }
    for item in items {
        // Err only means nobody is listening right now.
        let _ = tx.send(Arc::new(item));
    }
}

fn send_redis<T: Serialize>(tx: &mpsc::Sender<(String, String)>, topic: String, items: &[T]) {
    match serde_json::to_string(items) {
        Ok(payload) => {
            if tx.try_send((topic, payload)).is_err() {
                metrics::counter!("trailscope_live_dropped_batches_total").increment(1);
            }
        }
        Err(e) => tracing::error!(error = %e, "serializing live batch"),
    }
}

async fn publish_loop(
    mut conn: redis::aio::ConnectionManager,
    mut rx: mpsc::Receiver<(String, String)>,
) {
    while let Some((topic, payload)) = rx.recv().await {
        let res: redis::RedisResult<i64> = redis::cmd("PUBLISH")
            .arg(&topic)
            .arg(&payload)
            .query_async(&mut conn)
            .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, topic, "redis publish failed");
        }
    }
}

async fn subscribe_loop(client: redis::Client, bus: LiveBus, shutdown: CancellationToken) {
    let (req_topic, err_topic) = (bus.topic("requests"), bus.topic("errors"));
    let mut delay = Duration::from_millis(250);
    while !shutdown.is_cancelled() {
        let mut pubsub = match client.get_async_pubsub().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "redis subscribe: connect failed");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
                continue;
            }
        };
        if let Err(e) = pubsub.subscribe(&[&req_topic, &err_topic]).await {
            tracing::warn!(error = %e, "redis subscribe failed");
            tokio::time::sleep(delay).await;
            continue;
        }
        delay = Duration::from_millis(250);
        let mut stream = pubsub.on_message();
        loop {
            let msg = tokio::select! {
                () = shutdown.cancelled() => return,
                msg = stream.next() => msg,
            };
            let Some(msg) = msg else { break };
            let Ok(payload) = msg.get_payload::<String>() else {
                continue;
            };
            let channel = msg.get_channel_name();
            if channel == req_topic {
                forward(&bus.requests, &payload);
            } else if channel == err_topic {
                forward(&bus.errors, &payload);
            }
        }
        tracing::warn!("redis subscription closed; reconnecting");
    }
}

fn forward<T: DeserializeOwned>(tx: &broadcast::Sender<Arc<T>>, payload: &str) {
    match serde_json::from_str::<Vec<T>>(payload) {
        Ok(items) => fan_out(tx, items),
        Err(e) => tracing::warn!(error = %e, "bad live payload from redis"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_bus_delivers_to_subscribers() {
        let bus = LiveBus::local(&LiveConfig::default());
        let mut rx = bus.subscribe_requests();
        let mut erx = bus.subscribe_errors();
        bus.publish_requests(vec![LiveRequest {
            route: "/a".into(),
            ..Default::default()
        }]);
        bus.publish_errors(vec![ErrorSignal {
            status: 503,
            ..Default::default()
        }]);
        assert_eq!(rx.recv().await.unwrap().route, "/a");
        assert_eq!(erx.recv().await.unwrap().status, 503);
    }
}
