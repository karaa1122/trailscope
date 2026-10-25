# Configuration

Logbox reads configuration in three layers, each overriding the one before:

1. Built-in defaults.
2. `logbox.toml`: `./logbox.toml` if it exists, or the file given with `--config` or `$LOGBOX_CONFIG` (which must exist).
3. Environment variables `LOGBOX_<SECTION>_<KEY>`, e.g. `LOGBOX_CLICKHOUSE_URL` sets `[clickhouse] url`.

Values that look like numbers are fine for string settings: `LOGBOX_CLICKHOUSE_PASSWORD=123456` stays the string `"123456"`. List values in env vars use TOML array syntax: `LOGBOX_SERVER_CORS_ORIGINS='["https://a.example"]'`.

A minimal `logbox.toml`:

```toml
[server]
public_url = "https://logs.example.com"

[clickhouse]
url = "http://clickhouse:8123"
user = "logbox"
password = "change-me"
reader_password = "change-me-too"

[postgres]
url = "postgres://logbox:change-me@postgres:5432/logbox"
```

## `[server]`

| Key | Env | Default | Description |
|---|---|---|---|
| `listen` | `LOGBOX_SERVER_LISTEN` | `0.0.0.0:8080` | Address the HTTP server binds to |
| `public_url` | `LOGBOX_SERVER_PUBLIC_URL` | `http://localhost:8080` | External URL, used in alert links and to decide cookie security |
| `secure_cookies` | `LOGBOX_SERVER_SECURE_COOKIES` | auto | Force the `Secure` cookie flag. Defaults to on when `public_url` is https |
| `cors_origins` | `LOGBOX_SERVER_CORS_ORIGINS` | `[]` | Extra origins allowed to call the API (the embedded UI needs none) |
| `shutdown_timeout_secs` | `LOGBOX_SERVER_SHUTDOWN_TIMEOUT_SECS` | `20` | How long shutdown keeps retrying the final flush |
| `session_ttl_hours` | `LOGBOX_SERVER_SESSION_TTL_HOURS` | `336` | Login session lifetime (14 days) |

## `[clickhouse]`

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `LOGBOX_CLICKHOUSE_URL` | `http://localhost:8123` | HTTP interface |
| `database` | `LOGBOX_CLICKHOUSE_DATABASE` | `logbox` | Created by migrations |
| `user` | `LOGBOX_CLICKHOUSE_USER` | `default` | Read-write user for migrations and inserts. Needs access management to create the reader user |
| `password` | `LOGBOX_CLICKHOUSE_PASSWORD` | empty | |
| `reader_user` | `LOGBOX_CLICKHOUSE_READER_USER` | `logbox_reader` | Restricted user that every user-driven query runs as. Created and updated by `logbox migrate` |
| `reader_password` | `LOGBOX_CLICKHOUSE_READER_PASSWORD` | `logbox_reader` | **Change this in production** |

## `[postgres]`

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `LOGBOX_POSTGRES_URL` | `postgres://logbox:logbox@localhost:5432/logbox` | Metadata store (users, projects, keys, views, alerts) |
| `max_connections` | `LOGBOX_POSTGRES_MAX_CONNECTIONS` | `10` | Pool size |

## `[redis]`

Only needed in split mode, where ingest, API and alerts run as separate processes.

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `LOGBOX_REDIS_URL` | empty | `redis://host:6379`. Empty means in-process channels (only valid for `logbox all`) |
| `channel` | `LOGBOX_REDIS_CHANNEL` | `logbox:live` | Pub/sub channel prefix (`<prefix>:requests`, `<prefix>:errors`) |

## `[ingest]`

