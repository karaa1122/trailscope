# AGENTS.md — handoff notes for whoever picks this up next

This file lets any agent (or human) continue building Logbox if the current session is lost. **Keep it current:** update the status table and "Next steps" at every milestone commit.

## What we are building

Logbox: an open-source, self-hosted API request logging tool (Rust server + ClickHouse + Postgres + React UI + Node/Python SDKs). It must stay domain-neutral: no industry-specific wording anywhere.

- Full original brief: [docs/agents/spec.md](docs/agents/spec.md). **Read this first.** It defines the milestones M0–M8, the data model, the query language, the UI and the performance targets.
- Follow-up brief (SMTP email on 5xx): [docs/agents/spec-email-notifications.md](docs/agents/spec-email-notifications.md).
- Visual reference for the UI: [design/](design/) (HTML mockups; match them).
- Every judgement call goes in [docs/DECISIONS.md](docs/DECISIONS.md) (context, choice, alternatives).

## Standing instructions from the user

- Don't ask questions; decide, log the decision in `docs/DECISIONS.md`, keep going.
- Work through the milestones in order. At each milestone end: everything builds, lints and passes tests, then **commit with a conventional commit message and push directly to `main`**.
- **Tests: skipped from M4 on at the user's request (2026-10-24).** Don't write or run test suites; still keep `cargo build`, `cargo clippy` and the TypeScript typecheck/lint green. Existing tests stay in CI.
- Quality bar: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, strict TypeScript, no `any` without a comment, no `unwrap()`/`expect()` in non-test Rust code except at startup (enforced by workspace lints; `clippy.toml` allows them in tests).
- Check the current stable version before pinning any crate, npm/PyPI package, Docker image or GitHub Action.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Status

| Milestone | State | Notes |
|---|---|---|
| M0 Scaffold | done (`7c6042c`) | workspace, binary + config, compose, CI skeleton |
| M1 Ingest | done | 50k ev/s sustained at p99 17 ms on 4 CPUs (docs/benchmarks.md); Node core SDK; demo and load test |
| M2 Query language | done | parser, registry, compiler, evaluator, search, chips; injection, property, differential (vs ClickHouse) and fuzz tests; docs/query-language.md |
| M3 API | done | auth (sessions+CSRF, tokens, roles, invites, rate limit), setup, projects/keys/rules/targets, query endpoints, inspector/cURL/replay, live SSE, views, OpenAPI at /api/docs; docs/api.md |
| M4 Web part 1 | done | shell, palette, setup/login/invite, query bar (3 modes, quick filters, CodeMirror), Live tail, Explore, Inspector; UI embedded via rust-embed; overview/heatmap/errors endpoints |
| M5 Web part 2 | done | Journeys (`/api/v1/journeys` + swimlane UI), Errors list/detail, Routes (full heatmap + `/api/v1/routes/series`), release markers on every chart, Saved views page, Settings; RFC 3339 timestamps in API responses |
| M6 Alerts + email-on-5xx | done | rules (count, 5xx rate, p95, new/spiking error group) with cooldown; webhook/Slack/Telegram/email channels; real-time Email on 5xx with throttle, digest, cap, regression and signed mute links; Postgres outbox with retries = History; SMTP settings with env locking; encrypted secrets; Mailpit `dev` profile; docs/alerts.md. Smoke-tested end to end against Mailpit (no new test suites, per the M4 instruction) |
| M7 SDKs | **next** | |
| M8 Harden and ship | todo | |

## Dev environment (Linux, current since M6)

The repo is at `/home/kk/Documents/LogBox` on Ubuntu 24.04. Rust 1.99.1 (rustup, picked up from `rust-toolchain.toml`), Node 24 (fnm), pnpm 9 and Docker are on the host, so cargo and pnpm run natively. Only the databases run in containers (same names and ports as on Windows):

```bash
docker network create logbox-dev
docker run -d --name logbox-dev-ch --network logbox-dev -p 18123:8123 -p 19000:9000 \
  -e CLICKHOUSE_USER=logbox -e CLICKHOUSE_PASSWORD=logbox -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  --ulimit nofile=262144:262144 -v logbox-dev-ch:/var/lib/clickhouse clickhouse/clickhouse-server:26.8
docker run -d --name logbox-dev-pg --network logbox-dev -p 25432:5432 \
  -e POSTGRES_USER=logbox -e POSTGRES_PASSWORD=logbox -e POSTGRES_DB=logbox \
  -v logbox-dev-pg:/var/lib/postgresql postgres:18-alpine
docker run -d --name logbox-dev-mailpit --network logbox-dev -p 1025:1025 -p 8025:8025 axllent/mailpit:v1.32.0
# later: docker start logbox-dev-ch logbox-dev-pg logbox-dev-mailpit
```

