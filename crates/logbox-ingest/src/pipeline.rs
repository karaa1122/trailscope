//! The bounded queue and the ClickHouse writer.
//!
//! Admission is counted in rows and in approximate bytes: `try_push` reserves
//! room for a whole batch or refuses it (the handler then answers 429). Rows
//! stay counted until they are flushed, so while ClickHouse is down the queue
//! fills up and ingest pushes back instead of growing memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clickhouse::{Client, RowOwned, RowWrite};
use logbox_core::live::LiveBus;
use logbox_core::rows::{ErrorSignal, EventRow, LiveRequest, RequestRow, SpanRow};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
pub struct Batch {
    pub requests: Vec<RequestRow>,
    pub spans: Vec<SpanRow>,
    pub events: Vec<EventRow>,
    /// Approximate heap size, set when the batch is admitted.
    bytes: usize,
}

/// Per-entry overhead covers the two allocations and the tuple itself.
fn map_bytes(m: &[(String, String)]) -> usize {
    m.iter().map(|(k, v)| k.len() + v.len() + 96).sum()
}

impl Batch {
    /// Rough memory footprint: string payloads plus a per-row overhead.
    pub fn approx_bytes(&self) -> usize {
        let requests: usize = self
            .requests
            .iter()
            .map(|r| {
                1536 + r.path.len()
                    + r.query.len()
                    + r.user_agent.len()
                    + r.req_body.len()
                    + r.res_body.len()
                    + r.error_message.len()
                    + r.error_stack.len()
                    + map_bytes(&r.req_headers)
                    + map_bytes(&r.res_headers)
                    + map_bytes(&r.attributes)
                    + map_bytes(&r.correlation)
            })
            .sum();
        let spans: usize = self
            .spans
            .iter()
            .map(|s| 512 + s.name.len() + map_bytes(&s.attributes))
            .sum();
        let events: usize = self
            .events
            .iter()
            .map(|e| {
                768 + e.message.len()
                    + e.error_message.len()
                    + e.error_stack.len()
                    + map_bytes(&e.attributes)
                    + map_bytes(&e.correlation)
            })
            .sum();
        requests + spans + events
    }

    pub fn len(&self) -> usize {
        self.requests.len() + self.spans.len() + self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn append(&mut self, other: Batch) {
        self.bytes += other.bytes;
        self.requests.extend(other.requests);
        self.spans.extend(other.spans);
        self.events.extend(other.events);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PushError {
    Full,
    Closed,
}

pub struct Queue {
    depth: AtomicUsize,
    capacity: usize,
    bytes: AtomicUsize,
    max_bytes: usize,
    tx: mpsc::UnboundedSender<Batch>,
}

impl Queue {
    pub fn new(capacity: usize, max_bytes: usize) -> (Arc<Self>, mpsc::UnboundedReceiver<Batch>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                depth: AtomicUsize::new(0),
                capacity,
                bytes: AtomicUsize::new(0),
                max_bytes,
                tx,
            }),
            rx,
        )
    }

    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::Relaxed)
    }

    pub fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Relaxed)
    }

    pub fn is_full(&self) -> bool {
        self.depth() >= self.capacity || self.bytes() >= self.max_bytes
    }

    pub fn try_push(&self, mut batch: Batch) -> Result<(), PushError> {
        let n = batch.len();
        if n == 0 {
            return Ok(());
        }
        batch.bytes = batch.approx_bytes();
        let b = batch.bytes;
        let rows = self
            .depth
            .try_update(Ordering::AcqRel, Ordering::Acquire, |d| {
                (d + n <= self.capacity).then_some(d + n)
            });
        if rows.is_err() {
            return Err(PushError::Full);
        }
        let bytes = self
            .bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |d| {
                (d + b <= self.max_bytes).then_some(d + b)
            });
        if bytes.is_err() {
            self.release(n, 0);
            return Err(PushError::Full);
        }
        if self.tx.send(batch).is_err() {
            self.release(n, b);
            return Err(PushError::Closed);
        }
        self.report();
        Ok(())
    }

    fn release(&self, n: usize, bytes: usize) {
        self.depth.fetch_sub(n, Ordering::AcqRel);
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.report();
    }

    fn report(&self) {
        metrics::gauge!("logbox_ingest_queue_depth").set(self.depth() as f64);
        metrics::gauge!("logbox_ingest_queue_bytes").set(self.bytes() as f64);
    }
}

pub struct WriterConfig {
    pub flush_interval: Duration,
    pub flush_rows: usize,
    /// Inserts allowed in flight at once.
    pub concurrency: usize,
    /// How long to keep retrying a failing flush after shutdown begins.
    pub shutdown_grace: Duration,
}

/// Where flushed rows go. ClickHouse in production; a recorder in tests.
pub trait Sink: Send + Sync + 'static {
    fn write(&self, batch: &Batch) -> impl Future<Output = Result<(), String>> + Send;
}

