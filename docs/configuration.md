# Configuration

Trailscope reads configuration in three layers, each overriding the one before:

1. Built-in defaults.
2. `trailscope.toml`: `./trailscope.toml` if it exists, or the file given with `--config` or `$TRAILSCOPE_CONFIG` (which must exist).
3. Environment variables `TRAILSCOPE_<SECTION>_<KEY>`, e.g. `TRAILSCOPE_CLICKHOUSE_URL` sets `[clickhouse] url`. An empty variable counts as unset, so the compose files can pass optional settings through (`TRAILSCOPE_SMTP_HOST: ${TRAILSCOPE_SMTP_HOST:-}`) without overriding anything when you leave them out of `deploy/.env`.

Values that look like numbers are fine for string settings: `TRAILSCOPE_CLICKHOUSE_PASSWORD=123456` stays the string `"123456"`. List values in env vars use TOML array syntax: `TRAILSCOPE_SERVER_CORS_ORIGINS='["https://a.example"]'`.

A minimal `trailscope.toml`:

```toml
[server]
public_url = "https://logs.example.com"

[clickhouse]
url = "http://clickhouse:8123"
user = "trailscope"
password = "change-me"
reader_password = "change-me-too"

[postgres]
url = "postgres://trailscope:change-me@postgres:5432/trailscope"
```

## `[server]`

| Key | Env | Default | Description |
|---|---|---|---|
| `listen` | `TRAILSCOPE_SERVER_LISTEN` | `0.0.0.0:8080` | Address the HTTP server binds to |
| `public_url` | `TRAILSCOPE_SERVER_PUBLIC_URL` | `http://localhost:8080` | External URL, used in alert links and to decide cookie security |
| `secure_cookies` | `TRAILSCOPE_SERVER_SECURE_COOKIES` | auto | Force the `Secure` cookie flag. Defaults to on when `public_url` is https |
| `cors_origins` | `TRAILSCOPE_SERVER_CORS_ORIGINS` | `[]` | Extra origins allowed to call the API (the embedded UI needs none) |
| `shutdown_timeout_secs` | `TRAILSCOPE_SERVER_SHUTDOWN_TIMEOUT_SECS` | `20` | How long shutdown keeps retrying the final flush |
| `session_ttl_hours` | `TRAILSCOPE_SERVER_SESSION_TTL_HOURS` | `336` | Login session lifetime (14 days) |
| `trust_proxy` | `TRAILSCOPE_SERVER_TRUST_PROXY` | `false` | Take the client IP from the first `X-Forwarded-For` hop (for login throttling and audit). Turn on only behind a reverse proxy you control |

## `[clickhouse]`

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `TRAILSCOPE_CLICKHOUSE_URL` | `http://localhost:8123` | HTTP interface |
| `database` | `TRAILSCOPE_CLICKHOUSE_DATABASE` | `trailscope` | Created by migrations |
| `user` | `TRAILSCOPE_CLICKHOUSE_USER` | `default` | Read-write user for migrations and inserts. Needs access management to create the reader user |
| `password` | `TRAILSCOPE_CLICKHOUSE_PASSWORD` | empty | |
| `reader_user` | `TRAILSCOPE_CLICKHOUSE_READER_USER` | `trailscope_reader` | Restricted user that every user-driven query runs as. Created and updated by `trailscope migrate` |
| `reader_password` | `TRAILSCOPE_CLICKHOUSE_READER_PASSWORD` | `trailscope_reader` | **Change this in production** |

## `[postgres]`

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `TRAILSCOPE_POSTGRES_URL` | `postgres://trailscope:trailscope@localhost:5432/trailscope` | Metadata store (users, projects, keys, views, alerts) |
| `max_connections` | `TRAILSCOPE_POSTGRES_MAX_CONNECTIONS` | `10` | Pool size |

## `[redis]`

Only needed in split mode, where ingest, API and alerts run as separate processes.

| Key | Env | Default | Description |
|---|---|---|---|
| `url` | `TRAILSCOPE_REDIS_URL` | empty | `redis://host:6379`. Empty means in-process channels (only valid for `trailscope all`) |
| `channel` | `TRAILSCOPE_REDIS_CHANNEL` | `trailscope:live` | Pub/sub channel prefix (`<prefix>:requests`, `<prefix>:errors`) |

## `[ingest]`

