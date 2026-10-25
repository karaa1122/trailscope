# SDK behaviour

What every official Logbox SDK does, so that a request logged from Express looks the same as one logged from Django. [ingest-protocol.md](ingest-protocol.md) is the wire contract; this page is how the SDKs use it. A new SDK should follow it point by point.

| Package | Language | Integrations |
|---|---|---|
| [`@logbox/node`](sdk-node.md) | Node.js 18+ | core client, used directly by workers and scripts |
| [`@logbox/express`](sdk-express.md) | Node.js 18+ | Express 4 and 5 |
| [`@logbox/fastify`](sdk-fastify.md) | Node.js 20+ | Fastify 5 |
| [`@logbox/nestjs`](sdk-nestjs.md) | Node.js 20+ | NestJS 11 and 12 (Express or Fastify platform), Logger bridge, BullMQ |
| [`logbox-sdk`](sdk-python.md) (import `logbox`) | Python 3.10+ | Django, ASGI (FastAPI, Starlette), Celery, `logging` |

## 1. Never block the request path

- Capture calls are synchronous, cheap and never touch the network. They append to an **in-memory queue bounded at 10,000 items**. When it is full, the oldest item is dropped and counted (`stats().dropped`).
- A **background flush** (a timer in Node, a daemon thread in Python) runs every second, or early once 500 items are queued. A flush sends batches of up to 1,000 items as **gzipped NDJSON** to `POST /v1/ingest`.
- **Redaction, `beforeSend`, serialization and compression happen in the flush**, not when the item is captured.
- **Retries:** network errors, `429` and `5xx` put the batch back at the front of the queue (dropping the oldest items if there is no room) and pause flushing with exponential backoff and jitter (1 s doubling to 60 s, randomized between half and the full value). A `Retry-After` header sets the minimum pause. Other `4xx` responses drop the batch; `401`/`403` print one warning about the ingest key.
- **Shutdown** flushes what is left, waiting at most 5 seconds. Node: `client.shutdown()` (also on `beforeExit`, and by `LogboxModule` on application shutdown). Python: `client.shutdown()`, registered with `atexit` by `init()`.
- **Forks (Python):** after `os.fork()` the child starts with an empty queue, fresh locks and a new flush thread, so preforking servers (gunicorn, Celery prefork) don't share or duplicate items.
- An SDK must never raise into the application because of Logbox: transport, serialization and recording errors are caught and reported only through the SDK's debug log (`LOGBOX_DEBUG=1`).

## 2. Configuration

Each option can be passed in code; the ones below also read an environment variable.

| Option | Env | Default |
|---|---|---|
| endpoint | `LOGBOX_ENDPOINT` | `http://localhost:8080` |
| ingest key | `LOGBOX_INGEST_KEY` | none: nothing is sent |
| service | `LOGBOX_SERVICE` | `unknown-service` |
| environment | `LOGBOX_ENVIRONMENT` (Node also `NODE_ENV`) | `production` |
| release | `LOGBOX_RELEASE` | empty |
| enabled | `LOGBOX_ENABLED=0` disables | on |
| debug log | `LOGBOX_DEBUG=1` | off |

The others (queue size, flush interval and size, sampling, body capture, ignored routes, redaction, `beforeSend`) are code options, named in the language's style (`sampleRate` / `sample_rate`).

### Settings from the server