| Key | Env | Default | Description |
|---|---|---|---|
| `max_body_bytes` | `LOGBOX_INGEST_MAX_BODY_BYTES` | `5242880` (5 MB) | Compressed request body limit |
| `max_decompressed_bytes` | `LOGBOX_INGEST_MAX_DECOMPRESSED_BYTES` | `33554432` (32 MB) | Decompression cap |
| `max_items` | `LOGBOX_INGEST_MAX_ITEMS` | `10000` | Items per batch |
| `queue_capacity` | `LOGBOX_INGEST_QUEUE_CAPACITY` | `200000` | Rows queued before ingest answers 429 |
| `max_queue_bytes` | `LOGBOX_INGEST_MAX_QUEUE_BYTES` | `268435456` (256 MB) | Approximate memory budget of the queue |
| `flush_interval_ms` | `LOGBOX_INGEST_FLUSH_INTERVAL_MS` | `1000` | Writer flush interval |
| `flush_rows` | `LOGBOX_INGEST_FLUSH_ROWS` | `20000` | Flush early at this many rows |
| `writer_concurrency` | `LOGBOX_INGEST_WRITER_CONCURRENCY` | `4` | ClickHouse inserts in flight |
| `max_concurrent_batches` | `LOGBOX_INGEST_MAX_CONCURRENT_BATCHES` | `0` (2 × CPUs) | Batches decoded at once; bounds memory held by in-flight requests |
| `max_stored_body_bytes` | `LOGBOX_INGEST_MAX_STORED_BODY_BYTES` | `16384` | Bodies are truncated to this |
| `retry_after_secs` | `LOGBOX_INGEST_RETRY_AFTER_SECS` | `2` | `Retry-After` on 429 |

## `[retention]`

TTLs are applied to the ClickHouse tables by `logbox migrate` (and at startup in `all` mode). Changing them updates the TTLs in place; existing parts expire as they merge, without a rewrite. The UI shows these values read-only.

| Key | Env | Default | Description |
|---|---|---|---|
| `days` | `LOGBOX_RETENTION_DAYS` | `30` | Raw rows and rollups |
| `body_days` | `LOGBOX_RETENTION_BODY_DAYS` | `7` | Request/response bodies and headers. Must be ≤ `days` |

## `[query]`

Limits for user-driven queries. They are also enforced server-side by the reader user's settings profile, so they can't be raised from a query.

| Key | Env | Default | Description |
|---|---|---|---|
| `max_execution_time_secs` | `LOGBOX_QUERY_MAX_EXECUTION_TIME_SECS` | `15` | |
| `max_rows_to_read` | `LOGBOX_QUERY_MAX_ROWS_TO_READ` | `2000000000` | |
| `max_result_rows` | `LOGBOX_QUERY_MAX_RESULT_ROWS` | `100000` | |
| `max_memory_usage` | `LOGBOX_QUERY_MAX_MEMORY_USAGE` | `2147483648` (2 GB) | |
| `default_limit` | `LOGBOX_QUERY_DEFAULT_LIMIT` | `100` | Rows per page in the explorer |
| `max_limit` | `LOGBOX_QUERY_MAX_LIMIT` | `1000` | |

## `[live]`

| Key | Env | Default | Description |
|---|---|---|---|
| `max_events_per_sec` | `LOGBOX_LIVE_MAX_EVENTS_PER_SEC` | `200` | Per-client live tail cap; beyond it the stream sends `skipped` markers |
| `channel_capacity` | `LOGBOX_LIVE_CHANNEL_CAPACITY` | `16384` | In-process broadcast buffer |

## `[alerts]`

| Key | Env | Default | Description |
|---|---|---|---|
| `interval_secs` | `LOGBOX_ALERTS_INTERVAL_SECS` | `60` | Rule evaluation interval (at least 10) |
| `default_cooldown_secs` | `LOGBOX_ALERTS_DEFAULT_COOLDOWN_SECS` | `600` | Cooldown for new rules when none is given |
| `email_throttle_minutes` | `LOGBOX_ALERTS_EMAIL_THROTTLE_MINUTES` | `15` | Email on 5xx: at most one email per error group per this many minutes |
| `email_hourly_cap` | `LOGBOX_ALERTS_EMAIL_HOURLY_CAP` | `30` | Email on 5xx: emails per project per hour before a single "paused" email |
| `regression_hours` | `LOGBOX_ALERTS_REGRESSION_HOURS` | `24` | Email on 5xx: a group quiet this long that comes back is emailed as a regression |
| `mute_link_days` | `LOGBOX_ALERTS_MUTE_LINK_DAYS` | `7` | How long the mute links in emails stay valid |

