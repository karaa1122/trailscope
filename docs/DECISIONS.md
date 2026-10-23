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
