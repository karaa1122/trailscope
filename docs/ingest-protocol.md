# Ingest protocol

SDKs (or anything that speaks HTTP) send telemetry to Logbox in batches. This page is the contract; [sdk-spec.md](sdk-spec.md) describes how the official SDKs use it.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/v1/ingest` | Send a batch of requests, spans and events |
| `POST` | `/v1/releases` | Mark a deploy (shown as a marker on every time chart) |
| `GET` | `/v1/config` | Sampling and capture defaults for the key's project |

All three authenticate with an **ingest key**:

```
Authorization: Bearer lbx_ingest_…
```

Ingest keys are per project and write-only. They can't read anything back. Logbox stores only their SHA-256 hash; the plaintext is shown once, when the key is created.

## `POST /v1/ingest`

### Body

Either a JSON array of items, or NDJSON (one item per line). Logbox detects which one from the first non-whitespace character.

```
Content-Type: application/x-ndjson        (or application/json)
Content-Encoding: gzip                    (or zstd, or omitted)
```

### Limits

| Limit | Default | Config key |
|---|---|---|
| Compressed body | 5 MB | `ingest.max_body_bytes` |
| Decompressed body | 32 MB | `ingest.max_decompressed_bytes` |
| Items per batch | 10,000 | `ingest.max_items` |
| Stored body per request/response | 16 KB (truncated, `bodyTruncated` set) | `ingest.max_stored_body_bytes` |

### Items

Every item has a `type`: `request`, `span` or `event`. Field names are camelCase. Unknown fields are ignored.

Timestamps accept an RFC 3339 string (`2026-01-02T03:04:05.678Z`) or a number of **milliseconds** since the Unix epoch (fractions allowed, for microsecond spans). A missing timestamp means "now". Timestamps more than an hour in the future, or older than the retention period, are rejected.

Map fields (`attributes`, `correlation`, headers) take string, number or boolean values; numbers and booleans are stored as strings. Bodies may be strings or JSON values (which are serialized).

#### `request`

| Field | Type | Required | Notes |
|---|---|---|---|
| `service` | string | yes | e.g. `api` |
| `method` | string | yes | HTTP method, upper-cased by the server |
| `path` | string | yes | Raw path. A full URL is accepted; scheme and host are stripped. A `?query` suffix is split into `query` |
| `status` | number | yes | 100–599 |
| `durationMs` | number | yes | ≥ 0 |
| `route` | string | | Route pattern, e.g. `/users/:id`. If missing, the server derives one by replacing ID-like path segments with `:id` |
| `timestamp` | time | | When the request started |
| `environment` | string | | Default `production` |
| `release` | string | | Version of the service that handled the request |
| `requestId`, `traceId`, `spanId` | string | | The server generates a request ID if missing |
| `query` | string | | Without the leading `?` |
| `reqSize`, `resSize` | number | | Bytes |
| `userId` | string | | |
| `client`, `clientVersion` | string | | e.g. `ios`, `3.8.1` |
| `ip` | string | | Masked server-side when the project's `maskIp` setting is on (the default) |
| `userAgent` | string | | |
| `reqHeaders`, `resHeaders` | map | | Names are lower-cased |
| `reqBody`, `resBody` | string or JSON | | |
| `bodyTruncated` | boolean | | Set by SDKs that truncated a body |
| `errorType`, `errorMessage`, `errorStack` | string | | |
| `attributes` | map | | Free-form, up to 64 keys |
| `correlation` | map | | Journey keys: `account_id`, `session_id`, anything |
| `sampleRate` | number | | (0, 1]. The UI scales counts by `1 / sampleRate` |

#### `span`

| Field | Type | Required | Notes |
|---|---|---|---|
| `traceId`, `spanId` | string | yes | |
| `service`, `name` | string | yes | |
| `durationMs` | number | yes | |
| `parentSpanId`, `requestId` | string | | |
| `kind` | string | | `http`, `db`, `cache`, `queue`, `fn` (default), `external` |
| `start` | time | | |
| `status` | string | | `ok` (default) or `error` |
| `attributes` | map | | |

#### `event`

Background jobs, custom business events and application log lines.

| Field | Type | Required | Notes |
|---|---|---|---|
| `service`, `name` | string | yes | e.g. `worker`, `email.send` |
| `kind` | string | | `job`, `log` or `custom` (default) |
| `level` | string | | `debug`, `info` (default), `warn`, `error` (`warning`, `fatal` and `critical` are accepted) |
| `status` | string | | `ok`, `warn`, `error`. Defaults from `level` |
| `message` | string | | |
| `durationMs` | number | | |
| `attempt` | number | | 1-based job attempt |
| `requestId`, `traceId`, `userId` | string | | Link the event to a request and a journey |
| `errorType`, `errorMessage`, `errorStack` | string | | |
| `timestamp`, `environment`, `release`, `attributes`, `correlation` | | | As for requests |

### Response

Every item is validated on its own. Bad items are rejected individually and reported by index; the rest are accepted.

| Status | Meaning |
|---|---|
| `200` | All items accepted |
| `207` | Some items rejected (see `errors`) |
| `400` | No items accepted, or the body isn't valid JSON/NDJSON |
| `401` | Missing, unknown or revoked ingest key |
| `413` | Body or batch too large |
| `415` | Unsupported `Content-Encoding` |
| `429` | The ingest queue is full. Retry after `Retry-After` seconds |
| `503` | Shutting down, or the key store is unreachable. Retry |

```json
{
  "accepted": 98,
  "rejected": 2,
  "errors": [
    { "index": 3, "message": "status: must be between 100 and 599" },
    { "index": 7, "message": "unknown variant `metric`, expected one of `request`, `span`, `event`" }
  ]
}
```

Clients should retry `429` and `5xx` responses with backoff, and must not retry other `4xx` responses.

### What the server does with a batch

1. Authenticates the key (cached in memory for 30 seconds).
2. Decompresses with a size cap, then splits the body into items.
3. Validates and normalizes each item: required fields, value ranges, length caps (long strings are truncated, not rejected), route templating.
4. Applies the project's **redaction rules** (defense in depth; SDKs redact too): sensitive headers, sensitive JSON keys at any depth, value regexes, query strings, and IP masking.
5. Truncates stored bodies to 16 KB.
6. Computes the **error fingerprint**: xxhash64 of service, error type and the top three normalized stack frames (line and column numbers and IDs stripped, framework frames skipped). Without a stack it uses the message with numbers, UUIDs and IDs replaced by placeholders. A 5xx without error details is grouped as `HTTP <status>`.
7. Admits the batch to a bounded in-memory queue, bounded both in rows (`ingest.queue_capacity`) and in approximate bytes (`ingest.max_queue_bytes`). If it doesn't fit, the response is `429`. Nothing blocks, and memory stays bounded.
8. A writer flushes to ClickHouse every second or every 20,000 rows, with up to four inserts in flight, retrying failed inserts with backoff. While ClickHouse is down, rows stay queued and ingest answers `429`, so nothing is silently dropped.
9. After each successful flush, a compact summary of every request goes to the live tail, and every 5xx goes to the real-time alert engine.

On `SIGTERM` the server stops accepting connections, finishes in-flight requests, flushes the queue and exits.

## `POST /v1/releases`

```json
{
  "service": "api",
  "version": "v1.43.0",
  "environment": "production",
  "commit": "9f1c2ab",
  "url": "https://github.com/acme/api/releases/v1.43.0",
  "timestamp": "2026-01-02T03:04:05Z"
}
```

`service` and `version` are required; the timestamp defaults to now. The response is `201 {"id": "…"}`.

A deploy script can call it like this:

```bash
curl -fsS -X POST "$LOGBOX_ENDPOINT/v1/releases" \
  -H "Authorization: Bearer $LOGBOX_INGEST_KEY" \
  -H 'Content-Type: application/json' \
  -d "{\"service\":\"api\",\"version\":\"$VERSION\",\"commit\":\"$GIT_SHA\"}"
```

## `GET /v1/config`

Returns the project's SDK defaults, so they can be tuned from the UI:

```json
{
  "maskIp": true,
  "bodyCapture": "errors",
  "slowMs": 1000,
  "sampleRate": 1.0,
  "routeSampleRates": { "/search": 0.25 },
  "ignoreRoutes": ["/health", "/healthz", "/ready", "/readyz", "/livez"],
  "maxBodyBytes": 8192
}
```
