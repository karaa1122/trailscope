# Node.js core: `@logbox/node`

The client the framework integrations are built on. Use it directly in workers, scripts and frameworks without an integration; for web apps use [`@logbox/express`](sdk-express.md), [`@logbox/fastify`](sdk-fastify.md) or [`@logbox/nestjs`](sdk-nestjs.md). Shared behaviour is described in [sdk-spec.md](sdk-spec.md).

```bash
npm install @logbox/node
```

```ts
import { init } from '@logbox/node';

const logbox = init({ service: 'report-worker' }); // LOGBOX_INGEST_KEY from the environment

await logbox.trackJob('report.build', () => buildReport(), { attempt: 1 });
logbox.log('info', 'nightly run finished');
process.on('SIGTERM', () => void logbox.shutdown());
```

`init()` creates the process-wide client that the integrations pick up; `new LogboxClient(options)` creates an independent one.

## Options

| Option | Env | Default | |
|---|---|---|---|
| `endpoint` | `LOGBOX_ENDPOINT` | `http://localhost:8080` | |
| `ingestKey` | `LOGBOX_INGEST_KEY` | | Without one nothing is sent |
| `service` | `LOGBOX_SERVICE` | `unknown-service` | |
| `environment` | `LOGBOX_ENVIRONMENT`, `NODE_ENV` | `production` | |
| `release` | `LOGBOX_RELEASE` | | |
| `enabled` | `LOGBOX_ENABLED=0` disables | `true` | When false every call is a no-op |
| `maxQueueSize` | | `10000` | Oldest dropped beyond this |
| `flushIntervalMs` / `flushAt` / `maxBatchSize` | | `1000` / `500` / `1000` | |
| `requestTimeoutMs` / `shutdownTimeoutMs` | | `10000` / `5000` | |
| `sampleRate` / `routeSampleRates` | | `1` / `{}` | Errors and slow requests are always kept |
| `slowMs` | | `1000` | |
| `captureBodies` | | `'errors'` | `'always'`, `'never'` |
| `maxBodyBytes` | | `8192` | Per body |
| `ignoreRoutes` | | health and readiness routes | Strings (exact) or regexes |
| `requestIdHeader` | | `x-request-id` | |
| `redact` | | `{ headers, keys, patterns, maskIp: true, disableDefaults: false }` | Added to the built-in rules |
| `remoteConfig` | | `true` (`false` with a custom `fetch`) | Load sampling and capture settings from `/v1/config` |
| `beforeSend` | | | Edit an item or return `null` to drop it; runs in the flush |
| `debug` | `LOGBOX_DEBUG=1` | `false` | `true` or a function receiving diagnostics |
| `fetch` | | `globalThis.fetch` | |

Sampling and capture options set here win over the project settings from the server.

## API

```ts
// Context of the current request (no-ops outside one)
setUser(userId); setCorrelation('account_id', id); setAttribute('plan', 'pro');
recordError(err);                    // attach the exception that failed the request
currentContext(); runWithContext(ctx, fn);

// Events, logs and spans
client.captureEvent({ kind: 'custom', name: 'export.created', attributes: { rows: 1200 } });
client.log('warn', 'cache miss storm', { attributes: { keyspace: 'sessions' }, error });
client.captureException(err);
await client.span('external', 'POST billing-provider/charges', () => fetch(url, init));

// Jobs: a job started inside a request belongs to it
await client.trackJob('email.send', () => send(), { attempt: 2 });
const carried = serializeContext();  // store with the job...
await client.trackJob('email.send', () => send(), { context: deserializeContext(carried) }); // ...restore in the worker

// Lifecycle
await client.flush(); await client.shutdown(); client.stats();
```

### Instrumenting another HTTP framework

`observeHttp(client, req, res, next, { route, body, ip })` instruments a Node.js `IncomingMessage`/`ServerResponse` pair: it builds the context, echoes the request ID, keeps the start of the response body, runs `next` inside the context and records the request when the response closes. Frameworks that don't expose the Node objects can call `client.contextFromHeaders()` at the start and `client.recordHttp(ctx, exchange)` at the end, as `@logbox/fastify` does.