At startup and then every 5 minutes, the SDK fetches `GET /v1/config` (the project's settings, editable in the UI under Settings → Project) and applies `sampleRate`, `routeSampleRates`, `slowMs`, `bodyCapture`, `maxBodyBytes`, `ignoreRoutes` and `maskIp`. **An option the application set in code always wins** over the server's value. A failed fetch keeps the current values. Node skips the fetch when a custom `fetch` is injected (tests), unless `remoteConfig: true` is passed.

## 3. Context

Every request runs inside a context (AsyncLocalStorage in Node, contextvars in Python) holding:

| Field | Source |
|---|---|
| request ID | the incoming `X-Request-Id` (configurable) if it matches `[A-Za-z0-9._:-]{1,128}`, otherwise 32 random hex characters. Returned in the response's `X-Request-Id` header |
| trace ID, parent span ID | the incoming W3C `traceparent` if valid, otherwise a new trace ID |
| span ID | new, 16 hex characters |
| user ID | `setUser()` / `set_user()` (Django: the authenticated user, if the request already loaded it) |
| correlation keys | `setCorrelation(key, value)`: the journey keys (an account, an order, a session…) |
| attributes | `setAttribute(key, value)` |
| route | the framework's matched route pattern |
| error | the exception that failed the request |

Log lines, spans, events and jobs started inside the request read the context, so they carry its request ID, trace ID, user and correlation keys.

## 4. HTTP requests

One `request` item per HTTP exchange, recorded when the response finishes:

- **Route** is the framework's pattern converted to `:name` parameters (`/projects/:project_id/tasks`): Express `req.baseUrl + req.route.path`, Fastify `routeOptions.url`, Nest's matched route, Django `resolver_match.route` (`<int:id>` and regex groups become `:id`), Starlette/FastAPI `scope["route"].path_format` (`{id}` becomes `:id`) with any mount prefix. When nothing matched (a 404), the route is left empty and the server derives one from the path.
- **Ignored routes** (default `/health`, `/healthz`, `/ready`, `/readyz`, `/livez`, matched against the route and the path) aren't recorded.
- **Sampling:** requests with status ≥ 400, an error, or a duration ≥ `slowMs` (default 1,000 ms) are always kept. Others are kept with probability `sampleRate` (default 1), overridable per route. A kept sampled request records its `sampleRate` so counts can be scaled.
- **Bodies** follow the capture policy: `errors` (the default: status ≥ 400 or slow), `always` or `never`. At most `maxBodyBytes` (default 8 KB) per body is kept, cut at a UTF-8 boundary, and `bodyTruncated` is set. Binary and multipart bodies (by content type, or a NUL byte in the first 512 bytes) and compressed responses (`Content-Encoding` other than `identity`) are skipped, as are Django's streaming responses. Response bodies are copied as they are written, at most `maxBodyBytes + 1` bytes, whatever the response size, so a streamed response keeps its beginning.
- **Errors:** the exception's type, message and full stack are attached **only when the response status is ≥ 500**. A handled 4xx (validation, not found, forbidden) is an expected outcome and isn't grouped as an error.
- A request whose client disconnects before the response is sent is recorded with status **499**.
- **IP:** the framework's notion of the client IP when it has one (Express `req.ip`, Fastify `request.ip`, which honour their trust-proxy settings), otherwise the first `X-Forwarded-For` hop, then the socket address.
- Also recorded: method, path, query, duration (monotonic clock), request and response sizes, user agent, request and response headers.

## 5. Redaction

On by default, applied in the flush before anything leaves the process. The server applies the project's own rules again.

- **Headers:** `authorization`, `cookie`, `set-cookie`, `x-api-key`, plus custom names.
- **Body keys** at any depth, case-insensitive, ignoring `_` and `-`: `password`, `passwd`, `secret`, `token`, `access_token`, `refresh_token`, `otp`, `pin`, `cvv`, `card`, `card_number`, `iban`, `ssn`, plus custom keys and dotted paths (`user.profile.dob`, `*` matching any key or array index). Applies to JSON bodies, form bodies and the query string; bodies that aren't valid JSON (for example truncated ones) get a text-level pass for the same keys.
- **Patterns:** custom regexes, replaced in string values, header values, the query, error messages and log messages.
- **IP masking** (default on): the last IPv4 octet becomes `0`; IPv6 keeps the first 48 bits.
- Redacted values become `"[REDACTED]"`.

## 6. Jobs

- A job started inside a request belongs to it by default. To carry the context to another process, serialize it into the job (`serializeContext()` / `serialize_context()`; the BullMQ helper uses `job.data.__logbox`, the Celery integration a `logbox` message header) and restore it in the worker.
- Each run is an `event` with `kind: "job"`, the job name, `status` `ok` or `error`, the duration, the **1-based attempt number**, the error's type, message and stack on failure, and the restored request ID, trace ID, user and correlation keys. Celery retries are recorded as `error` events with level `warn`.
- Log lines written during the job carry the job's context.

## 7. Logs, events and spans

- **Logs:** `log(level, message)` and the logger bridges (Nest `LogboxLogger`, Python `LogboxHandler`) send `event` items with `kind: "log"`, the logger name or context as the event name, and the current context. Bridges skip the SDK's own logs and, by default, framework startup noise and Nest's `ExceptionsHandler` lines (the request already carries that error).
- **Events:** `captureEvent` / `capture_event` for custom events (`kind: "custom"`).
- **Spans:** `span(kind, name, fn)` (Node) and `with client.span(kind, name)` (Python) time a block as a child of the current request's span.

## 8. Overhead

Each SDK ships a benchmark of what it adds to the request path, which must stay **under 0.2 ms at p99**. Results are in [benchmarks.md](benchmarks.md#sdk-overhead).
