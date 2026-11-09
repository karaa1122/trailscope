# Decisions

One entry per decision that the spec left open. Newest at the bottom.

## 001. Development toolchain runs in containers

- **Context:** The development machine has Node and Docker but no Rust or Python toolchain.
- **Choice:** Rust (`rust:1-bookworm`) and Python (`python:3.13-slim`) run in long-lived dev containers with the repo bind-mounted. ClickHouse and Postgres for integration tests run as containers on a `trailscope-dev` network. CI uses native toolchains and service containers.
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

- **Context:** Configuration comes from `trailscope.toml` and env vars.
- **Choice:** `TRAILSCOPE_<SECTION>_<KEY>` maps to `<section>.<key>`, e.g. `TRAILSCOPE_CLICKHOUSE_READER_PASSWORD`. Section names have no underscores, so the first underscore after the section is unambiguous. String fields that hold secrets accept bare numbers (`TRAILSCOPE_CLICKHOUSE_PASSWORD=123456`).
- **Alternatives:** figment's `__` separator (`TRAILSCOPE_CLICKHOUSE__URL`), which is easy to get wrong.

## 005. Repository URL and image name

- **Context:** The project needs a canonical URL for package metadata and image labels.
- **Choice:** `https://github.com/karaa1122/trailscope`. CI publishes to `ghcr.io/${{ github.repository }}`, so forks work without edits. Compose builds `trailscope:local` from source by default; `TRAILSCOPE_IMAGE` selects a published image.
- **Alternatives:** A placeholder organisation.

## 006. Distroless runtime image and built-in healthcheck

- **Context:** The final image should be slim and non-root, and compose needs a healthcheck.
- **Choice:** `gcr.io/distroless/cc-debian13:nonroot`. It has no shell or curl, so the binary has a hidden `trailscope healthcheck` subcommand that probes `/healthz` on the local listener.
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
- **Choice:** A failing flush retries with backoff (200 ms up to 10 s) for as long as the process runs. The rows stay counted against the queue, so ingest answers 429 rather than dropping data. During shutdown, retries stop after `server.shutdown_timeout_secs`; anything still unwritten is counted in `trailscope_ingest_dropped_rows_total` and logged.
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

- **Context:** Trailscope stores no SQL `NULL`s; missing values are empty strings, absent map keys or zeros.
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
- **Choice:** Roles and *can view bodies* are install-wide; every member sees every project. A small team running one Trailscope rarely needs per-project ACLs. API tokens can be bound to one project for narrower access.
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

- **Context:** The email brief asks for the SMTP password to be encrypted with `TRAILSCOPE_SECRET_KEY` (AES-256-GCM or XChaCha20-Poly1305) and for signed mute links. Channel secrets (webhook and Slack URLs, bot tokens) need the same protection.
- **Choice:** XChaCha20-Poly1305 with a random 24-byte nonce per value (`nonce || ciphertext`), and HMAC-SHA256 for mute links and webhook signatures. Two subkeys are derived from the one secret with SHA-256 and distinct labels. When `TRAILSCOPE_SECRET_KEY` is unset, a random key is generated once and stored in the Postgres `settings` table, and the UI and the logs warn about it, so `docker compose up` works without extra setup.
- **Alternatives:** Refusing to start without the key (a worse first run), AES-256-GCM (random 96-bit nonces are uncomfortable for long-lived keys), or a KDF like HKDF (unnecessary for a high-entropy secret).

## 037. SMTP settings: configuration wins per field

