# Benchmarks

Measured results against the performance targets. Each section says how it was measured so the numbers can be reproduced.

| Target | Result |
|---|---|
| Ingest ≥ 50,000 events/s sustained on 4 vCPU, p99 < 50 ms, bounded memory under overload | 50,000 events/s at p99 17.3 ms; 108,000 events/s at saturation with 429s and 628 MB peak RSS |
| Query p95 < 500 ms for typical filtered queries over 100M rows | 56–409 ms p95 over 102M requests |
| Heatmap and tiles < 150 ms from the rollups | 78 ms and 13 ms p95 |
| Live tail < 2 s from ingest to screen | 631 ms p95 |
| SDK overhead < 0.2 ms p99 per request | at most 0.05 ms added (Django), 0.009 ms (Express) |
| Initial JS < 300 KB gzipped; smooth at 10k rows | 200 KB; 60 fps scrolling 10,000 live rows |

## Ingest test machine

Development workstation: a 12-core Windows 11 host running Docker Desktop, whose Linux VM has **8 vCPUs and 11 GB RAM** shared by every container below. Results on dedicated hardware should be equal or better.

| Container | Limits |
|---|---|
| `logbox all` (release build) | `--cpus 4 --memory 1g` |
| ClickHouse 26.8 | none (shares the remaining VM CPUs) |
| Postgres 18 | none |
| Load generator (`logbox demo --load`) | none (shares the remaining VM CPUs) |

## Ingest

Target: **≥ 50,000 events/s sustained on 4 vCPU, p99 HTTP ingest latency < 50 ms, memory bounded under overload (429s, not OOM).**

Payload: demo-generated `request` items (generic SaaS API traffic with headers, attributes and correlation keys), gzipped NDJSON, about 130 bytes compressed per item.

### Sustained 50k events/s (open loop)

```bash
logbox demo --url http://logbox-bench:8080 --load \
  --concurrency 32 --batch-size 500 --seconds 60 --target-eps 50000
```

| Metric | Result | Target |
|---|---|---|
| Offered and accepted | 50,000 events/s for 60 s (3,016,000 events) | ≥ 50,000 |
| HTTP latency p50 / p95 / p99 / max | 4.9 / 8.3 / **17.3** / 63 ms | p99 < 50 ms |
| 429s | 0 | |
| Rows written to ClickHouse | 3,016,000 (all of them) | |
| Server RSS | 298 MB | |

### Overload (closed loop, as fast as possible)

```bash
logbox demo --url http://logbox-bench:8080 --load --concurrency 64 --batch-size 1000 --seconds 30
```

| Metric | Result |
|---|---|
| Accepted throughput at saturation | 108,000 events/s |
| Batches refused with 429 + `Retry-After` | 1,904 |
| Errors / crashes / OOM kills | 0 |
| Peak RSS (1 GB container limit) | 628 MB |

Memory stays bounded under overload by three limits: the queue's row cap (`ingest.queue_capacity`), its byte budget (`ingest.max_queue_bytes`), and the number of batches decoded at once (`ingest.max_concurrent_batches`, 2 × CPUs).

### What made the difference

The first run reached only 27k events/s, with p99 at 320 ms under saturation and 900 MB RSS. Three changes fixed it:

1. Up to four ClickHouse inserts in flight instead of one sequential writer. A single insert stream topped out near 29k rows/s.
2. The clickhouse crate's per-insert schema validation is off for the hot path. Integration tests cover the row types instead.
3. mimalloc as the global allocator, so freed batch memory goes back to the OS instead of staying in fragmented glibc arenas.

## SDK overhead

Target: **< 0.2 ms p99 per request.**

Every benchmark measures the work the SDK adds in the request's own thread or event loop: building the context, echoing the request ID, keeping the first `maxBodyBytes` of the bodies and queueing the request. Redaction, serialization, gzip and the network happen later in the background flush, so they're not on the request path. 2% of the requests are 500s, so their bodies are captured. Each benchmark exits non-zero when the added p99 is over budget.

Integration numbers were measured on a Linux workstation (Intel Core i7-12650H, Ubuntu 24.04, Node 24.21, Python 3.12.3), not on the ingest test machine above.

### `@logbox/node` core

`sdks/node/packages/node/bench/overhead.mjs` times building and enqueueing one request event (headers, correlation, body capture on 2% of requests) for 200,000 iterations.

| Machine | p50 | p99 | p99.9 |
|---|---|---|---|
| Ingest test machine (Windows, Node 24) | 0.0005 ms | **0.0049 ms** | 0.059 ms |
| Linux workstation | 0.0001 ms | **0.0012 ms** | 0.0027 ms |

### Framework integrations