Run the server natively (UI at http://localhost:8090, owner@example.com / "correct horse battery" on the existing dev database; SMTP there points at Mailpit, host 127.0.0.1 port 1025):

```bash
pnpm build && cargo build -p logbox-server
LOGBOX_CLICKHOUSE_URL=http://127.0.0.1:18123 LOGBOX_CLICKHOUSE_USER=logbox LOGBOX_CLICKHOUSE_PASSWORD=logbox \
LOGBOX_POSTGRES_URL=postgres://logbox:logbox@127.0.0.1:25432/logbox LOGBOX_SERVER_LISTEN=127.0.0.1:8090 \
LOGBOX_SERVER_PUBLIC_URL=http://localhost:8090 LOGBOX_SECRET_KEY=dev-secret-key-for-local-testing-only \
./target/debug/logbox all
```

Everyday checks: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`, then `pnpm install && pnpm build && pnpm lint && pnpm typecheck`.

Linux machine notes:

- Other projects' containers run here (`ch_postgres` on 5433, `eventy_db` on 15432, `mongodb` on 27017, and more); leave them alone.
- clang isn't installed; the workspace builds without it.
- Playwright: `e2e` pins `@playwright/test` 1.64.1, which expects a newer Chromium than the one in `~/.cache/ms-playwright`. Launch with `executablePath: '/home/kk/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome'` instead of downloading another.
- The ingest wire format is camelCase (`requestId`, `durationMs`, `errorType`…); snake_case fields are silently ignored.

## Dev environment (previous Windows machine)

The host had Node 24 + pnpm 9 + Docker, but **no Rust or Python**. Toolchains ran in containers with the repo bind-mounted:

```bash
# Git Bash on Windows: always export this, or `-w /work` and `-v` paths get mangled
export MSYS_NO_PATHCONV=1

docker network create logbox-dev
# ClickHouse (HTTP 18123, native 19000), user logbox/logbox with access management
docker run -d --name logbox-dev-ch --network logbox-dev -p 18123:8123 -p 19000:9000 \
  -e CLICKHOUSE_USER=logbox -e CLICKHOUSE_PASSWORD=logbox -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  --ulimit nofile=262144:262144 -v logbox-dev-ch:/var/lib/clickhouse clickhouse/clickhouse-server:26.8
# Postgres on 25432 (15432 is used by another local project)
docker run -d --name logbox-dev-pg --network logbox-dev -p 25432:5432 \
  -e POSTGRES_USER=logbox -e POSTGRES_PASSWORD=logbox -e POSTGRES_DB=logbox \
  -v logbox-dev-pg:/var/lib/postgresql postgres:18-alpine
# Rust toolchain container (target dir and registry on named volumes)
docker run -d --name logbox-rust --network logbox-dev -v "C:\\Projects\\LogBox:/work" \
  -v logbox-cargo-registry:/usr/local/cargo/registry -v logbox-target:/target \
  -e CARGO_TARGET_DIR=/target -w /work rust:1-bookworm sleep infinity
docker exec logbox-rust sh -c 'rustup component add rustfmt clippy && apt-get update -qq && apt-get install -y -qq cmake clang'
# Python container
docker run -d --name logbox-py -v "C:\\Projects\\LogBox:/work" -v logbox-pip-cache:/root/.cache/pip \
  -w /work/sdks/python python:3.13-slim sleep infinity
docker exec logbox-py pip install -e ".[dev]"
```

From inside `logbox-rust`, ClickHouse is `http://logbox-dev-ch:8123` and Postgres is `postgres://logbox:logbox@logbox-dev-pg:5432/logbox`.

Everyday commands:

```bash
docker exec logbox-rust sh -c 'cd /work && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace'
docker exec logbox-rust sh -c 'cd /work && cargo deny check'      # cargo-deny is installed in the container
pnpm install && pnpm build && pnpm lint && pnpm typecheck && pnpm test   # on the host
docker exec logbox-py sh -c 'ruff check . && ruff format --check . && mypy && pytest -q'
```

Machine notes:

- If Docker Desktop has stopped (it did once, possibly when the machine slept), start `C:\Users\KARAA\AppData\Local\Programs\DockerDesktop\Docker Desktop.exe`, wait for `docker version`, then `docker start logbox-dev-ch logbox-dev-pg logbox-rust logbox-py`.

- Port 8080 was held by a SigNoz stack from another project; the user asked for it to be stopped (`docker compose -p signoz stop`, containers and volumes kept). Another local stack (`eventy`: ports 8000, 15432, 6380, 27017) is running and must be left alone.
- Bash heredocs containing an apostrophe inside `<<'EOF'` blocks have broken the tool's quoting before; use the Write tool for files with prose.
- Several crates have new majors whose APIs differ from older docs: clickhouse 0.15 (`client.insert::<T>(t).await?`, params via `.param()`), sqlx 0.9, reqwest 0.13, rand 0.10, sha2 0.11, utoipa 6, tower-http 0.7, sqlparser 0.63. Read the source under `/usr/local/cargo/registry/src/*/<crate>-<ver>/` when unsure.

## Pinned versions

Rust 1.99.1 (`rust-toolchain.toml`), ClickHouse 26.8 LTS, Postgres 18, Redis 8, Node 24, pnpm 9.15.9, TypeScript 6.0.3 (TS 7 isn't supported by typescript-eslint yet), Vite 8, React 19.3, Python 3.10+ (tested on 3.13). Workspace crate versions are in the root `Cargo.toml` `[workspace.dependencies]`.

## Architecture as built so far

- `crates/logbox-core`: `config` (defaults → `logbox.toml` → `LOGBOX_<SECTION>_<KEY>` env), `wire` (ingest JSON types), `rows` (ClickHouse row structs, `LiveRequest`, `ErrorSignal`), `ch` (writer and reader clients, migration runner, reader user and settings profile, TTL sync), `pg` (pool and sqlx migrations), `live` (`LiveBus`: broadcast, or Redis pub/sub in split mode, with separate `requests` and `errors` topics), `secrets` (token generation and SHA-256), `admin` (create project, create ingest key), `project` (`ProjectSettings` JSONB), `testing` (per-test databases from `LOGBOX_TEST_*`).
- `crates/logbox-ingest`: `/v1/ingest`, `/v1/releases`, `/v1/config`. Modules: `keys` (key cache), `decode` (gzip/zstd with a cap, array/NDJSON split), `normalize` (validation, route templating, truncation), `redact`, `fingerprint`, `pipeline` (queue bounded by rows and bytes; writer with 4 concurrent inserts; publishes live and 5xx signals after each flush).
- `crates/logbox-demo`: a deterministic generic SaaS traffic generator (`generator.rs`), backfill, live traffic and a scripted incident (release v1.43.0 breaks `GET /projects/:id/tasks`, v1.43.1 fixes it), and `--load` with `--target-eps` for open-loop latency tests.
- `crates/logbox-server`: the `logbox` binary. `serve.rs` wires the roles; `ops.rs` serves `/healthz`, `/readyz` and `/metrics`. Uses mimalloc. Shutdown order: stop accepting, then drain the writer.
- `migrations/clickhouse/0001_init.sql` (embedded with `include_str!` in `ch.rs`; add new files to its `MIGRATIONS` list) and `migrations/postgres/0001_init.sql` (sqlx).
- `sdks/node/packages/node`: `@logbox/node` core (queue, transport, redaction, context, sampling, `trackJob`, benchmark in `bench/`).
- `scripts/dev/rust.sh` runs a command in the Rust container with the test env vars; `scripts/dev/env.sh` holds the env for running `logbox` against the dev services.
- `crates/logbox-api`: `AppState` (config, ClickHouse, Postgres, LiveBus, optional ingest KeyCache, login limiter, replay HTTP client, shutdown token). Modules: `auth` (Auth extractor: session cookie + CSRF or Bearer API token; `require(Role, Scope)`, `can_view_bodies`), `setup`, `users` (login/logout/me/password, members, invites, tokens, `ClientIp`), `projects`, `query` (`prepare` → `Prepared`, `fetch_json` runs as the reader and strips the `_c_` alias prefix), `inspector`, `live`, `views`, `openapi` (utoipa-axum route table). The integration test is `tests/api.rs`.
- `web/src`: `main.tsx`, `router.tsx` (code-based TanStack routes, typed search params, `Guard`), `lib/` (`api.ts` typed client + CSRF, `types.ts`, `session.tsx` (me, projects, current project/env, theme), `query.ts` (QueryValue ↔ URL, addCondition, history), `live.ts` (SSE hook with pause buffer), `hotkeys.ts`, `format.ts`), `components/` (`Shell` sidebar/rail, `CommandPalette`, `QueryBar`, `SqlEditor` (lazy CodeMirror), `charts.tsx` (Heatmap, Histogram, TimeChart, Waterfall, JsonView), `TimeRange`, `ui.tsx`), `pages/` (`LiveTail`, `Explore`, `Inspector`, `Journeys`, `Errors` (list + group), `Routes`, `Views`, `Settings` (`?tab=`, including `SettingsNotifications` for SMTP), `Alerts` (`?tab=rules|email|channels|history`), `Auth` (login, invite, setup wizard), `Placeholder` (the "no projects" screen)). Styles: `styles/tokens.css` (dark + light tokens), `styles/app.css`.
- `crates/logbox-server/src/ui.rs` serves `web/dist` (rust-embed, SPA fallback, CSP). `crates/logbox-api/src/overview.rs` (tiles, heatmap and per-route series from route_minute), `errors.rs` (groups list/detail with new/spiking) and `journeys.rs` (requests + events + spans for one key, collapsed into lanes).
- `e2e/screenshots.mjs`: Playwright screenshots of a running instance (`BASE_URL=… OUT=… node screenshots.mjs`); Chromium is installed on the host.
- `crates/logbox-alerts` (M6): `crypto` (`Keys`: XChaCha20-Poly1305 for stored secrets, HMAC tokens; key from `secret_key` or generated into the Postgres `settings` table), `smtp` (stored + configured settings merged per field, `locked_fields`, lettre send with the full error chain), `channels` (webhook with `X-Logbox-Signature`, Slack, Telegram, email recipients), `outbox` (`notifications` table: queue with lease and backoff, also the history), `rules` (60 s evaluator, SQL per kind, ok/firing with cooldown, Explore links), `throttle` (pure noise-control state), `email5xx` (`Engine` on `LiveBus::subscribe_errors()`, saves `email_groups` every 15 s), `render` (email HTML/text, rule messages), `mute` (signed links). `start()` is called by `serve.rs` for the `all` and `alerts` roles. `crates/logbox-api/src/alerts.rs` has the endpoints; `AppState` carries `secrets` and an `outbox`. Postgres schema: `migrations/postgres/0002_alerts.sql`.
- Not built yet: the Python SDK is a skeleton.

## Query language (M2, done)

`crates/logbox-query`: `registry.rs` (columns for `requests` and `events`, the single source of truth), `parse.rs` (sqlparser ClickHouse dialect → typed AST; `resolve_column` enforces the sensitive and live rules), `ast.rs` (AST plus `Display` back to SQL), `compile.rs` (`compile`, `where_clause`; values are always `{pN:Type}` params), `eval.rs` (`Prepared` evaluator; `Record` is implemented for `LiveRequest` and `RequestRow`), `regex.rs` (RE2 validation and ASCII class rewriting), `search.rs`, `chips.rs`, and `QueryInput` in `lib.rs` (sql/search/filters). Tests: `tests/injection.rs`, `tests/props.rs`, `tests/clickhouse.rs` (differential; widen with `LOGBOX_DIFF_CASES=4000 LOGBOX_DIFF_RANDOM=1`), and `fuzz/` (`cargo +nightly fuzz run parse`; nightly and cargo-fuzz are installed in the dev container).

## Alerts (M6, done)

Decisions 036–043 in docs/DECISIONS.md; user-facing behaviour in docs/alerts.md. Verified by hand against Mailpit: a 500 from ingest is emailed in about 2 s with the spec's subject, no body/header/IP data in the raw message, later occurrences are throttled, mute links confirm on POST and reject tampered tokens, and an error-rate rule went ok → firing → resolved with one email each. The brief's unit and Mailpit integration tests were not written (tests are skipped from M4 on); `throttle.rs` is pure and easy to cover if the user asks for tests later.

## Plan for M7 (SDKs)

Spec ("SDKs" section of docs/agents/spec.md): shared behaviour in `docs/sdk-spec.md` (bounded queue 10k dropping oldest, flush every 1 s or 500 events, gzip, backoff with jitter, `Retry-After`, flush on shutdown; `X-Request-Id` + W3C `traceparent`; async-local context; default redaction list; body capture `errors|always|never`, 8 KB, skip binary/multipart; sampling that keeps errors and slow requests; ignore health routes; < 0.2 ms p99 overhead benchmark).

1. Read `sdks/node/packages/node` (the M1 core) and `docs/ingest-protocol.md`; write `docs/sdk-spec.md` from the core's actual behaviour.
2. `@logbox/express` (middleware), `@logbox/fastify` (plugin), `@logbox/nestjs` (module, interceptor, exception filter with error type/message/stack, `Logger` bridge to `events`, BullMQ helper that carries context in job data and logs runs as `kind=job` with `attempt`). Each with typed config and a README with a 10-line quickstart.
3. Python `logbox` in `sdks/python`: background-thread transport, contextvars, Django middleware, Starlette/FastAPI middleware, Celery signal hooks, `logging` handler. Python isn't set up on this machine yet: `python3` exists; create a venv for `ruff`, `mypy` and building.
4. `examples/`: minimal Express, NestJS, Django and FastAPI apps wired to the SDKs, runnable against the dev server.
5. Benchmarks per SDK (overhead per request), results in `docs/benchmarks.md`. One doc per SDK under `docs/`.

## How to resume

1. `git log --oneline` and this file's status table tell you where things stand.
2. Recreate the dev containers above if they're missing (`docker ps -a`).
3. Run the everyday checks to confirm the tree is green before changing anything.
4. Continue with the first unchecked item in the plan; update this file when a milestone lands.
