# Decisions

One entry per decision that the spec left open. Newest at the bottom.

## 001. Development toolchain runs in containers

- **Context:** The development machine has Node and Docker but no Rust or Python toolchain.
- **Choice:** Rust (`rust:1-bookworm`) and Python (`python:3.13-slim`) run in long-lived dev containers with the repo bind-mounted. ClickHouse and Postgres for integration tests run as containers on a `logbox-dev` network. CI uses native toolchains and service containers.
- **Alternatives:** Installing rustup with the MSVC or GNU toolchain on Windows (needs Visual Studio Build Tools, and diverges from the Linux production target).

## 002. Pinned versions

- **Context:** The spec asks to check current stable versions rather than rely on memory.
- **Choice:** Rust 1.99.1 (`rust-toolchain.toml`), ClickHouse 26.8 (an LTS line), Postgres 18, Redis 8, Node 24, TypeScript 6.0.3. TypeScript 7 (the native compiler) is out, but typescript-eslint 8.x supports `<6.1`, so 6.0.3 is the newest version the lint stack accepts.
- **Alternatives:** TypeScript 7 without type-aware linting.

## 003. One pnpm workspace at the repo root

- **Context:** The layout shows `sdks/node/` as a pnpm workspace, and `web/` and `examples/` also need Node packages. pnpm doesn't support nested workspaces well.
- **Choice:** A single root `pnpm-workspace.yaml` covering `web`, `sdks/node/packages/*`, the Node examples and `e2e`. The SDK packages still live under `sdks/node/packages/` and are released with changesets; private packages are ignored by changesets.
- **Alternatives:** A separate lockfile per directory (duplicated dev tooling and slower CI).

## 004. Environment variable naming

- **Context:** Configuration comes from `logbox.toml` and env vars.
- **Choice:** `LOGBOX_<SECTION>_<KEY>` maps to `<section>.<key>`, e.g. `LOGBOX_CLICKHOUSE_READER_PASSWORD`. Section names have no underscores, so the first underscore after the section is unambiguous. String fields that hold secrets accept bare numbers (`LOGBOX_CLICKHOUSE_PASSWORD=123456`).
- **Alternatives:** figment's `__` separator (`LOGBOX_CLICKHOUSE__URL`), which is easy to get wrong.

## 005. Repository URL and image name

- **Context:** The project needs a canonical URL for package metadata and image labels.
- **Choice:** `https://github.com/karaa1122/LogBox`. CI publishes to `ghcr.io/${{ github.repository }}`, so forks work without edits. Compose builds `logbox:local` from source by default; `LOGBOX_IMAGE` selects a published image.
- **Alternatives:** A placeholder organisation.

## 006. Distroless runtime image and built-in healthcheck

- **Context:** The final image should be slim and non-root, and compose needs a healthcheck.
- **Choice:** `gcr.io/distroless/cc-debian13:nonroot`. It has no shell or curl, so the binary has a hidden `logbox healthcheck` subcommand that probes `/healthz` on the local listener.
- **Alternatives:** `debian:trixie-slim` with curl (larger image, more CVE surface).

## 007. Split-mode ports

- **Context:** In split mode ingest and api are separate processes.
- **Choice:** The api (with the UI) is published on 8080 and ingest on 8081. A one-shot `migrate` service runs first, and the other services wait for it to complete.
- **Alternatives:** A reverse proxy in front of both (one more moving part for small installs).

## 008. Route templating when SDKs send no route

- **Context:** `route` should be a pattern, but raw HTTP clients may send only a path, and high-cardinality paths would blow up the rollups.
- **Choice:** When `route` is missing, the server replaces ID-like path segments with `:id`: all-digit segments, UUIDs, long hex strings, prefixed IDs like `usr_8f2k1x`, and long opaque tokens. Anything that isn't all digits or a UUID must contain a digit, so `/oauth/access_tokens` stays literal.
- **Alternatives:** Reject items without a route (unfriendly), or store the path as the route (unbounded cardinality).

## 009. Server errors without error details still get a group