| Integration | How | Added p50 | Added p99 |
|---|---|---|---|
| `@logbox/express` | `bench/overhead.mjs`: the middleware around a route handler, with synthetic request and response objects, 200,000 iterations, against the bare handler | 0.0043 ms | **0.0089 ms** |
| `@logbox/fastify` | `bench/overhead.mjs`: two identical apps through `app.inject()` (in process, no sockets), 50,000 requests each | 0.0088 ms | **0.0021 ms** (within noise) |
| `@logbox/nestjs` | Not measured separately: the same Node HTTP instrumentation as Express, plus one interceptor call per request | — | — |
| Python, ASGI (FastAPI, Starlette) | `sdks/python/bench/overhead.py`: the middleware around an ASGI app, 50,000 requests, against the bare app | 0.0167 ms | **0.0331 ms** |
| Python, Django | Same script: the middleware around a view with `RequestFactory` requests | 0.0291 ms | **0.0495 ms** |

```bash
pnpm --filter "./sdks/node/**" build
node sdks/node/packages/express/bench/overhead.mjs
node sdks/node/packages/fastify/bench/overhead.mjs
cd sdks/python && python bench/overhead.py
```

## Query, live tail and web

Measured on the Linux workstation (Intel Core i7-12650H, 10 cores / 16 threads, 16 GB RAM, NVMe) with the release build of `logbox all` and ClickHouse 26.8 in Docker without resource limits, both on the same machine.

### Dataset

**102 million requests over 15 days** in one project, plus 13.6M spans and 1.6M events. `logbox demo --backfill-hours 24 --backfill-rate 120` produced 6.8M requests through the ingest API (generic SaaS traffic with a daily curve, an incident and a release); ClickHouse `INSERT … SELECT` then copied those requests 14 times, each one day further back with new request IDs, so the rollups were built by the materialized views as for real ingest.

| Table | Rows | On disk | Bytes per row |
|---|---|---|---|
| `requests` | 102.2M | 6.4 GB (42.8 GB uncompressed) | 68 |
| `spans` | 13.6M | 455 MB | 35 |
| `route_minute` (rollup) | 216k | 209 MB | 1,015 |

### Queries

Wall time through the HTTP API (sign-in session, JSON response), 20 runs after 2 warm-ups, as the read-only ClickHouse user with its limits. "All data" is the full 15-day window.

| Query | p50 | p95 | Target |
|---|---|---|---|
| Rows: `status >= 500` (all data, 100 rows) | 49 ms | 56 ms | p95 < 500 ms |
| Rows: `route = '/projects/:id/tasks' AND duration_ms > 800` | 28 ms | 75 ms | p95 < 500 ms |
| Rows: `attributes['plan'] = 'enterprise' AND status >= 400` | 183 ms | 189 ms | p95 < 500 ms |
| Rows: search `user_id:u_1137 status:5xx` | 76 ms | 84 ms | p95 < 500 ms |
| Rows: `correlation['account_id'] = 'acc_108'` | 271 ms | 274 ms | p95 < 500 ms |
| Histogram: `status >= 500` (all data) | 54 ms | 56 ms | p95 < 500 ms |
| Histogram: everything, last 24 h | 36 ms | 40 ms | p95 < 500 ms |
| Group by route with p50/p95/p99 (all 102M rows) | 399 ms | 409 ms | p95 < 500 ms |
| Facets: status, route, service (last 24 h) | 36 ms | 39 ms | p95 < 500 ms |
| **Overview tiles** (rollup, 60 min) | 11 ms | **13 ms** | < 150 ms |
| **Route heatmap** (rollup, 24 h) | 74 ms | **78 ms** | < 150 ms |
| Error groups (rollup, all data) | 8 ms | 10 ms | |

Journeys read every request, event and span carrying a key, so their cost depends on how common the key is: a request ID takes 28 ms, a busy user (10k requests a day) 506 ms, and a low-cardinality key such as the demo's `account_id` (a few hundred values, so every granule contains every account) 709 ms at p50 over 7 days. High-cardinality keys (orders, sessions, request IDs) are the common case.

What made the difference: the first run had the tiles at 173 ms and the heatmap at 146 ms because `route_minute` was sorted by route before minute, so a one-hour window read the whole 15 days of rollup rows (each with a latency digest). Migration 0002 sorts it by `(project_id, minute, …)` with 1,024-row granules, and the two tile queries now run concurrently.

### Live tail

Time from `POST /v1/ingest` returning to the request arriving on an open Live tail stream (Server-Sent Events, filtered by service), 40 requests spread across the flush interval:

| p50 | p95 | max | Target |
|---|---|---|---|
| 629 ms | 631 ms | 725 ms | < 2 s |

Most of it is the ingest writer's one-second flush interval (`ingest.flush_interval_ms`): requests appear once they are written to ClickHouse.

### Web

| Metric | Result | Target |
|---|---|---|
| Initial JavaScript (gzipped) | 200 KB (CodeMirror, 124 KB, loads only with SQL mode) | < 300 KB |
| Live tail with **10,000 rows** while 200 requests/s keep streaming in, scrolling 150 px per frame for 5 s | 60 fps, p95 frame 16.8 ms, no frame over 50 ms, 26 rows in the DOM (virtualized) | stays smooth |

The frame-time check is a Playwright script on Chromium, filling Live tail from `logbox demo --rate 400` (the stream is capped at 200 events/s per client, `live.max_events_per_sec`).