pub struct ClickHouseSink {
    pub client: Client,
}

impl Sink for ClickHouseSink {
    async fn write(&self, batch: &Batch) -> Result<(), String> {
        let (a, b, c) = tokio::join!(
            insert(&self.client, "requests", &batch.requests),
            insert(&self.client, "spans", &batch.spans),
            insert(&self.client, "events", &batch.events),
        );
        a.and(b).and(c).map_err(|e| e.to_string())
    }
}

async fn insert<T>(client: &Client, table: &str, rows: &[T]) -> clickhouse::error::Result<()>
where
    T: RowOwned + RowWrite,
{
    if rows.is_empty() {
        return Ok(());
    }
    let mut ins = client.insert::<T>(table).await?;
    for row in rows {
        ins.write(row).await?;
    }
    ins.end().await
}

struct Writer<S> {
    sink: Arc<S>,
    queue: Arc<Queue>,
    bus: LiveBus,
    permits: Arc<Semaphore>,
    inflight: JoinSet<()>,
}

impl<S: Sink> Writer<S> {
    /// Hands `buf` to a flush task, waiting for a free slot first. While all
    /// slots are busy nothing is taken off the channel, the queue fills up and
    /// ingest answers 429: that's the backpressure.
    async fn dispatch(&mut self, buf: &mut Batch, deadline: Option<Instant>) {
        if buf.is_empty() {
            return;
        }
        let Ok(permit) = self.permits.clone().acquire_owned().await else {
            return;
        };
        let batch = std::mem::take(buf);
        let (sink, queue, bus) = (self.sink.clone(), self.queue.clone(), self.bus.clone());
        self.inflight.spawn(async move {
            flush(&*sink, &queue, &bus, batch, deadline).await;
            drop(permit);
        });
        // Reap finished tasks so the set doesn't grow.
        while self.inflight.try_join_next().is_some() {}
    }
}

/// Runs until `shutdown` fires, then drains everything queued and returns.
pub async fn run_writer<S: Sink>(
    sink: S,
    queue: Arc<Queue>,
    mut rx: mpsc::UnboundedReceiver<Batch>,
    bus: LiveBus,
    cfg: WriterConfig,
    shutdown: CancellationToken,
) {
    let mut w = Writer {
        sink: Arc::new(sink),
        queue,
        bus,
        permits: Arc::new(Semaphore::new(cfg.concurrency.max(1))),
        inflight: JoinSet::new(),
    };
    let mut buf = Batch::default();
    let mut tick = tokio::time::interval(cfg.flush_interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            msg = rx.recv() => match msg {
                Some(batch) => {
                    buf.append(batch);
                    if buf.len() >= cfg.flush_rows {
                        w.dispatch(&mut buf, None).await;
                    }
                }
                None => break,
            },
            _ = tick.tick() => w.dispatch(&mut buf, None).await,
        }
    }

    // Shutdown: the server has stopped accepting, so drain what's left.
    let deadline = Some(Instant::now() + cfg.shutdown_grace);
    rx.close();
    while let Some(batch) = rx.recv().await {
        buf.append(batch);
        if buf.len() >= cfg.flush_rows {
            w.dispatch(&mut buf, deadline).await;
        }
    }
    w.dispatch(&mut buf, deadline).await;
    while w.inflight.join_next().await.is_some() {}
    tracing::info!("ingest writer drained");
}

/// Writes one batch, retrying with backoff until it succeeds or `deadline` passes.
async fn flush<S: Sink>(
    sink: &S,
    queue: &Queue,
    bus: &LiveBus,
    batch: Batch,
    deadline: Option<Instant>,
) {
    let (n, bytes) = (batch.len(), batch.bytes);
    let mut delay = Duration::from_millis(200);
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let started = Instant::now();
        match sink.write(&batch).await {
            Ok(()) => {
                let secs = started.elapsed().as_secs_f64();
                metrics::histogram!("logbox_ingest_flush_seconds").record(secs);
                metrics::counter!("logbox_ingest_rows_total", "table" => "requests")
                    .increment(batch.requests.len() as u64);
                metrics::counter!("logbox_ingest_rows_total", "table" => "spans")
                    .increment(batch.spans.len() as u64);
                metrics::counter!("logbox_ingest_rows_total", "table" => "events")
                    .increment(batch.events.len() as u64);
                queue.release(n, bytes);
                publish(bus, &batch.requests);
                return;
            }
            Err(e) => {
                metrics::counter!("logbox_ingest_flush_errors_total").increment(1);
                tracing::warn!(attempt, rows = n, error = %e, "flush failed; retrying");
                if deadline.is_some_and(|d| Instant::now() + delay > d) {
                    tracing::error!(rows = n, "giving up on flush during shutdown; rows dropped");
                    metrics::counter!("logbox_ingest_dropped_rows_total").increment(n as u64);
                    queue.release(n, bytes);
                    return;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
            }
        }
    }
}

