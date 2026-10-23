# Logbox build spec (original brief)

This is the brief the project is being built from, kept verbatim so any agent can pick up the work. The follow-up brief for email notifications is in [spec-email-notifications.md](spec-email-notifications.md). Progress and conventions are in [/AGENTS.md](../../AGENTS.md).

---

# Build Logbox — open-source, self-hosted API request logging

You are building **Logbox** (working name), an open-source, self-hosted tool for logging, searching and debugging HTTP API traffic. It is a separate product that any team runs next to their own services, like Grafana. It must work for **any HTTP API in any domain**: SaaS, e-commerce, fintech, internal tools. Nothing in the code, demo data, docs or UI copy may assume a specific industry.

## How to work

- **Do not ask me questions.** Everything you need is below. When something is ambiguous, pick the most sensible option, write it down in `docs/DECISIONS.md` (one entry per decision: context, choice, alternatives), and keep going.
- Work through the milestones in order and finish all of them in this session. At the end of each milestone, everything must build, lint cleanly and pass its tests. Commit at the end of each milestone using conventional commits (`feat:`, `fix:`, `chore:`…).
- Prefer boring, well-maintained libraries. Before pinning a crate, npm package, PyPI package or Docker image, check its current stable version instead of relying on memory.
- If Docker isn't available in your environment, still write the compose files and Dockerfiles, and run everything else natively. Use testcontainers (or local ClickHouse/Postgres binaries) for integration tests where possible.
- Code quality bar: `cargo fmt`, `cargo clippy -- -D warnings`, strict TypeScript, no `any` without a comment, and no `unwrap()`/`expect()` in non-test Rust code except at startup.

## Product in one paragraph