| Key | Env | Default | Description |
|---|---|---|---|
| `max_body_bytes` | `TRAILSCOPE_INGEST_MAX_BODY_BYTES` | `5242880` (5 MB) | Compressed request body limit |
| `max_decompressed_bytes` | `TRAILSCOPE_INGEST_MAX_DECOMPRESSED_BYTES` | `33554432` (32 MB) | Decompression cap |
| `max_items` | `TRAILSCOPE_INGEST_MAX_ITEMS` | `10000` | Items per batch |
| `queue_capacity` | `TRAILSCOPE_INGEST_QUEUE_CAPACITY` | `200000` | Rows queued before ingest answers 429 |
| `max_queue_bytes` | `TRAILSCOPE_INGEST_MAX_QUEUE_BYTES` | `268435456` (256 MB) | Approximate memory budget of the queue |
| `flush_interval_ms` | `TRAILSCOPE_INGEST_FLUSH_INTERVAL_MS` | `1000` | Writer flush interval |
| `flush_rows` | `TRAILSCOPE_INGEST_FLUSH_ROWS` | `20000` | Flush early at this many rows |
| `writer_concurrency` | `TRAILSCOPE_INGEST_WRITER_CONCURRENCY` | `4` | ClickHouse inserts in flight |
| `max_concurrent_batches` | `TRAILSCOPE_INGEST_MAX_CONCURRENT_BATCHES` | `0` (2 × CPUs) | Batches decoded at once; bounds memory held by in-flight requests |
| `max_stored_body_bytes` | `TRAILSCOPE_INGEST_MAX_STORED_BODY_BYTES` | `16384` | Bodies are truncated to this |
| `retry_after_secs` | `TRAILSCOPE_INGEST_RETRY_AFTER_SECS` | `2` | `Retry-After` on 429 |

## `[retention]`

TTLs are applied to the ClickHouse tables by `trailscope migrate` (and at startup in `all` mode). Changing them updates the TTLs in place; existing parts expire as they merge, without a rewrite. The UI shows these values read-only.

| Key | Env | Default | Description |
|---|---|---|---|
| `days` | `TRAILSCOPE_RETENTION_DAYS` | `30` | Raw rows and rollups |
| `body_days` | `TRAILSCOPE_RETENTION_BODY_DAYS` | `7` | Request/response bodies and headers. Must be ≤ `days` |

## `[query]`

Limits for user-driven queries. They are also enforced server-side by the reader user's settings profile, so they can't be raised from a query.

| Key | Env | Default | Description |
|---|---|---|---|
| `max_execution_time_secs` | `TRAILSCOPE_QUERY_MAX_EXECUTION_TIME_SECS` | `15` | |
| `max_rows_to_read` | `TRAILSCOPE_QUERY_MAX_ROWS_TO_READ` | `2000000000` | |
| `max_result_rows` | `TRAILSCOPE_QUERY_MAX_RESULT_ROWS` | `100000` | |
| `max_memory_usage` | `TRAILSCOPE_QUERY_MAX_MEMORY_USAGE` | `2147483648` (2 GB) | |
| `default_limit` | `TRAILSCOPE_QUERY_DEFAULT_LIMIT` | `100` | Rows per page in the explorer |
| `max_limit` | `TRAILSCOPE_QUERY_MAX_LIMIT` | `1000` | |

## `[live]`

| Key | Env | Default | Description |
|---|---|---|---|
| `max_events_per_sec` | `TRAILSCOPE_LIVE_MAX_EVENTS_PER_SEC` | `200` | Per-client live tail cap; beyond it the stream sends `skipped` markers |
| `channel_capacity` | `TRAILSCOPE_LIVE_CHANNEL_CAPACITY` | `16384` | In-process broadcast buffer |

## `[alerts]`

| Key | Env | Default | Description |
|---|---|---|---|
| `interval_secs` | `TRAILSCOPE_ALERTS_INTERVAL_SECS` | `60` | Rule evaluation interval (at least 10) |
| `default_cooldown_secs` | `TRAILSCOPE_ALERTS_DEFAULT_COOLDOWN_SECS` | `600` | Cooldown for new rules when none is given |
| `email_throttle_minutes` | `TRAILSCOPE_ALERTS_EMAIL_THROTTLE_MINUTES` | `15` | Email on 5xx: at most one email per error group per this many minutes |
| `email_hourly_cap` | `TRAILSCOPE_ALERTS_EMAIL_HOURLY_CAP` | `30` | Email on 5xx: emails per project per hour before a single "paused" email |
| `regression_hours` | `TRAILSCOPE_ALERTS_REGRESSION_HOURS` | `24` | Email on 5xx: a group quiet this long that comes back is emailed as a regression |
| `mute_link_days` | `TRAILSCOPE_ALERTS_MUTE_LINK_DAYS` | `7` | How long the mute links in emails stay valid |

