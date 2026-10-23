# @logbox/node

The core [Logbox](https://github.com/karaa1122/LogBox) client for Node.js. It batches requests, spans, job runs and log lines in memory and ships them in the background. It never blocks your request path.

Most apps use a framework integration built on this package: [`@logbox/express`](../express), [`@logbox/fastify`](../fastify) or [`@logbox/nestjs`](../nestjs). Use `@logbox/node` directly for workers, scripts, or frameworks without an integration.

## Quickstart

```ts
import { init } from '@logbox/node';

const logbox = init({
  endpoint: 'http://localhost:8080',
  ingestKey: process.env.LOGBOX_INGEST_KEY,
  service: 'worker',
});

await logbox.trackJob('report.generate', () => buildReport(), { attempt: 1 });
logbox.log('info', 'nightly run finished');
process.on('SIGTERM', () => void logbox.shutdown());
```

## What it does

- **Never blocks.** Capture calls append to a bounded in-memory queue (default 10,000 items; when full, the oldest are dropped and counted). A background timer flushes every second, or early at 500 items. Batches are gzipped NDJSON, retried with exponential backoff and jitter, and `429 Retry-After` is honoured. `shutdown()` flushes with a timeout.
- **Redacts by default.** Headers `authorization`, `cookie`, `set-cookie` and `x-api-key`; body keys like `password`, `token`, `access_token`, `card_number` and `ssn` at any depth (case-insensitive, `_`/`-` ignored); plus your own keys, dotted paths and regexes. The last IPv4 octet is masked. Redaction runs during the flush, not on the request path.
- **Carries context.** `runWithContext`, `setUser` and `setCorrelation` keep the request ID, trace ID, user and journey keys in AsyncLocalStorage, so log lines and jobs started inside a request are linked to it. `serializeContext()` and `trackJob(..., { context })` carry it into background jobs.
- **Samples sensibly.** `client.sample(route, status, durationMs)` keeps every error and slow request and samples healthy ones at `sampleRate`, with per-route overrides. Kept events record their `sampleRate` so the UI can scale counts.

## Configuration

Every option has an environment variable fallback.

| Option | Env | Default |
|---|---|---|
| `endpoint` | `LOGBOX_ENDPOINT` | `http://localhost:8080` |
| `ingestKey` | `LOGBOX_INGEST_KEY` | none (nothing is sent) |
| `service` | `LOGBOX_SERVICE` | `unknown-service` |
| `environment` | `LOGBOX_ENVIRONMENT`, `NODE_ENV` | `production` |
| `release` | `LOGBOX_RELEASE` | |
| `enabled` | `LOGBOX_ENABLED=0` disables | `true` |
| `maxQueueSize` | | `10000` |
| `flushIntervalMs` / `flushAt` / `maxBatchSize` | | `1000` / `500` / `1000` |
| `sampleRate` / `routeSampleRates` | | `1` / `{}` |
| `slowMs` | | `1000` |
| `captureBodies` | | `'errors'` (`'always'`, `'never'`) |
| `maxBodyBytes` | | `8192` |
| `ignoreRoutes` | | `/health`, `/healthz`, `/ready`, `/readyz`, `/livez` |
| `redact` | | `{ headers, keys, patterns, maskIp: true }` |
| `beforeSend` | | edit or drop items (return `null`) |
| `debug` | `LOGBOX_DEBUG=1` | `false` |

## API

```ts
client.captureRequest({ method, route, path, status, durationMs, ... })
client.captureEvent({ kind: 'custom', name: 'export.created', attributes: { rows: 1200 } })
client.captureSpan({ kind: 'db', name: 'SELECT users', durationMs: 4.2 })
client.log('warn', 'cache miss storm', { attributes: { keyspace: 'sessions' } })
client.captureException(err)
await client.span('external', 'POST billing-provider/charges', () => fetch(...))
await client.trackJob('email.send', () => send(), { attempt: job.attemptsMade + 1, context })
await client.flush()
await client.shutdown()
client.stats() // { queued, dropped, sent, rejected, failedBatches }
```

## Overhead

`pnpm bench` measures the request-path cost of capturing a request. On Node 24 it is about 0.005 ms at p99, against a 0.2 ms budget. See [docs/benchmarks.md](../../../../docs/benchmarks.md).

## License

Apache-2.0