fn publish(bus: &LiveBus, requests: &[RequestRow]) {
    if requests.is_empty() {
        return;
    }
    bus.publish_requests(requests.iter().map(LiveRequest::from).collect());
    let errors: Vec<ErrorSignal> = requests
        .iter()
        .filter(|r| r.status >= 500)
        .map(ErrorSignal::from)
        .collect();
    bus.publish_errors(errors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use logbox_core::config::LiveConfig;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    #[derive(Clone, Default)]
    struct Recorder {
        rows: Arc<Mutex<Vec<usize>>>,
        fail: Arc<AtomicBool>,
    }

    impl Sink for Recorder {
        async fn write(&self, batch: &Batch) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("down".into());
            }
            self.rows.lock().unwrap().push(batch.len());
            Ok(())
        }
    }

    fn req(status: u16) -> RequestRow {
        RequestRow {
            status,
            route: "/x".into(),
            ..Default::default()
        }
    }

    fn batch(n: usize) -> Batch {
        Batch {
            requests: (0..n).map(|_| req(200)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn queue_refuses_batches_that_do_not_fit() {
        let (q, _rx) = Queue::new(10, usize::MAX);
        assert_eq!(q.try_push(batch(6)), Ok(()));
        assert_eq!(q.try_push(batch(5)), Err(PushError::Full));
        assert_eq!(q.try_push(batch(4)), Ok(()));
        assert!(q.is_full());
        q.release(10, q.bytes());
        assert_eq!(q.depth(), 0);
    }

    #[test]
    fn queue_refuses_batches_over_the_byte_budget() {
        let big = || Batch {
            requests: vec![RequestRow {
                req_body: "x".repeat(10_000),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (q, _rx) = Queue::new(1_000, 25_000);
        assert_eq!(q.try_push(big()), Ok(()));
        assert_eq!(q.try_push(big()), Ok(()));
        assert_eq!(q.try_push(big()), Err(PushError::Full));
        // A refused byte reservation must not leak row capacity.
        assert_eq!(q.depth(), 2);
        assert!(q.bytes() > 20_000);
    }

    fn cfg() -> WriterConfig {
        WriterConfig {
            flush_interval: Duration::from_millis(20),
            flush_rows: 1000,
            concurrency: 2,
            shutdown_grace: Duration::from_millis(300),
        }
    }

    #[tokio::test]
    async fn flushes_on_interval_publishes_and_releases() {
        let (q, rx) = Queue::new(100, usize::MAX);
        let bus = LiveBus::local(&LiveConfig::default());
        let mut live = bus.subscribe_requests();
        let mut errors = bus.subscribe_errors();
        let sink = Recorder::default();
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_writer(
            sink.clone(),
            q.clone(),
            rx,
            bus,
            cfg(),
            stop.clone(),
        ));

        q.try_push(Batch {
            requests: vec![req(200), req(502)],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(live.recv().await.unwrap().status, 200);
        assert_eq!(errors.recv().await.unwrap().status, 502);
        assert_eq!(q.depth(), 0);
        stop.cancel();
        task.await.unwrap();
        assert_eq!(*sink.rows.lock().unwrap(), vec![2]);
    }

    #[tokio::test]
    async fn keeps_rows_while_sink_is_down_then_recovers() {
        let (q, rx) = Queue::new(5, usize::MAX);
        let sink = Recorder::default();
        sink.fail.store(true, Ordering::SeqCst);
        let stop = CancellationToken::new();
        let bus = LiveBus::local(&LiveConfig::default());
        let task = tokio::spawn(run_writer(
            sink.clone(),
            q.clone(),
            rx,
            bus,
            cfg(),
            stop.clone(),
        ));

        q.try_push(batch(5)).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Still counted: the next batch is refused (the handler answers 429).
        assert_eq!(q.try_push(batch(1)), Err(PushError::Full));

        sink.fail.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(q.depth(), 0);
        assert_eq!(*sink.rows.lock().unwrap(), vec![5]);
        stop.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn drains_on_shutdown() {
        let (q, rx) = Queue::new(1000, usize::MAX);
        let sink = Recorder::default();
        let stop = CancellationToken::new();
        let bus = LiveBus::local(&LiveConfig::default());
        let slow = WriterConfig {
            flush_interval: Duration::from_secs(3600),
            ..cfg()
        };
        let task = tokio::spawn(run_writer(
            sink.clone(),
            q.clone(),
            rx,
            bus,
            slow,
            stop.clone(),
        ));
        q.try_push(batch(3)).unwrap();
        q.try_push(batch(4)).unwrap();
        stop.cancel();
        task.await.unwrap();
        assert_eq!(sink.rows.lock().unwrap().iter().sum::<usize>(), 7);
        assert_eq!(q.depth(), 0);
    }
}