## Secret key

| Key | Env | Default | Description |
|---|---|---|---|
| `secret_key` (top level, not in a section) | `TRAILSCOPE_SECRET_KEY` | empty | Encrypts stored secrets (SMTP password, channel URLs and tokens) and signs mute links. When empty, a key is generated and kept in Postgres. **Set a long random value in production** (`openssl rand -base64 48`); changing it makes stored secrets unreadable. See [alerts.md](alerts.md#secret-key) |

## `[smtp]`

Email notifications. Each field can also be entered in the UI (Settings → Notifications). A field set here or through its env var takes precedence over the UI value and shows as locked in the UI. Full details are in [alerts.md](alerts.md).

| Key | Env | Default | Description |
|---|---|---|---|
| `host` | `TRAILSCOPE_SMTP_HOST` | empty | Empty disables email (unless configured in the UI) |
| `port` | `TRAILSCOPE_SMTP_PORT` | `587` | |
| `security` | `TRAILSCOPE_SMTP_SECURITY` | `starttls` | `starttls`, `tls` (implicit TLS, usually port 465) or `none` |
| `username` | `TRAILSCOPE_SMTP_USERNAME` | empty | Empty sends without authentication |
| `password` | `TRAILSCOPE_SMTP_PASSWORD` | empty | |
| `from` | `TRAILSCOPE_SMTP_FROM` | empty | Sender address, e.g. `trailscope@example.com`. Required to send |
| `from_name` | `TRAILSCOPE_SMTP_FROM_NAME` | empty (`Trailscope`) | Sender display name |

## `[oidc]`

Optional single sign-on with any OpenID Connect provider (Google, Microsoft Entra ID, Okta, Auth0, Keycloak, Authentik, Dex…), using the authorization code flow with PKCE. Password sign-in keeps working alongside it.

| Key | Env | Default | Description |
|---|---|---|---|
| `issuer` | `TRAILSCOPE_OIDC_ISSUER` | empty | Issuer URL; `<issuer>/.well-known/openid-configuration` must exist. Empty disables OIDC |
| `client_id` | `TRAILSCOPE_OIDC_CLIENT_ID` | empty | |
| `client_secret` | `TRAILSCOPE_OIDC_CLIENT_SECRET` | empty | |
| `display_name` | `TRAILSCOPE_OIDC_DISPLAY_NAME` | empty | Label on the sign-in button ("Sign in with …") |
| `auto_create_domains` | `TRAILSCOPE_OIDC_AUTO_CREATE_DOMAINS` | `[]` | Email domains whose users get a `viewer` account on first sign-in, e.g. `'["example.com"]'` |

Register `<server.public_url>/api/v1/auth/oidc/callback` as the redirect URI at the provider, with the scopes `openid email profile`. On sign-in, Trailscope matches the provider's subject to a member, then the email address (which must not be marked unverified), and remembers the subject. Someone without an account is signed in only if their email domain is in `auto_create_domains`; everyone else needs an invite first. Disabled members can't sign in either way.

## `[log]`

| Key | Env | Default | Description |
|---|---|---|---|
| `level` | `TRAILSCOPE_LOG_LEVEL` | `info,trailscope=info` | `tracing` filter syntax. `RUST_LOG` overrides it |
| `format` | `TRAILSCOPE_LOG_FORMAT` | `pretty` | `pretty` or `json` |

## Demo and load test

`trailscope demo` takes its own flags (`trailscope demo --help`). The most useful are:

| Flag | Default | Description |
|---|---|---|
| `--url` | `http://localhost:8080` | Trailscope to send to (`$TRAILSCOPE_DEMO_URL`) |
| `--key` | | Ingest key (`$TRAILSCOPE_DEMO_KEY`). Without one, a "Demo" project and key are created in Postgres |
| `--backfill-hours` | `2` | History to generate first, including a past incident |
| `--rate` | `12` | Live requests per second |
| `--incident-after` | `120` | Seconds into the live run when a bad release deploys (0 disables) |
| `--load` | | Load-test mode |
| `--concurrency`, `--batch-size`, `--seconds` | `16`, `1000`, `30` | Load-test shape |
| `--target-eps` | `0` | Offered events/s in load mode (0 = as fast as possible) |