- **Context:** A 5xx response often has no exception attached (a proxy timeout, a handler that returns 503).
- **Choice:** If a request has status ≥ 500 and no error type, message or stack, the server sets `error_type = "HTTP <status>"` and a message like `GET /route returned 503`, and fingerprints it. 4xx responses are grouped only when the SDK reports an error type or message.
- **Alternatives:** Leave such requests out of error groups (they'd be invisible on the Errors screen and in 5xx emails).

## 010. Queue bounded by rows and bytes; batch decoding bounded too

- **Context:** The queue must stay bounded under overload. A row count alone isn't enough when bodies are large.
- **Choice:** Admission reserves both rows (`ingest.queue_capacity`) and approximate bytes (`ingest.max_queue_bytes`, 256 MB). A semaphore limits batches being decoded at once (2 × CPUs). In a 1 GB container under a 108k events/s overload, RSS peaked at 628 MB with no OOM.
- **Alternatives:** A bounded `mpsc` channel of batches (bounds batch count, not rows or bytes).

## 011. Concurrent ClickHouse inserts, validation off, mimalloc

- **Context:** A single sequential writer topped out near 29k rows/s.
- **Choice:** Up to `ingest.writer_concurrency` (4) inserts in flight; a waiting writer is what turns into backpressure. The clickhouse crate's per-insert schema validation is off for the hot path, with integration tests covering the row types. mimalloc is the global allocator, because glibc kept freed batch memory resident.
- **Alternatives:** ClickHouse `async_insert` (moves buffering into ClickHouse and makes failures harder to see), or the crate's streaming `Inserter` (harder to retry a failed batch).

## 012. Failed flushes are retried indefinitely while running

- **Context:** ClickHouse can be briefly unavailable.
- **Choice:** A failing flush retries with backoff (200 ms up to 10 s) for as long as the process runs. The rows stay counted against the queue, so ingest answers 429 rather than dropping data. During shutdown, retries stop after `server.shutdown_timeout_secs`; anything still unwritten is counted in `logbox_ingest_dropped_rows_total` and logged.
- **Alternatives:** Dropping after N attempts (silent data loss while the server reports success).

## 013. Separate live and error channels

- **Context:** The live tail tolerates loss (a lagging browser can skip events), but real-time 5xx emails must not miss errors because a live client is slow.
- **Choice:** Two topics: `requests` (summaries of every request, without bodies, headers or stacks) and `errors` (5xx signals, without bodies, headers or IPs). Each is an in-process broadcast in `all` mode and Redis pub/sub (`<prefix>:requests`, `<prefix>:errors`) in split mode. When Redis is configured, every process publishes and subscribes through Redis only, so both modes behave the same.
- **Alternatives:** One channel for both (alert input could be dropped under live-tail lag).

## 014. Live filters can't use bodies or headers

- **Context:** Live tail filters with the same expression language as Explore, evaluated in memory against the live summary.
- **Choice:** Live summaries carry every filterable scalar plus `attributes` and `correlation`, but not bodies or headers (too large to fan out to every subscriber). A live filter that references them is rejected with a clear message pointing to Explore.
- **Alternatives:** Ship full rows to the live channel (roughly 10× the fan-out bandwidth).

## 015. Body redaction normalizes key names

- **Context:** The default key list includes `access_token`, but apps send `accessToken` or `access-token` too.
- **Choice:** Key matching is case-insensitive and ignores `_` and `-`. Keys must match whole (`pinned` isn't `pin`). Dotted rules (`user.profile.dob`) match full paths, where `*` matches any key or array index. When a JSON body was truncated and no longer parses, a regex fallback still redacts `"key": value` pairs.
- **Alternatives:** Substring matching (`pin` would also redact `shipping`, `mapping` and `spinner`).

## 016. `IS NULL` means "empty"

- **Context:** Logbox stores no SQL `NULL`s; missing values are empty strings, absent map keys or zeros.
- **Choice:** `x IS NULL` compiles to `empty(x)` for strings and map values, `x = 0` for numbers and IDs, and epoch 0 for timestamps. The evaluator uses the same rule.
- **Alternatives:** Nullable columns (slower in ClickHouse, and every SDK would need to tell "absent" from "empty").

## 017. Unicode-aware string functions

- **Context:** ClickHouse's `lower`, `upper`, `length` and `position` work on bytes; most people expect characters.
- **Choice:** They compile to `lowerUTF8`, `upperUTF8`, `lengthUTF8` and `positionUTF8`. The evaluator uses Rust's Unicode-aware equivalents, and the differential test checks them on non-ASCII data.
- **Alternatives:** Byte semantics (surprising for non-English data).

## 018. Regex parity between RE2 and the live tail

- **Context:** ClickHouse runs `match()` with RE2; the live tail runs Rust `regex`, which accepts more syntax and makes `\d`, `\w`, `\s` and `\b` Unicode-aware.
- **Choice:** Patterns are parsed with `regex-syntax` and rejected when RE2 would reject them (stacked repetition, counts over 1000, nested classes and set operations, unsupported flags). For evaluation, those classes are rewritten to RE2's ASCII definitions. The differential test caught both problems.
- **Alternatives:** Using RE2 bindings in the evaluator (C++ build dependency), or documenting the differences.

## 019. A comparison needs a column

- **Context:** `1 = 1` and `'a' = 'a'` are harmless but are classic injection tautologies, and they never express a real filter.
- **Choice:** At least one side of every comparison, and the subject of every IN, BETWEEN, LIKE and IS NULL, must read a column or map value.
- **Alternatives:** Allowing constant comparisons.

## 020. The time picker always applies

- **Context:** The spec says the time range comes from the picker "unless the expression narrows it further".
- **Choice:** The server always wraps the filter as `project_id = … AND timestamp BETWEEN picker range AND (filter)`. Conditions on `timestamp` in the filter AND with the range, so they can only narrow it. This also bounds every query's scan.
- **Alternatives:** Letting a `timestamp` condition replace the picker (filters could then scan all retained data).

## 021. Bodies and headers are gated in filters too

- **Context:** Members without *can view bodies* must not see bodies or headers. Filtering on them would leak their contents one guess at a time.
- **Choice:** `req_body`, `res_body`, `req_headers` and `res_headers` are marked sensitive in the registry. Referencing them in any query mode is an error unless the member has the permission.
- **Alternatives:** Hiding them only in results.

## 022. No casts

- **Context:** The spec rejects "casts to unsafe types".
- **Choice:** All casts (`CAST`, `::`, `toString()` and friends) are rejected. Comparisons already coerce literals to the column's type (timestamps from strings, IDs from strings or numbers, booleans from 0/1), which covers the real use cases.
- **Alternatives:** A cast whitelist (more surface area, more ways for SQL and the evaluator to disagree).

## 023. Install-wide roles

- **Context:** The spec defines owner, admin, member and viewer roles without saying whether they're per project.
- **Choice:** Roles and *can view bodies* are install-wide; every member sees every project. A small team running one Logbox rarely needs per-project ACLs. API tokens can be bound to one project for narrower access.
- **Alternatives:** Per-project membership tables (more UI and more checks for a rare need; can be added later without breaking the API).

## 024. Sessions, CSRF and login throttling

- **Context:** The spec asks for HttpOnly/Secure/SameSite=Lax cookies, CSRF tokens and login rate limiting.
- **Choice:** A random session token in the cookie, with only its SHA-256 stored. A per-session CSRF token returned at login and by `/auth/me`, required in `X-CSRF-Token` on state-changing requests and compared in constant time. Failed sign-ins are limited in memory per email (8) and per IP (30) over 15 minutes; unknown emails still run a dummy password check, so response timing doesn't reveal which accounts exist. Changing a password signs out the user's other sessions.
- **Alternatives:** Double-submit cookies (no server state, but weaker if a subdomain is compromised), or a persistent limiter in Postgres (unnecessary for one process; split-mode API replicas each limit independently).

## 025. Invites are links

- **Context:** Members need a way to join before email is configured.
- **Choice:** Admins create an invite and get a single-use link valid for 7 days to share. Email delivery of invites can use the SMTP settings once alerts land.
- **Alternatives:** Admins setting initial passwords (they would know them).

## 026. Aliases never shadow columns

- **Context:** In ClickHouse, a SELECT alias is visible in WHERE. `toUnixTimestamp64Milli(timestamp) AS timestamp` made `WHERE timestamp >= …` compare the integer alias, so queries silently returned nothing. The integration test caught it.
- **Choice:** Transformed output columns are aliased with a `_c_` prefix, which `fetch_json` strips.
- **Alternatives:** The `prefer_column_name_to_alias` setting (easy to forget on one query).

## 027. Replay safety

- **Context:** Replay sends stored requests to other systems.
- **Choice:** Only admin-configured targets (http/https, no credentials, no query) are allowed. Redirects aren't followed, with a 30-second timeout. Auth headers (`authorization`, `cookie`, `x-api-key`, `proxy-authorization`) and redacted values are never replayed, and requests whose stored body was truncated are refused. Replaying requires the member role and *can view bodies*; every replay is recorded.
- **Alternatives:** Arbitrary URLs (an SSRF vector), or replaying stored auth headers (they're redacted anyway).

## 028. No new tests after M3

- **Context:** During M4 the user asked to skip tests to move faster.
- **Choice:** From M4 on, no new test suites are written or run as part of the milestones. Builds, clippy and the TypeScript typecheck/lint stay green, and the existing tests stay in CI.
- **Alternatives:** Keep the original "every milestone passes its tests" bar (the user explicitly chose speed).

## 029. Hand-rolled SVG charts

- **Context:** The spec allows uPlot or hand-rolled SVG/canvas, with an initial JS budget of 300 KB gzipped.
- **Choice:** Sparklines, the stacked histogram (with release markers and drag-to-zoom), the route heatmap and the span waterfall are small React SVG components styled with the design tokens, so both themes come for free. The initial bundle is about 177 KB gzipped. CodeMirror (about 124 KB) is code-split and loads only with SQL mode.
- **Alternatives:** uPlot (canvas, faster for very large series, but would need its own theming and accessibility work).

## 030. Query state lives in the URL

- **Context:** "The whole query is encoded in the URL, so every view is shareable."
- **Choice:** `mode`, `q` (SQL or search text), `chips` (JSON) and the time range (`range`, or absolute `from`/`to` after a zoom) are route search params. The inspector keeps the originating filter in its URL, so "nearby requests" and the back link preserve context.
- **Alternatives:** In-memory state with a "copy link" button.

## 031. Live tail with chip filters

- **Context:** The live SSE endpoint accepts SQL or search text; the Filters mode holds chips.
- **Choice:** Chips are rendered to SQL client-side (`addCondition`) for the live stream; Explore sends chips as-is. The server parses either into the same AST.
- **Alternatives:** A chips parameter on the SSE endpoint (more URL encoding for the same result).

## 032. Journeys: key guessing, related events and collapsing

- **Context:** A journey is "everything that happened for this value", and the search box should accept a bare ID.
- **Choice:** `key:value` searches a correlation key (or `user_id`, `request_id`, `trace_id`). A bare value is a trace ID when it is 32 hex characters, a request ID when it starts with `req_`, and a user ID otherwise. Events are included when they carry the key or were triggered by one of the matched requests (`request_id`), so jobs show up even if the SDK didn't copy the correlation keys. Runs of more than 5 items with the same lane, title and status collapse into one "N similar" row, and the server caps reads at 2,000 requests and 2,000 events. The UI defaults to a 24-hour window and refreshes every 10 s while the journey is in progress.
- **Alternatives:** A dedicated journeys table maintained at ingest (faster for very long journeys, but another write path and more storage); requiring `key:value` always (less forgiving).

## 033. Timestamps in API responses are RFC 3339

- **Context:** `time::OffsetDateTime` fields were serialized with the crate's default tuple format (`[2026, 297, 18, …]`), which the UI couldn't parse.
- **Choice:** Every `OffsetDateTime` in a response uses `time::serde::rfc3339` (strings like `2026-10-24T18:27:19.277Z`). Query results keep epoch milliseconds, as before.
- **Alternatives:** Epoch milliseconds everywhere (consistent with query rows, but less readable in admin responses).

## 034. Release markers everywhere, filtered where it matters

- **Context:** "Release markers everywhere": histograms, heatmaps, route charts, error charts and journeys.
- **Choice:** One `TimeChart` SVG component draws lines and bars with release markers and an optional threshold (the project's `slowMs` as the SLO). Labels closer than 8% of the width are dropped (the lines stay). The error group chart only shows releases of the group's service; other charts show all releases in the environment. Latency lines break where a bucket had no traffic instead of dropping to zero.
- **Alternatives:** One marker style per chart, or showing all releases on the error chart (noisy with many services).

## 035. Settings on one page

- **Context:** M5 adds settings for projects, keys, members, tokens, redaction, replay targets, sampling and retention.
- **Choice:** A single `/settings` page with a section list (`?tab=`), showing only the sections the signed-in role can use. Secrets (ingest keys, API tokens, invite links) are shown once with a copy button. Sampling and ignored routes are project settings delivered to SDKs through `/v1/config`; retention is read-only in the UI because it is applied to ClickHouse TTLs at startup.
- **Alternatives:** Separate pages per area (more navigation for rarely used screens).

## 036. One secret key, with a database fallback

- **Context:** The email brief asks for the SMTP password to be encrypted with `LOGBOX_SECRET_KEY` (AES-256-GCM or XChaCha20-Poly1305) and for signed mute links. Channel secrets (webhook and Slack URLs, bot tokens) need the same protection.
- **Choice:** XChaCha20-Poly1305 with a random 24-byte nonce per value (`nonce || ciphertext`), and HMAC-SHA256 for mute links and webhook signatures. Two subkeys are derived from the one secret with SHA-256 and distinct labels. When `LOGBOX_SECRET_KEY` is unset, a random key is generated once and stored in the Postgres `settings` table, and the UI and the logs warn about it, so `docker compose up` works without extra setup.
- **Alternatives:** Refusing to start without the key (a worse first run), AES-256-GCM (random 96-bit nonces are uncomfortable for long-lived keys), or a KDF like HKDF (unnecessary for a high-entropy secret).

## 037. SMTP settings: configuration wins per field

- **Context:** SMTP can be set in the UI or through `LOGBOX_SMTP_*`, and "env wins over UI values, and the UI shows those fields as locked".
- **Choice:** A field is locked when its env var is present or `logbox.toml` sets it to something other than the default. Locked fields come from configuration and the rest from the stored UI values. The UI password is encrypted, never returned (only `passwordSet`), kept when the form sends `null`, and cleared with an empty string. SMTP counts as configured with a host and a from address. Every send opens its own connection with a 20-second timeout.
- **Alternatives:** All-or-nothing (env set → UI read-only), which breaks the common case of setting only the password through a secret store.

## 038. Email on 5xx runs on the error signal stream

- **Context:** The rule must fire in real time, within 30 seconds, not on the 60-second loop, and must never include bodies, headers or IPs.
- **Choice:** The engine subscribes to the `ErrorSignal` stream ingest publishes after each flush (built in M1). Signals carry no bodies, headers or IPs, so emails can't leak them. The rule's filter is evaluated in memory with the query language evaluator over the signal's fields (`ErrorSignal` implements `Record`). Rules, recipients, SMTP status and mutes are reloaded every 30 seconds. Projects without an active rule are skipped before any work.
- **Alternatives:** Querying ClickHouse on a short loop (more load, slower, and more columns within reach).

## 039. Noise control lives in memory and is saved to Postgres

- **Context:** First occurrence immediately, then at most one digest per 15 minutes per group, a regression after 24 quiet hours, and a cap of 30 emails per project per hour with one "paused" email.
- **Choice:** A pure `Throttle` (no I/O, explicit timestamps) holds per-group state. The engine saves changed groups to `email_groups` every 15 seconds and on shutdown, and reloads groups seen in the last 30 days at startup, so restarts don't resend "first" emails. Digests go out when a new occurrence arrives after the window or from the 15-second timer when the window ended with occurrences pending. The cap counts messages (one email to five recipients counts once) per UTC clock hour. Occurrences over the cap stay pending, so the next digest includes them. Errors without a fingerprint are grouped by status, method and route.
- **Alternatives:** Keeping all state in Postgres with row locks per signal (a database round trip per error during an incident), or a sliding-hour cap (harder to explain in the "paused until" email).

## 040. Mute links confirm before muting

- **Context:** "Mute this error for 1h / 24h / forever" links must work without logging in and expire.
- **Choice:** The token is `base64url(json).base64url(hmac)` with the project, fingerprint, duration and an expiry (7 days by default). The link opens a small confirmation page, and the mute applies on a form `POST`, because mail security scanners fetch every link in an email and would otherwise mute errors on delivery. Mutes are stored on `email_groups.muted_until` (`infinity` for forever) and can be lifted under Alerts.
- **Alternatives:** Muting on `GET` (simpler, but scanners trigger it), or single-use tokens (needs a token table; a replayed mute is harmless).

## 041. One outbox table that is also the history

- **Context:** Sends need a persistent queue with exponential backoff and 5 attempts, and every send must be recorded and shown under Alerts → History.
- **Choice:** Each message per recipient (or per channel) is a row in `notifications` holding the rendered message, status, attempts, next attempt, last error and the server's response. The worker claims due rows with `FOR UPDATE SKIP LOCKED` and a 2-minute lease, so a crashed worker's rows are retried and two workers never send the same row. Retries wait 30 s, 1, 2 and 4 minutes. Test sends run synchronously so the UI can show the exact SMTP or HTTP error, and they are recorded in the same table. History is the same rows, newest first.
- **Alternatives:** Separate queue and log tables (two writes per send, and they can drift apart), or an external queue (another service to run).

## 042. Alert rules: state, cooldown and links

- **Context:** "A rule is a query, a condition and a window … States ok/firing/resolved with dedupe and a cooldown, so each state change sends one notification … Messages link back to the UI with the query pre-filled."
- **Choice:** Rules are stored with the same `QueryInput` as Explore and compiled with `logbox_query::where_clause`, so filters are parameterized and run as the read-only user. Bodies and headers can't be used in rule filters. The stored states are `ok` and `firing`; "resolved" is the firing → ok transition. A firing notification is skipped if the previous notification was less than the cooldown ago, and a resolved notification is sent only if the firing was announced. Editing the definition resets the state. Error rate and p95 count requests as stored, while counts are weighted by `1 / sample_rate`. Links open Explore with the query and the evaluated window as absolute `from`/`to`. Channels are per project and created by admins. Rules can be edited by members.
- **Alternatives:** Pending states with "for N minutes" (more to explain; the window already smooths spikes), or weighting error rate by sampling (the ratio is the same when sampling is uniform).

## 043. A single alerts process in split mode

- **Context:** In split mode every subscriber receives every Redis message, and the rule loop has no leader election.
- **Choice:** Run exactly one `logbox alerts` process (documented in docs/alerts.md). The outbox itself is safe with several workers, but the 5xx engine and the rule loop would each send twice.
- **Alternatives:** Leader election with a Postgres advisory lock (worth adding if someone needs alert high availability).

## 044. One HTTP recorder per SDK, shared by its integrations

- **Context:** Express, Fastify and NestJS (and Django and ASGI in Python) must record requests the same way: ignore list, sampling, body policy, truncation, error fields.
- **Choice:** The core client owns `recordHttp(ctx, exchange)` / `record_http(ctx, exchange)`, which applies every rule. Integrations only collect the exchange. In Node, `observeHttp()` instruments any `IncomingMessage`/`ServerResponse` pair (context, request ID header, a bounded copy of the response body taken in `write`/`end`, record on `close`). Express and NestJS (on both platforms, where Nest middleware receives the raw Node objects) use it. Fastify uses its hooks and calls `recordHttp` directly.
- **Alternatives:** Each integration building its own items (the rules would drift apart), or monkey-patching `http.Server` (catches everything, but no route patterns and surprising in a library).

## 045. Error details only on 5xx responses

- **Context:** Ingest fingerprints any request that carries an error type, so a handled `NotFoundException` or validation error would create error groups.
- **Choice:** SDKs attach the exception's type, message and stack only when the response status is ≥ 500. The status already marks 4xx responses, and their bodies are still captured under the `errors` policy.
- **Alternatives:** Fingerprinting only 5xx on the server (but an SDK sending a 4xx with an error type may mean it, e.g. from a custom `captureRequest`).

## 046. SDKs load project settings, code wins

- **Context:** Decision 035 made sampling, body capture and ignored routes project settings delivered through `/v1/config`, but no SDK read them.
- **Choice:** SDKs fetch `/v1/config` at startup and every 5 minutes (in the background) and apply each setting the application didn't set in code. Node skips it when a custom `fetch` is injected (tests and special transports) unless `remoteConfig: true` is passed; Python has `remote_config=False`. The Python flush thread starts with the client, so the settings arrive before the first request.
- **Alternatives:** Server settings always win (surprising when code sets a rate explicitly), or a fetch per flush (needless load).

## 047. Jobs belong to the request that started them

- **Context:** A job started in a request handler (`trackJob` without a `context`) got fresh IDs, so it didn't show up in the request's journey.
- **Choice:** `trackJob` / `track_job` default to the current context: the job's events carry the request's ID, trace, user and correlation keys, with a new span ID. Jobs in other processes restore a serialized context (BullMQ job data `__logbox`, a Celery message header `logbox`).
- **Alternatives:** Always new IDs, linking only through correlation keys (journeys would miss jobs that don't repeat the keys).

## 048. Fastify without fastify-plugin, context re-entered before handlers

- **Context:** The plugin must apply to the whole instance, and Fastify parses bodies on stream events, outside the request's async context.
- **Choice:** The plugin sets the `skip-override`, display-name and plugin-meta symbols that fastify-plugin would set, avoiding a dependency whose v6 compatibility with Fastify 5 isn't documented. `onRequest` runs the rest of the lifecycle inside the context, and `preHandler` enters it again with `enterWith`. NestJS does the same on the Fastify platform by adding a `preHandler` hook through the HTTP adapter, so guards (which run before interceptors) see the context too.
- **Alternatives:** `AsyncResource` binding of every hook (more code for the same result), or documenting that handlers lose the context (not acceptable).

## 049. NestJS: middleware, interceptor and an opt-in filter

- **Context:** The brief asks for a module, an interceptor and an exception filter that captures errors, plus a `Logger` bridge.
- **Choice:** `LogboxModule.forRoot()` applies the middleware to every route (`{*path}`, the Nest 11+ wildcard, so the peer range is Nest 11 and 12) and registers the interceptor globally; the interceptor records handler exceptions and sets the route. `LogboxExceptionFilter` (a `BaseExceptionFilter`, so responses are unchanged) is registered only with `exceptionFilter: true`, because a global catch-all filter could shadow or be shadowed by the application's own. `LogboxLogger` extends `ConsoleLogger` and skips Nest's startup contexts and `ExceptionsHandler`, whose unhandled-exception lines would create a second error group for the error the request already carries. The package is built with tsup and uses explicit `@Inject()` tokens, since esbuild doesn't emit decorator metadata.
- **Alternatives:** Always registering the filter (conflicts), or a filter only (misses the route pattern).

## 050. Python SDK: standard library only

- **Context:** The SDK runs inside Django, FastAPI and Celery processes, often preforked.
- **Choice:** No runtime dependencies: `urllib` for HTTP (gzip NDJSON), a lock-guarded `deque` (so a failed batch can go back to the front and the oldest items are the ones dropped), a daemon flush thread, `contextvars` for context and `atexit` for the final flush. `os.register_at_fork` gives a forked child an empty queue, fresh locks and a new thread. The ASGI middleware is plain ASGI (no Starlette import), reads the route from `scope["route"]`, and must be added last, since Starlette runs the most recently added middleware outermost. The Django middleware supports sync and async stacks, reads bodies only when the capture policy keeps them, and records the authenticated user only if Django already loaded it, to avoid a query. Celery is instrumented with signals: `before_task_publish` adds the context header, `task_prerun`/`task_postrun` record the run, and retries count as failed attempts.
- **Alternatives:** httpx or requests (a dependency for every user), an asyncio transport (doesn't fit WSGI or Celery), or a Celery base task class (users would have to change their tasks).

## 051. SDK benchmarks use synthetic requests

- **Context:** The target is under 0.2 ms p99 added per request, which network noise would hide.
- **Choice:** Each benchmark measures the integration in process: Express with synthetic request and response objects against the bare handler, Fastify with `app.inject()` on two identical apps, Python's ASGI and Django middleware around a trivial app or view. Every benchmark exits non-zero over budget. Results are in docs/benchmarks.md.
- **Alternatives:** Load tests over real sockets (useful for throughput, not for a 0.2 ms budget).

## 052. Examples instead of new SDK tests

- **Context:** The brief asks for tests in every package; the user asked to skip new tests from M4 on (decision 028).
- **Choice:** No new test suites. New Node packages run `vitest run --passWithNoTests` so CI stays green, and the core keeps its existing tests. Every integration was verified against a running Logbox with the apps in `examples/` (Express, Fastify, NestJS on both platforms with BullMQ, FastAPI, Django with a prefork Celery worker): routes, request IDs, `traceparent`, users, correlation keys, 5xx errors, job linking and attempts, log lines, spans and remote settings.
- **Alternatives:** Writing the brief's test suites now (against the user's instruction).