- **Context:** SMTP can be set in the UI or through `TRAILSCOPE_SMTP_*`, and "env wins over UI values, and the UI shows those fields as locked".
- **Choice:** A field is locked when its env var is present or `trailscope.toml` sets it to something other than the default. Locked fields come from configuration and the rest from the stored UI values. The UI password is encrypted, never returned (only `passwordSet`), kept when the form sends `null`, and cleared with an empty string. SMTP counts as configured with a host and a from address. Every send opens its own connection with a 20-second timeout.
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
- **Choice:** Rules are stored with the same `QueryInput` as Explore and compiled with `trailscope_query::where_clause`, so filters are parameterized and run as the read-only user. Bodies and headers can't be used in rule filters. The stored states are `ok` and `firing`; "resolved" is the firing → ok transition. A firing notification is skipped if the previous notification was less than the cooldown ago, and a resolved notification is sent only if the firing was announced. Editing the definition resets the state. Error rate and p95 count requests as stored, while counts are weighted by `1 / sample_rate`. Links open Explore with the query and the evaluated window as absolute `from`/`to`. Channels are per project and created by admins. Rules can be edited by members.
- **Alternatives:** Pending states with "for N minutes" (more to explain; the window already smooths spikes), or weighting error rate by sampling (the ratio is the same when sampling is uniform).

## 043. A single alerts process in split mode

- **Context:** In split mode every subscriber receives every Redis message, and the rule loop has no leader election.
- **Choice:** Run exactly one `trailscope alerts` process (documented in docs/alerts.md). The outbox itself is safe with several workers, but the 5xx engine and the rule loop would each send twice.
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
- **Choice:** `trackJob` / `track_job` default to the current context: the job's events carry the request's ID, trace, user and correlation keys, with a new span ID. Jobs in other processes restore a serialized context (BullMQ job data `__trailscope`, a Celery message header `trailscope`).
- **Alternatives:** Always new IDs, linking only through correlation keys (journeys would miss jobs that don't repeat the keys).

## 048. Fastify without fastify-plugin, context re-entered before handlers

- **Context:** The plugin must apply to the whole instance, and Fastify parses bodies on stream events, outside the request's async context.
- **Choice:** The plugin sets the `skip-override`, display-name and plugin-meta symbols that fastify-plugin would set, avoiding a dependency whose v6 compatibility with Fastify 5 isn't documented. `onRequest` runs the rest of the lifecycle inside the context, and `preHandler` enters it again with `enterWith`. NestJS does the same on the Fastify platform by adding a `preHandler` hook through the HTTP adapter, so guards (which run before interceptors) see the context too.
- **Alternatives:** `AsyncResource` binding of every hook (more code for the same result), or documenting that handlers lose the context (not acceptable).

## 049. NestJS: middleware, interceptor and an opt-in filter

- **Context:** The brief asks for a module, an interceptor and an exception filter that captures errors, plus a `Logger` bridge.
- **Choice:** `TrailscopeModule.forRoot()` applies the middleware to every route (`{*path}`, the Nest 11+ wildcard, so the peer range is Nest 11 and 12) and registers the interceptor globally; the interceptor records handler exceptions and sets the route. `TrailscopeExceptionFilter` (a `BaseExceptionFilter`, so responses are unchanged) is registered only with `exceptionFilter: true`, because a global catch-all filter could shadow or be shadowed by the application's own. `TrailscopeLogger` extends `ConsoleLogger` and skips Nest's startup contexts and `ExceptionsHandler`, whose unhandled-exception lines would create a second error group for the error the request already carries. The package is built with tsup and uses explicit `@Inject()` tokens, since esbuild doesn't emit decorator metadata.
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
- **Choice:** No new test suites. New Node packages run `vitest run --passWithNoTests` so CI stays green, and the core keeps its existing tests. Every integration was verified against a running Trailscope with the apps in `examples/` (Express, Fastify, NestJS on both platforms with BullMQ, FastAPI, Django with a prefork Celery worker): routes, request IDs, `traceparent`, users, correlation keys, 5xx errors, job linking and attempts, log lines, spans and remote settings.
- **Alternatives:** Writing the brief's test suites now (against the user's instruction).

## 053. OTLP/HTTP receiver: hand-written messages, server spans become requests