## Secret key

| Key | Env | Default | Description |
|---|---|---|---|
| `secret_key` (top level, not in a section) | `LOGBOX_SECRET_KEY` | empty | Encrypts stored secrets (SMTP password, channel URLs and tokens) and signs mute links. When empty, a key is generated and kept in Postgres. **Set a long random value in production** (`openssl rand -base64 48`); changing it makes stored secrets unreadable. See [alerts.md](alerts.md#secret-key) |

## `[smtp]`

Email notifications. Each field can also be entered in the UI (Settings → Notifications). A field set here or through its env var takes precedence over the UI value and shows as locked in the UI. Full details are in [alerts.md](alerts.md).

| Key | Env | Default | Description |
|---|---|---|---|
| `host` | `LOGBOX_SMTP_HOST` | empty | Empty disables email (unless configured in the UI) |
| `port` | `LOGBOX_SMTP_PORT` | `587` | |
| `security` | `LOGBOX_SMTP_SECURITY` | `starttls` | `starttls`, `tls` (implicit TLS, usually port 465) or `none` |
| `username` | `LOGBOX_SMTP_USERNAME` | empty | Empty sends without authentication |
| `password` | `LOGBOX_SMTP_PASSWORD` | empty | |
| `from` | `LOGBOX_SMTP_FROM` | empty | Sender address, e.g. `logbox@example.com`. Required to send |
| `from_name` | `LOGBOX_SMTP_FROM_NAME` | empty (`Logbox`) | Sender display name |

## `[oidc]`

Optional single sign-on.

| Key | Env | Default | Description |
|---|---|---|---|
| `issuer` | `LOGBOX_OIDC_ISSUER` | empty | Issuer URL. Empty disables OIDC |
| `client_id` | `LOGBOX_OIDC_CLIENT_ID` | empty | |
| `client_secret` | `LOGBOX_OIDC_CLIENT_SECRET` | empty | |
| `display_name` | `LOGBOX_OIDC_DISPLAY_NAME` | empty | Label on the login button |
| `auto_create_domains` | `LOGBOX_OIDC_AUTO_CREATE_DOMAINS` | `[]` | Email domains whose users get a `viewer` account on first login |

## `[log]`

| Key | Env | Default | Description |
|---|---|---|---|
| `level` | `LOGBOX_LOG_LEVEL` | `info,logbox=info` | `tracing` filter syntax. `RUST_LOG` overrides it |
| `format` | `LOGBOX_LOG_FORMAT` | `pretty` | `pretty` or `json` |

## Demo and load test

`logbox demo` takes its own flags (`logbox demo --help`). The most useful are:

| Flag | Default | Description |
|---|---|---|
| `--url` | `http://localhost:8080` | Logbox to send to (`$LOGBOX_DEMO_URL`) |
| `--key` | | Ingest key (`$LOGBOX_DEMO_KEY`). Without one, a "Demo" project and key are created in Postgres |
| `--backfill-hours` | `2` | History to generate first, including a past incident |
| `--rate` | `12` | Live requests per second |
| `--incident-after` | `120` | Seconds into the live run when a bad release deploys (0 disables) |
| `--load` | | Load-test mode |
| `--concurrency`, `--batch-size`, `--seconds` | `16`, `1000`, `30` | Load-test shape |
| `--target-eps` | `0` | Offered events/s in load mode (0 = as fast as possible) |