Services send structured events through tiny SDKs that batch in memory and never block a request. A Rust server ingests the batches into ClickHouse. A web UI provides a live tail, an explorer with a **SQL `WHERE` filter mode** (like Jira's JQL, but SQL), a request inspector (request/response bodies, headers, timing waterfall, copy as cURL, replay), journeys (everything that happened for one correlation key — an order, a user, a session, a tenant, anything — across services and background jobs), grouped errors, a route heatmap with release markers, and alerts. It is fast, keyboard-first, and secure by default: redaction, read-only queries, roles.

## Architecture

```
your services ── SDK (batch, gzip, non-blocking) ──► logbox-server (ingest)
                                                       │  bounded queue → batched inserts
                                                       ▼
                                                   ClickHouse  (events, spans, logs, rollups)
                                                       ▲
web UI (React SPA, embedded in the binary) ◄── logbox-server (api) ── Postgres (users, projects, keys, views, alerts)
                                                       │
                                          live tail: in-process broadcast, or Redis pub/sub in split mode
```

- **One Rust binary, `logbox`**, with subcommands: `logbox all` (ingest + api + alerts + UI in one process, the default for small installs), `logbox ingest`, `logbox api`, `logbox alerts`, `logbox migrate`, `logbox demo`. In `all` mode live tail uses a `tokio::sync::broadcast` channel. In split mode it uses Redis pub/sub. Redis is optional.
- **Rust stack:** tokio, axum, tower-http (compression, decompression, CORS, request IDs, tracing), serde, the official `clickhouse` crate (RowBinary inserts with its batching inserter), sqlx for Postgres (with migrations), `redis` or `fred` for pub/sub, argon2 for passwords, `sqlparser` for query parsing (see the Query language section), lettre for email, reqwest for webhooks, rust-embed to serve the SPA, utoipa for OpenAPI, tracing and tracing-subscriber for logging, metrics plus a Prometheus exporter.
- **Web:** Vite + React + TypeScript, TanStack Router, TanStack Query, TanStack Virtual for long tables, CodeMirror 6 for the query editor, uPlot (or hand-rolled SVG/canvas) for charts, Radix primitives for accessible popovers, menus and dialogs. Do not use a big UI kit; build the components with CSS variables. The built assets are embedded into the binary.
- **Storage:** ClickHouse for all telemetry. Postgres for metadata only.

## Repository layout

```
logbox/
  crates/
    logbox-core/      # shared types, event schema, config (env + logbox.toml), errors
    logbox-query/     # parser, validator, ClickHouse compiler, in-memory evaluator, search syntax, chips <-> AST
    logbox-ingest/    # /v1/ingest, validation, server-side redaction, fingerprinting, batching, live publish
    logbox-api/       # auth, projects, keys, query/explore, inspector, journeys, errors, routes, live SSE, OpenAPI
    logbox-alerts/    # rule evaluation and notification channels
    logbox-demo/      # realistic generic traffic generator + load test mode
    logbox-server/    # the `logbox` binary: CLI, wiring, migrations, embedded UI
  migrations/
    clickhouse/       # versioned .sql, applied by `logbox migrate` and at startup
    postgres/         # sqlx migrations
  web/                # React SPA
  sdks/
    node/             # pnpm workspace: @logbox/node (core), @logbox/express, @logbox/fastify, @logbox/nestjs
    python/           # `logbox` package: django, fastapi/starlette, celery integrations
  examples/           # minimal Express, NestJS, Django and FastAPI apps wired to the SDKs
  deploy/
    docker-compose.yml
    docker-compose.split.yml   # separate ingest/api/alerts + redis
    clickhouse/                # low-memory config for small VPSes
  docs/
  .github/
```

## Data model (ClickHouse)

Create database `logbox`. Use `LowCardinality(String)` for low-cardinality columns and ZSTD codecs on large text. Every table is keyed by `project_id` first.

**`requests`** — one row per HTTP request:

- identity and placement: `project_id`, `environment`, `service`, `release`, `timestamp DateTime64(3)`
- IDs: `request_id`, `trace_id`, `span_id`
- request: `method`, `route` (pattern, e.g. `/users/:id`), `path`, `query`, `status UInt16`, `duration_ms Float32`, `req_size UInt32`, `res_size UInt32`
- client: `user_id`, `client` (e.g. ios/android/web/server), `client_version`, `ip` (masked, stored as String), `user_agent`
- payload: `req_headers Map(LowCardinality(String), String)`, `res_headers`, `req_body String`, `res_body String`, `body_truncated UInt8`
- errors: `error_type`, `error_message`, `error_stack`, `error_fingerprint UInt64`
- extensibility: `attributes Map(LowCardinality(String), String)` for free-form attributes, `correlation Map(LowCardinality(String), String)` for journey keys (e.g. `order_id`, `account_id`, `session_id` — whatever the user's app sends)
- `sample_rate Float32`

Settings:

- `ORDER BY (project_id, service, route, timestamp)`, `PARTITION BY toYYYYMMDD(timestamp)`
- bloom filter skip indexes on `request_id`, `trace_id`, `user_id`, `mapValues(correlation)`
- table TTL from config (default 30 days), plus a column TTL on `req_body`, `res_body`, `req_headers` and `res_headers` (default 7 days)

**`spans`** — `project_id`, `trace_id`, `span_id`, `parent_span_id`, `request_id`, `service`, `kind` (http, db, cache, queue, fn, external), `name`, `start` (DateTime64(6)), `duration_ms`, `status`, `attributes`.

**`events`** — non-HTTP things: background jobs, custom business events, app log lines. Columns: `project_id`, `timestamp`, `service`, `kind` (`job`, `log`, `custom`), `name`, `level`, `message`, `status` (ok/warn/error), `duration_ms`, `request_id`, `trace_id`, `attempt UInt16`, `error_type`, `error_message`, `error_fingerprint`, `attributes`, `correlation`.

**`route_minute`** — `AggregatingMergeTree` fed by a materialized view from `requests`, keyed by project, service, route, method and minute. Columns: `countState()`, `countIfState(status >= 500)`, `countIfState(status >= 400)`, `quantilesTDigestState(0.5, 0.95, 0.99)(duration_ms)`. It powers the overview tiles, sparklines and route heatmap without scanning raw rows.

**`error_groups`** — materialized view keyed by `(project_id, error_fingerprint)` with first/last seen (min/max states), count, sample `request_id`, `error_type`, a sample message, `service` and the release where it was first seen.

**Fingerprint** = xxhash64 of service + error type + the top 3 normalized stack frames (strip line/column numbers and IDs). If there's no stack, use the message with numbers and UUIDs replaced by placeholders.

**`releases`** (Postgres) — project, service, version, timestamp, optional commit and URL. Shown as markers on every time chart.

## Ingest protocol

- `POST /v1/ingest`, `Authorization: Bearer lbx_ingest_<random>`. Body is NDJSON or a JSON array; `Content-Encoding` may be gzip or zstd. Each item has `type: "request" | "span" | "event"` plus the fields above, in camelCase on the wire. Limits: 5 MB compressed, 10,000 items per batch. Validate every item; reject bad items individually and report the rejected indexes in the response (`207`-style JSON with an accepted count and per-index errors).
- `POST /v1/releases` with the same key marks a deploy.
- The server applies the project's server-side redaction rules too (defense in depth), computes fingerprints, truncates oversize bodies (default 16 KB), and pushes rows into a **bounded in-memory queue**. A writer task flushes to ClickHouse every 1 s or 20,000 rows, whichever comes first. When the queue is full, return `429` with `Retry-After`. Never block or crash.
- On SIGTERM, stop accepting, flush the queue, then exit (graceful shutdown).
- After a successful flush, publish a compact summary of each request to the live channel.
- Ingest keys are write-only and per project. Store them as SHA-256 hashes; show the plaintext only once. Read/API tokens (`lbx_api_…`) are a separate type with scopes.
- Later milestone: accept OTLP/HTTP protobuf traces and logs at `/v1/otlp/v1/traces` and `/v1/otlp/v1/logs`, mapped into `spans`/`events`.

## Query language (the key feature)

Users filter with a SQL `WHERE` expression, the way Jira users write JQL:

```sql
WHERE status >= 500 AND route LIKE '/checkout/%' AND duration_ms > 800
  AND attr['tenant'] IN ('acme', 'globex') AND timestamp > now() - INTERVAL 15 MINUTE
```

Implement this in `logbox-query`. **Security is the top priority: user text must never reach ClickHouse as SQL.**

1. **Normalize.** The leading `WHERE` keyword is optional. Strip trailing semicolons and whitespace. Reject anything that remains after the expression, including a second statement. Reject SQL comments. Maximum 4,000 characters.
2. **Parse** the text with `sqlparser` (`Parser::parse_expr` with the ClickHouse dialect) into its AST.
3. **Convert** that AST into Logbox's own small typed AST. Allow only:
   - comparisons `= != <> < <= > >=`
   - `AND` / `OR` / `NOT` and parentheses
   - `IN` / `NOT IN` with a literal list (max 500 items), `BETWEEN`
   - `LIKE` / `NOT LIKE` / `ILIKE`
   - `IS [NOT] NULL` (maps to empty-value checks)
   - map access `attr['key']`, `attributes['key']`, `correlation['key']`
   - string, number and boolean literals
   - `now()`, `INTERVAL n SECOND|MINUTE|HOUR|DAY`, and `+`/`-` between timestamps and intervals
   - a function whitelist: `lower`, `upper`, `length`, `startsWith`, `endsWith`, `position`, and `match(col, 'regex')` with a 200-character regex limit

   Reject anything else with a clear error carrying a line and column, so the editor can underline it: subqueries, `UNION`, unknown columns or functions, `system.*`, casts to unsafe types, and so on. Also enforce a maximum nesting depth of 32 and a maximum of 256 nodes.
4. **Type-check** against a **column registry** (a single Rust source of truth listing name, ClickHouse type, description, and whether the column is filterable/facetable/groupable for each table). The UI's autocomplete and docs come from the same registry, through an endpoint.
5. **Compile** to ClickHouse SQL. Column names come only from the registry, never from the input text. Every literal becomes a ClickHouse query parameter (`{p1:String}` and so on). The server always wraps the expression as `WHERE project_id = {pid} AND timestamp BETWEEN {from} AND {to} AND (<expr>)`. The time range comes from the UI picker unless the expression narrows it further.
6. **Execute** as a dedicated ClickHouse user `logbox_reader` with a readonly profile, `SELECT` on the `logbox` database only, `max_execution_time` 15 s, `max_rows_to_read`, `max_result_rows` and `max_memory_usage` limits. A `LIMIT` is always applied. Pagination uses keyset ordering on `(timestamp, request_id)`.
7. **Evaluate in memory.** Provide an interpreter that evaluates the same AST against a single in-memory event. Live tail uses it to filter the stream server-side, so the live view and the explorer behave identically.
8. **Alternate syntaxes compile to the same AST:**
   - **Search syntax**, e.g. `status:>=500 route:/checkout/* user:u_123 "free text"` (free text matches `path`, `error_message` and `message`).
   - **Filter chips** — a list of `{field, op, value}` objects. Chips and AST convert both ways when the expression is a top-level `AND` of simple comparisons; otherwise the UI says "this query can only be edited as SQL".
9. **Tests.** Write unit tests for every operator and error message, and property tests (proptest) checking parse → compile → parameters round-trips. Add a `cargo-fuzz` target. Include an explicit injection test suite that must fail safely: `1=1; DROP TABLE requests`, `status = 1 UNION SELECT * FROM system.users`, comments `--` and `/* */`, backslash and quote escapes, unicode homoglyph quotes, nested subqueries, `attr['x'']) OR 1=1 --`, and very deep nesting.

Endpoints:

- `POST /api/v1/query/validate` → `{ok, errors: [{message, line, col, len}], chips?}`
- `POST /api/v1/query/requests` (rows)
- `POST /api/v1/query/histogram` (counts per bucket, split by status class)
- `POST /api/v1/query/facets` (top values plus counts for requested columns, under the current filter)
- `POST /api/v1/query/group` (group by any groupable column with `count`, `error_rate`, `p50`/`p95`/`p99`)
- `GET /api/v1/schema` (the column registry)
- `GET /api/v1/values?column=route&prefix=/us` (top values from the last hour, for autocomplete)

## Other API surface

- **Auth:** email + password (argon2id), HttpOnly/Secure/SameSite=Lax session cookies, CSRF tokens on state-changing requests, login rate limiting. A first-run setup wizard creates the owner account. Roles are `owner`, `admin`, `member` and `viewer`, plus a per-member `can_view_bodies` permission (bodies and headers are hidden from people without it). OIDC login is optional, in a later milestone.
- **Admin:** projects, environments, services (auto-discovered from ingest), ingest keys, API tokens, members, redaction rules, sampling defaults shown to SDKs, retention settings (shown read-only because TTLs come from config), notification channels.
- **Inspector:** `GET /api/v1/requests/:request_id` returns the full row, its spans, related events, and the error group. `GET /api/v1/requests/:id/curl?target=<base_url>` returns a cURL command with redacted values left as placeholders. `POST /api/v1/requests/:id/replay` re-sends the request to a base URL the admin has configured as a **replay target** (never an arbitrary URL), with auth headers left out unless the user provides them, and stores the result.
- **Journeys:** `GET /api/v1/journeys?key=order_id&value=ord_123` (any correlation key, or `user_id`, `request_id` or `trace_id`) merges requests, events and spans into one ordered timeline, grouped by service lane. Repetitive noise (same route and status more than 5 times in a row) is collapsed into "N similar".
- **Errors:** list error groups with sparklines, `new` (first seen in the last 24 h) and `spiking` (rate over the last 15 min ≥ 3× the previous 24 h baseline) flags, and a detail view with samples.
- **Routes:** heatmap data from `route_minute` (metric: p95, error rate or volume), plus release markers.
- **Live:** `GET /api/v1/live?filter=<expr>` as SSE. Cap each client at 200 events/s and emit `skipped: N` markers beyond that. Pausing is client-side.
- **Saved views:** a name, the query (in whichever mode it was written), the time range, the columns, and whether it's shared with the project.
- **Ops:** `/healthz`, `/readyz`, Prometheus `/metrics` for Logbox's own health (queue depth, flush latency, rows/s, rejected items, query latency), and OpenAPI JSON with a docs page.

## Alerts

A rule is a query, a condition and a window: `count > N`, `error_rate > X%`, `p95(duration_ms) > X ms` on a filter, or `new error group` / `spiking error group`. Rules are evaluated every 60 s. Each rule has states (ok, firing, resolved) with dedupe and a cooldown, so each state change sends one notification. Channels: generic webhook (signed with HMAC-SHA256), Slack incoming webhook, Telegram bot, and email over SMTP. Messages link back to the UI with the query pre-filled.

## SDKs

All SDKs share the same behaviour and a written spec in `docs/sdk-spec.md`:

- **Never block the request path.** Events go into an in-memory bounded queue (default 10,000; drop the oldest and count the drops). A background flush runs every 1 s or every 500 events, gzipped, with exponential backoff and jitter on failure, and respects `429` `Retry-After`. Flush on shutdown with a timeout.
- **Context:** generate or accept `X-Request-Id`, return it in the response header, and propagate W3C `traceparent`. Keep `request_id`/`trace_id`/`user_id`/correlation values in async-local context (AsyncLocalStorage in Node, contextvars in Python), so app log lines and background jobs carry them.
- **Redaction (default on):** headers `authorization`, `cookie`, `set-cookie`, `x-api-key`; body keys matching `password`, `passwd`, `secret`, `token`, `access_token`, `refresh_token`, `otp`, `pin`, `cvv`, `card`, `card_number`, `iban`, `ssn` (case-insensitive, at any depth); plus user-supplied key paths and regexes. Redacted values become `"[REDACTED]"`. Users can mask the last octet of the client IP (default on).
- **Body capture policy:** `"errors"` (the default: capture bodies when status ≥ 400 or the request is slow), `"always"`, or `"never"`. Default limit is 8 KB per body, and the event is marked when truncated. Skip binary and multipart bodies. Default `slowMs` is 1000.
- **Sampling:** keep 100% of errors and slow requests; healthy requests get a configurable rate (default 1.0) with per-route overrides. Record `sample_rate` on each event so the UI can scale counts.
- **Ignore list:** health and readiness routes by default.
- **Measured overhead:** a benchmark in each SDK repo showing < 0.2 ms p99 added per request.

Packages:

- `@logbox/node` (core client)
- `@logbox/express` (middleware)
- `@logbox/fastify` (plugin)
- `@logbox/nestjs` (module + interceptor + exception filter that captures the error type, message and stack, plus a logger bridge so `Logger` calls become `events` with the request's IDs, and a BullMQ helper that injects and restores context in job data so job runs are logged as `events` with `kind=job` and `attempt`)
- Python `logbox` (Django middleware, Starlette/FastAPI middleware, Celery signal hooks for job events, background-thread transport, `logging` handler bridge)

Each package ships typed configuration, tests, and a README with a 10-line quickstart.

## UI design

The look is dark-first (with a light theme too), dense and keyboard-first, closer to Linear or Warp than a generic admin template.

**Typography:** Geist for UI text and Geist Mono for all data, self-hosted (OFL license) from the repo.

**Tokens** (CSS variables, with a light-theme equivalent of each):

| Token | Value |
|---|---|
| bg | `#0A0C0F` |
| sidebar | `#0D1014` |
| panel | `#111419` |
| raised | `#161A21` |
| line | `#1D222B` |
| line-strong | `#2E3540` |
| text | `#E7EAEE` |
| text-2 | `#B7BDC7` |
| muted | `#9099A8` |
| dim | `#7D8696` |
| accent (lime) | `#C8F65D` |
| ok/2xx (blue) | `#8AB4F8` |
| warn/4xx (amber) | `#F5B544` |
| error/5xx | `#FF6B5B` (text `#FF8A7D`) |

The accent is used sparingly: active states, the primary button, the "now" line and release markers. Status colours appear only where they carry meaning, and always alongside the number or label, never as colour alone. Check that body text meets 4.5:1 contrast.

**Layout:** a left sidebar (logo, environment switcher, nav, a services list with health dots, a keyboard hints card) that collapses to an icon rail on detail pages. Every screen must work at phone width: the sidebar stacks, toolbars wrap, and wide tables scroll inside their own box.

**Query bar** (shared by Live tail, Explore and Routes):

- A segmented toggle: `Search | Filters | SQL WHERE`.
- SQL mode: a CodeMirror editor with line numbers, syntax highlighting (keywords in accent, strings blue, numbers amber), autocomplete from `/schema` and `/values` (columns with types, `attr['…']` keys, functions, keywords, top values), and inline error squiggles from `/query/validate`, debounced at 250 ms. Next to it are a Run button (`⌘↵`) and Save as view. Under it, a status line such as `✓ valid · 5 conditions · ~1.2M rows scanned · 38 ms`, plus History and Syntax help links.
- Filters mode: removable chips (`field op value ×`) and `+ Add filter`.
- Search mode: a single-line input.
- Below every mode, a **Quick filters** row of facet dropdowns (Service, Status, Method, Route, User, Client, Duration, Release, `+ attribute`), each showing value counts and adding a condition when clicked.
- Switching modes converts the query when it's representable.
- The whole query is encoded in the URL, so every view is shareable.

**Screens:**

1. **Live tail.**
   - Overview tiles: requests/min, error rate, p95 and slow requests, each with a delta and sparkline.
   - A route heatmap (routes × the last 30 minutes, one cell per minute, coloured by p95 from fast to over-SLO, with release markers above the columns and a metric toggle for p95/errors/volume). Clicking a cell filters the view.
   - A "New & spiking errors" panel.
   - The stream table with columns time, method, status pill, route (with a SLOW badge), a duration bar plus ms, user, client and service.
   - Filter pills: All / Errors ≥400 / 5xx / Slow, with counts.
   - Live/Paused toggle (Space) showing "Paused · N new".
   - A sampling note in the footer.
   - Rows open the inspector.
2. **Explore.** The query bar, a time histogram stacked by status class with release markers, a facet sidebar with counts, a virtualized results table with a column chooser, and a "group by" mode showing a table and chart of count/error rate/percentiles by any groupable column.
3. **Request inspector.**
   - A list of nearby requests under the current filter (J/K to move).
   - A header with a method pill, the route in large mono text, a status pill, the duration and the timestamp.
   - Chips for service, user, client, release and request ID.
   - Actions: Copy as cURL (C), Replay on target, Open journey, Share link.
   - An error card showing the type, message, collapsible stack (framework frames hidden) and group stats (count, users affected, first seen relative to a release).
   - Tabs for Request body / Response / Headers / cURL, with a JSON viewer that shows redacted fields as chips.
   - A details panel: request ID, trace ID, user, client, IP, service/pod, release, sizes.
   - A "Where the time went" waterfall of spans, with a one-line insight such as "91% waiting on external call".
4. **Journeys.**
   - A search box accepting `key:value` for any correlation key, user, request or trace.
   - A status line and summary tiles: elapsed time, requests, jobs, errors, services touched.
   - A **swimlane timeline**: one lane per service or job queue, dots coloured by status, the inspected request ringed in accent, noise shown as small grey dots, a "now" line when the journey is still in progress.
   - An event log with time, status dot, lane badge, title, detail and duration. The row you arrived from is tagged "YOU CAME FROM HERE".
   - Context cards built from the correlation and attribute values, and related error groups and traces.
5. **Errors:** group list and group detail.
6. **Routes:** a full-page heatmap and per-route latency charts.
7. **Alerts:** list, editor (using the query bar) and history.
8. **Settings.**
9. **Setup wizard:** create the owner, create the first project, show the ingest key once, show copy-paste SDK snippets for each framework, and confirm "first event received" live.

**Interaction details:** a command palette (`⌘K`) for navigation and actions; `/` focuses the query; `G L` / `G E` / `G J` jump between screens; hover tooltips on heatmap cells and chart points; skeleton states; and empty states that explain what to do next.

If a `design/` folder exists in the repo, treat it as the visual reference and match it.

## Demo and test data

`logbox demo` generates realistic traffic for a **fictional generic SaaS API**:

- routes: `/auth/login`, `/auth/refresh`, `/users/:id`, `/projects`, `/projects/:id/tasks`, `/search`, `/files/upload`, `/billing/invoices`, `/webhooks/incoming`
- background jobs: `email.send`, `report.generate`, `webhook.deliver`
- errors: an upstream `TimeoutError` with retries, `422 ValidationError`, `429 RateLimited`, `401`
- correlation keys `project_id` and `account_id`, and several clients and versions
- a scripted incident: a release deploys, the error rate and p95 spike on one route, then it recovers

It also has a `--load` mode that reports events/s and ingest latency. The `examples/` apps use the same generic domain.

## Packaging and deploy

- A multi-stage Dockerfile (cargo-chef caching, a slim final image, non-root user) built **multi-arch, linux/amd64 and linux/arm64**.
- `deploy/docker-compose.yml`: `logbox` (all mode) plus `clickhouse` plus `postgres`, with named volumes and healthchecks, and a ClickHouse config tuned for a 2–4 GB VPS. `docker compose up -d` must give a working UI at `http://localhost:8080` with migrations applied automatically.
- `docker-compose.split.yml` adds Redis and runs ingest, api and alerts separately.
- All configuration comes from env vars or `logbox.toml`, documented in `docs/configuration.md`.

## Open source setup

- License: **Apache-2.0** for the server and the SDKs.
- `README.md`: a one-line pitch, a screenshot (generated by Playwright from the demo data and saved to `docs/images/`), feature list, 60-second quickstart, SDK snippets, an architecture diagram in Mermaid, and links to the docs.
- `CONTRIBUTING.md` (dev setup with one command, running tests, commit style), `CODE_OF_CONDUCT.md` (Contributor Covenant), `SECURITY.md` (private disclosure), issue templates (bug, feature) and a PR template.
- `docs/`: `architecture.md`, `query-language.md` (full grammar, examples, security model), `ingest-protocol.md`, `sdk-spec.md`, one doc per SDK, `self-hosting.md` (VPS sizing, backups, upgrades, TLS behind a reverse proxy), `configuration.md`, `security.md`, `DECISIONS.md`.
- **CI (GitHub Actions):**
  - Rust: fmt, clippy, tests with ClickHouse and Postgres service containers, cargo-deny
  - web: lint, typecheck, unit tests, build
  - Node and Python SDK tests
  - Playwright e2e against the compose stack with demo data
  - on tag: multi-arch image to GHCR, npm publish (changesets) and PyPI publish (trusted publishing)
- Dependabot for cargo, npm, pip and GitHub Actions.

## Performance targets (verify, and record the results in `docs/benchmarks.md`)

- Ingest: ≥ 50,000 events/s sustained on 4 vCPU, with p99 HTTP ingest latency < 50 ms and memory bounded under overload (`429`s, not OOM).
- Query: p95 < 500 ms for typical filtered queries over 100M rows on the demo dataset; heatmap and tiles < 150 ms from the rollups.
- Live tail: < 2 s from ingest to the screen.
- SDK overhead: < 0.2 ms p99 per request.
- Web: initial JS < 300 KB gzipped; the stream table stays smooth at 10k rows (virtualized).

## Milestones (do all of them, in order)

1. **M0 — Scaffold:** Cargo workspace, pnpm workspace, Python package skeleton, lint/format configs, CI skeleton, compose with ClickHouse and Postgres, `logbox` binary with subcommands and config loading, `/healthz`.
2. **M1 — Ingest:** ClickHouse and Postgres migrations, ingest endpoint with auth, validation, redaction, fingerprinting, bounded queue and batched writer, graceful shutdown, `/metrics`; `@logbox/node` core client; `logbox demo` including `--load`. Done when demo traffic lands in ClickHouse and the load test hits the target.
3. **M2 — Query language:** `logbox-query` complete (parser, registry, compiler, evaluator, search syntax, chips round-trip), with unit, property, fuzz and injection tests, and `docs/query-language.md`.
4. **M3 — API:** auth and roles, setup wizard API, projects/keys/members, query endpoints, schema/values, inspector, cURL, replay targets, live SSE, saved views, OpenAPI.
5. **M4 — Web part 1:** app shell, theming, command palette, setup wizard, query bar (all three modes and quick filters), Live tail, Explore, Request inspector; embedded in the binary.
6. **M5 — Web part 2 and backend:** Journeys, Errors, Routes heatmap, releases endpoint and markers everywhere, Settings.
7. **M6 — Alerts:** rules, evaluator, all four channels, alerts UI.
8. **M7 — SDKs:** Express, Fastify, NestJS (with logger bridge and BullMQ helper), Python (Django, FastAPI/Starlette, Celery, logging bridge), working `examples/`, benchmarks.
9. **M8 — Harden and ship:** OTLP receiver, OIDC login, Playwright e2e and screenshots, performance verification and `docs/benchmarks.md`, full docs and README, release workflow. Then do a final pass over the whole repo for any industry-specific wording.

When everything is done, print a short summary: what was built, how to run it (`docker compose up -d`, then the URL), the benchmark results, and anything deferred, with the reason.