- **Context:** M8 asks for an OTLP receiver at `/v1/otlp/v1/traces` and `/v1/otlp/v1/logs`, mapped into spans and events.
- **Choice:** OTLP/HTTP with protobuf or JSON (gzip allowed), authenticated with an ingest key. The dozen OTLP messages the mapping reads are declared in `otlp.rs` with prost derives (unknown fields skipped), and the same structs derive serde for OTLP/JSON (hex IDs, 64-bit integers as strings or numbers). Spans become spans; a server span with an HTTP method also becomes a request (request ID = span ID; route, path, status, user and client from semantic conventions; the first `exception` event as the error), so OpenTelemetry-instrumented services show up in Live tail, Explore, Errors, alerts and Email on 5xx. Route syntaxes (`{id}`, `<int:id>`) are rewritten to `:id`. Log records become `log` events. Everything then goes through the same normalization, redaction, limits and queue as `/v1/ingest`, and rejected items are reported in `partialSuccess`. Metrics and OTLP/gRPC are out of scope.
- **Alternatives:** The `opentelemetry-proto` crate (its `trace` and `logs` features pull in the OpenTelemetry SDK), spans only (Trailscope's views are built on requests), or a separate `/v1/traces` path (the spec names `/v1/otlp`).

## 054. OIDC sign-in: code flow with PKCE, link by subject then email

- **Context:** Optional OIDC login, configured by `[oidc]`; `/auth/me` already reported whether it's enabled.
- **Choice:** `GET /api/v1/auth/oidc/start` redirects to the provider with `state`, `nonce` and a PKCE challenge kept in a short-lived signed cookie (HMAC with the secret key, `SameSite=Lax`, scoped to the callback path). The callback checks the state, exchanges the code (`client_secret_basic` unless the provider only lists `client_secret_post`), validates the ID token against the provider's JWKS (issuer, audience, expiry with 60 s leeway, nonce; keys refreshed once on an unknown `kid`; HS* tokens with the client secret), and takes the email from the token or the userinfo endpoint. A member is found by `oidc_subject`, then by email (not marked unverified), and the subject is stored; unknown people get a viewer account only for `auto_create_domains`. Failures redirect to `/login?error=…`. `GET /api/v1/setup` now reports OIDC, since the sign-in page has no session. Verified against a mock provider: existing member linked, viewer auto-created, unknown domain refused, replayed callbacks rejected.
- **Alternatives:** The `openidconnect` crate (more complete, many more dependencies), linking by email only (a provider reusing an address could take over an account), or storing pending logins in Postgres (a signed cookie needs no cleanup).

## 055. No Playwright e2e suite

- **Context:** M8 lists "Playwright e2e and screenshots"; tests are skipped from M4 on (decision 028).
- **Choice:** No e2e test suite or CI job. Playwright is used for the README screenshots (`e2e/screenshots.mjs`) and for the manual checks behind docs/benchmarks.md.
- **Alternatives:** Writing the suite now (against the user's instruction); it can be added later on top of `e2e/`.

## 056. The Python SDK is published as `logbox-sdk` (superseded by 062)

- **Context:** The name `logbox` on PyPI belongs to an unrelated project (a terminal output toolbox), so the release workflow couldn't publish under it. The npm names `@logbox/*` are unclaimed.
- **Choice:** The distribution is `logbox-sdk` (`pip install logbox-sdk`); the import name stays `logbox`, so code and docs keep `import logbox`. Whoever publishes the first release must also own the `@logbox` npm scope (or rename the Node packages) and set up PyPI trusted publishing for `logbox-sdk`.
- **Alternatives:** Renaming the import package as well (longer imports for no benefit), or asking the owner of `logbox` to transfer it (uncertain and slow).

## 057. Django keeps the text fields of multipart forms

- **Context:** Multipart bodies were skipped entirely, so a login form sent as `form-data` (Postman's default) showed no request body even on errors.
- **Choice:** For `multipart/form-data`, the Django middleware records the form fields Django or DRF already parsed (`request._post`, `request._files`): text values as-is (then redacted like JSON keys), files as `[file name, size, type]`. If the view never parsed the form, nothing is read, so logging never parses a large upload. Raw multipart bytes are still skipped in every SDK.
- **Alternatives:** Keeping multipart skipped (useless for form logins), or parsing the form in the middleware (would buffer uploads the application meant to stream).

## 058. Explore reads jobs and log events too

- **Context:** Explore only read the `requests` table, so job runs and log events (the `events` table) could be seen in Journeys but not filtered, charted, faceted or grouped. The query endpoints already accepted `table: "events"`.
- **Choice:** A "Data" switch on Explore (Requests | Jobs & logs), kept in the URL as `?table=events`; without it Explore reads requests, so existing links, saved views and alert links are unchanged. Each table has its own default columns, facets (`kind` first, so jobs are one click away) and default grouping (`name`). Switching keeps the query and the time range and resets the columns and the grouping. An event row opens the request that started it, as in Journeys, without passing the events filter along (the inspector applies it to the nearby requests). Saved views store and restore the table. The list/group switch now reads "List | Group by" so it doesn't repeat "Requests".
- **Alternatives:** One list mixing requests and events (different columns, statuses and sampling, and one keyset cursor can't page both tables), or a separate Jobs page (would duplicate Explore's chart, facets, grouping and query bar).

## 059. Replay checks only whether the request body was cut

- **Context:** One `body_truncated` flag covered both bodies, so a request with a complete body and a long (cut) response, typically an error page, couldn't be replayed ("the stored body was truncated").
- **Choice:** A `req_body_truncated` column (ClickHouse migration 0003) and an optional `reqBodyTruncated` wire field. The server sets it when it cuts the request body itself; the SDKs send it next to `bodyTruncated`. When an older SDK sends only `bodyTruncated`, it counts for the request body too, and rows written before the migration inherit `body_truncated` through the column default, so replay stays cautious where it can't know. Replay refuses only on `req_body_truncated`; the column is also queryable.
- **Alternatives:** Guessing from the stored body's length (wrong whenever an SDK's limit is lower than the server's), or replacing `body_truncated` with two new flags (breaks existing queries, saved views and older SDKs).

## 060. Empty TRAILSCOPE_* variables count as unset

- **Context:** The compose files didn't pass `TRAILSCOPE_SMTP_*` or `TRAILSCOPE_OIDC_*` to the container, so setting them in `deploy/.env` did nothing (and OIDC can only be configured this way). Passing them as `${TRAILSCOPE_SMTP_PORT:-}` would set empty values, which failed to parse for numbers and locked the SMTP fields in the UI to empty.
- **Choice:** The config loader skips `TRAILSCOPE_*` variables whose value is empty, and SMTP field locking uses the same rule. Both compose files now pass the SMTP and OIDC settings (and, in split mode, the secret key and retention) through; `.env.example` lists them.
- **Alternatives:** A separate env file per optional feature (more files to explain), or documenting that people must edit the compose file (the bug this fixes).

## 061. "Send test email" saves unsaved settings first

- **Context:** The test endpoint uses the stored SMTP settings, so after editing the form without saving, the test reported on the old settings (for example, a test "sent" to a local catch-all while the form showed a real provider).
- **Choice:** When the form has unsaved changes, the button reads "Save and send test email", saves, then sends; a line under it says so.
- **Alternatives:** Letting the test endpoint accept unsaved settings (a second code path for credentials, and the password would have to be sent again), or disabling the button until saved (one more step for the common case).

## 062. The project is called Trailscope

- **Context:** "Logbox" was the working name. Before publishing, it clashed with React Native's built-in error screen (LogBox), and `logbox` on PyPI belongs to another project (decision 056).
- **Choice:** Trailscope: following the trail of each request across services and jobs is what the tool is for. On the day of the rename, `trailscope` was free on PyPI and crates.io and as a GitHub user name, `@trailscope/*` was unpublished on npm, and no product of that name turned up in a search. Everything was renamed at once, before the first release: crates (`trailscope-*`), the binary and config file (`trailscope`, `trailscope.toml`), settings (`TRAILSCOPE_*`), the ClickHouse database and reader user, metrics (`trailscope_*`), headers (`X-Trailscope-*`), token prefixes (`trs_ingest_`, `trs_api_`), the SDKs (`@trailscope/*` on npm, `trailscope` on PyPI, now one name for install and import) and the key-derivation labels. Stored secrets encrypted under the old labels must be entered again.
- **Alternatives:** Keeping Logbox (crowded searches and a split PyPI name), Hopscope (catchier, less clear), or accepting the old token prefixes and key labels too (compatibility nobody needs before a first release, kept forever).

## 063. The Java SDK

- **Context:** Java services (mostly Spring Boot) had to use OpenTelemetry to reach Trailscope, without bodies, users, journey keys or job linking.
- **Choice:** A Maven build in `sdks/java` (Java 17+, group `dev.trailscope`): `trailscope-java`, a core with no runtime dependencies (`java.net.http`, a small JSON writer, a daemon flush thread) that follows `docs/sdk-spec.md` like the Node and Python cores; `trailscope-servlet`, a Jakarta Servlet 6 filter for any servlet container that keeps at most `maxBodyBytes` of each body; `trailscope-spring-boot-starter`, auto-configuration with `trailscope.*` properties, the Spring MVC route pattern, controller exceptions and `@Async` context through a `TaskDecorator` (only when the app has none of its own); and `trailscope-logback`, an appender the starter attaches for WARN and above. The context is a `ThreadLocal`, carried to other threads by `Trailscope.wrap()` and to other processes by `serializeContext()`. Error types are fully qualified class names. The signed-in principal's name is recorded as the user unless `trailscope.capture-principal=false`. An exception that escapes the filter is recorded at once as a 500 with its stack; the container's error page, written afterwards, isn't captured. The group ID needs the `trailscope.dev` domain to be verified on Maven Central; `io.github.karaa1122` is the fallback (Java packages stay `dev.trailscope`).
- **Alternatives:** Only an OpenTelemetry guide (no bodies, users or journeys), Gradle (Maven was on hand and is what Maven Central documents), depending on Jackson or Spring in the core (pulls versions into every app), `InheritableThreadLocal` (leaks context into pooled threads), simple class names as error types like Node and Python (ambiguous across packages in Java).

## 064. The PHP and Laravel SDK

- **Context:** PHP services (mostly Laravel) had no way to send requests, jobs and log lines to Trailscope, and PHP has no background threads, so the spec's "a background flush every second" can't be followed literally.
- **Choice:** One Composer package, `trailscope/trailscope` in `sdks/php` (PHP 8.2+): a framework-agnostic core (`Trailscope\`) and a Laravel integration (`Trailscope\Laravel\`, auto-discovered; `illuminate/*` only in dev and suggest), supporting Laravel 12 and 13 (11 is past its security support). Capture calls only append to the in-memory queue; the queue is sent after the response has gone out (Laravel's terminating callbacks, after `fastcgi_finish_request()` under PHP-FPM), between queue jobs and on idle worker loops, on Octane ticks and at request end, and in a shutdown function, with a 1 s connect and 3 s send timeout. Because PHP-FPM keeps nothing between requests, a small state file (in `storage/framework/cache`) shares the remote settings, the retry pause and the failure count between processes; items still unsent at the end of a request are dropped and counted. Stacks are formatted as `at Class->method (file:line)` lines so the server's fingerprinting can group them, and `/vendor/` frames are skipped there like `node_modules`. Job events are named after the job class; log lines whose exception already sits on a failed request or job are dropped. The package is published from a read-only mirror repository kept in sync by `.github/workflows/php-split.yml`, because Packagist reads `composer.json` at a repository root.
- **Alternatives:** Separate core and Laravel packages (two mirrors to keep), sending at the end of every request (an HTTP call per request, often while the worker could take the next one), a disk spool or a forked sender (moving parts and permissions on every host), the Laravel cache for shared state (not available to the core), native PHP traces (`#0 file(line)`, which the fingerprint wouldn't parse), or supporting Laravel 11 (no longer maintained upstream).

## 065. A website with planned Cloud pricing and the docs

- **Context:** The project needed a public home before announcing a hosted version: something that explains Trailscope, publishes the documentation, and measures interest in a Cloud before any of it is built. The domain is trailscope.pro, served from an existing Ubuntu server that already hosts other sites behind nginx with certbot.
- **Choice:** `website/`, built with Astro and Starlight. The landing page reuses the product's colours, fonts and the screenshots in `docs/images`. The docs are not a second copy: `scripts/sync-docs.mjs` turns `docs/*.md` into Starlight pages on every build (first heading becomes the title, links between docs become site links, links to other repository files go to GitHub, Mermaid blocks render in the browser, a link to a missing page fails the build), so GitHub and the site show the same text; `docs/README.md` is the index on both. Planned Cloud plans, stated as not live yet: Free (1M events a month, 7 days), Team ($19, 10M, 30 days, $1.50 per extra million), Business ($99, 50M, 90 days, SSO, $1.20 per extra million) and self-hosted free with everything, priced a little under comparable hosted tools, with an event being a request, job run or log line. The waitlist is a 200-line standard-library Python service (`waitlist/waitlist.py`, SQLite, per-address rate limit, honeypot field, works without JavaScript) behind the nginx site, run by systemd with `DynamicUser` and a read-only file system. The site loads nothing from other hosts, has no analytics and sets a strict Content-Security-Policy (inline scripts and WebAssembly allowed for Astro and the docs search). Deploys are timestamped releases with a `current` link, the pattern the server already used; `setup-server.sh` only reloads nginx when `nginx -t` passes and restores the previous config otherwise, and leaves the site alone once certbot has added HTTPS.
- **Alternatives:** VitePress (reads `docs/` in place, but the landing page would be Vue inside a docs theme), Docusaurus (heavier, React-based), hand-written HTML for everything (no search, sidebar or per-page navigation), a hosted form service for the waitlist (an outside account and a third party holding the emails), Caddy (the server's ports 80 and 443 already belong to nginx), publishing prices only after the Cloud exists (no way to test demand first).

## 066. The website's chrome stays quiet

- **Context:** The first version of trailscope.pro carried the usual signs of a generated landing page: lime colour washes behind the hero, the screenshots, the architecture diagram, the featured plan and the waitlist; lime on every label, check mark, icon tile and step number; uppercase tracked labels; headings at weight 650 to 700 with trailing periods; a pill badge above a centred, stacked hero with three actions; eight equal cards with tinted icon tiles; and a "Most teams" badge on a Cloud plan nobody can buy yet. We went through it with the unslop checklist (github.com/heydamir/unslop, CC BY 4.0).
- **Choice:** Polish, not a redesign: the palette, Geist, the logo, the sections, the copy and the real screenshots stay. The surfaces are the product's neutral ramp with hairlines; the lime marks only the primary action on each screen (the header button became secondary), so the colour on the page comes from the screenshots. Labels are sentence case in Geist Mono, muted; headings are weight 500 without trailing periods; spacing is on a 4px grid with one easing curve. The hero is left-aligned with one primary and one secondary action, and the product view starts higher so it shows on the first screen; on phones it keeps a readable size and is cropped on the right. The clone commands live only in the quickstart, with their copy button. The eight extra features are a list with hairlines and muted line icons instead of cards. The "Most teams" badge is gone because it can't be true before launch. One detail of the product's own: each feature label shows the web UI's shortcut for that page (`G J`, `G E`, `G R`, `↵` for the inspector) as keycaps.
- **Alternatives:** a full redesign with a new palette or a generated hero image (the real screenshots already show the product, and the lime is the product's own colour); keeping the hero centred with a shorter lede (still the template layout, and it pushed the product view below the first screen); keeping the install commands in the hero (a third action stacked under the buttons, duplicated in the quickstart).
