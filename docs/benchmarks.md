# Benchmarks

Measured results against the performance targets. Each section says how it was measured so the numbers can be reproduced.

## Test machine

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

### `@logbox/node` core

`sdks/node/packages/node/bench/overhead.mjs` times building and enqueueing one request event (headers, correlation, body capture on 2% of requests) for 200,000 iterations on Node 24. Redaction, serialization and gzip happen later, in the background flush, so they're not on the request path.

| p50 | p99 | p99.9 |
|---|---|---|
| 0.0005 ms | **0.0049 ms** | 0.059 ms |

Framework middleware benchmarks (Express, Fastify, NestJS, Django, FastAPI) are added with the integrations.

## Query, live tail and web

Measured when those milestones land.
