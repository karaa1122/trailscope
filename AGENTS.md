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
| M6 Alerts + email-on-5xx | **next** | includes the follow-up email brief |
| M7 SDKs | todo | |
| M8 Harden and ship | todo | |

## Dev environment (this Windows machine)

The host has Node 24 + pnpm 9 + Docker, but **no Rust or Python**. Toolchains run in containers with the repo bind-mounted:

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
- `web/src`: `main.tsx`, `router.tsx` (code-based TanStack routes, typed search params, `Guard`), `lib/` (`api.ts` typed client + CSRF, `types.ts`, `session.tsx` (me, projects, current project/env, theme), `query.ts` (QueryValue ↔ URL, addCondition, history), `live.ts` (SSE hook with pause buffer), `hotkeys.ts`, `format.ts`), `components/` (`Shell` sidebar/rail, `CommandPalette`, `QueryBar`, `SqlEditor` (lazy CodeMirror), `charts.tsx` (Heatmap, Histogram, TimeChart, Waterfall, JsonView), `TimeRange`, `ui.tsx`), `pages/` (`LiveTail`, `Explore`, `Inspector`, `Journeys`, `Errors` (list + group), `Routes`, `Views`, `Settings` (`?tab=`), `Auth` (login, invite, setup wizard), `Placeholder` (still used for Alerts)). Styles: `styles/tokens.css` (dark + light tokens), `styles/app.css`.
- `crates/logbox-server/src/ui.rs` serves `web/dist` (rust-embed, SPA fallback, CSP). `crates/logbox-api/src/overview.rs` (tiles, heatmap and per-route series from route_minute), `errors.rs` (groups list/detail with new/spiking) and `journeys.rs` (requests + events + spans for one key, collapsed into lanes).
- `e2e/screenshots.mjs`: Playwright screenshots of a running instance (`BASE_URL=… OUT=… node screenshots.mjs`); Chromium is installed on the host.
- Not built yet: `logbox-alerts` is an empty stub; the Python SDK is a skeleton.

## Query language (M2, done)

`crates/logbox-query`: `registry.rs` (columns for `requests` and `events`, the single source of truth), `parse.rs` (sqlparser ClickHouse dialect → typed AST; `resolve_column` enforces the sensitive and live rules), `ast.rs` (AST plus `Display` back to SQL), `compile.rs` (`compile`, `where_clause`; values are always `{pN:Type}` params), `eval.rs` (`Prepared` evaluator; `Record` is implemented for `LiveRequest` and `RequestRow`), `regex.rs` (RE2 validation and ASCII class rewriting), `search.rs`, `chips.rs`, and `QueryInput` in `lib.rs` (sql/search/filters). Tests: `tests/injection.rs`, `tests/props.rs`, `tests/clickhouse.rs` (differential; widen with `LOGBOX_DIFF_CASES=4000 LOGBOX_DIFF_RANDOM=1`), and `fuzz/` (`cargo +nightly fuzz run parse`; nightly and cargo-fuzz are installed in the dev container).

## Plan for M6 (alerts and email on 5xx)

Spec: "A rule is a query, a condition and a window: `count > N`, `error_rate > X%`, `p95(duration_ms) > X ms` on a filter, or `new error group` / `spiking error group`. Rules are evaluated every 60 s. States ok/firing/resolved with dedupe and a cooldown, so each state change sends one notification. Channels: webhook (HMAC-SHA256), Slack incoming webhook, Telegram bot, email over SMTP. Messages link back to the UI with the query pre-filled." Plus the whole follow-up brief (next section and docs/agents/spec-email-notifications.md).

1. Postgres migration `0002_alerts.sql`: `alert_rules` (project, name, table, query JSON, kind, threshold, window, cooldown, channels, enabled, state, last_eval), `alert_channels` (kind webhook/slack/telegram/email, config JSON with secrets encrypted), `alert_events` (history), `smtp_settings` (one row; password encrypted), `email_rules` (built-in "Email on 5xx" per project: filter, recipients), `email_outbox` (queue: attempts, next_attempt_at, status, error), `error_mutes` (fingerprint, until).
2. Config: `secret_key` (`LOGBOX_SECRET_KEY`), `smtp.from_name`; crypto with XChaCha20-Poly1305 (check the current `chacha20poly1305` version) and HMAC-SHA256 mute links.
3. `crates/logbox-alerts`: evaluator task (60 s tick, compiles rule queries with `logbox_query::compile`, runs as the reader), state machine with cooldown, channel senders (reqwest; lettre with tokio + rustls for SMTP), the 5xx email pipeline subscribed to `LiveBus::subscribe_errors()` with throttling/digest/cap/regression, and the outbox worker. Started by `serve.rs` for the `all` and `alerts` roles.
4. API: rules/channels CRUD, test-send, SMTP settings (env-locked fields, password never returned), history, mute link endpoint (no login).
5. UI: `/alerts` page (rules list, editor using `QueryBar`, channels, Email on 5xx card, SMTP settings, history).
6. Mailpit in a `dev` compose profile; docs/alerts.md and docs/configuration.md.
7. The dev UI instance: container `logbox-ui` publishes 8090 (owner@example.com / "correct horse battery"); restart it after rebuilding the binary. Demo traffic runs separately in `logbox-rust` (`/target/debug/logbox demo --url http://logbox-ui:8080 --key <ingest key> --backfill-hours 0`); start it again after container restarts. Without `--key` the demo creates a new project.

## Email-on-5xx (follow-up brief) integration points

- **Config:** `smtp.*` exists in `Config`. Still needed: `smtp.from_name`, a top-level `secret_key` (env `LOGBOX_SECRET_KEY`) used for AES-256-GCM or XChaCha20-Poly1305 encryption of the stored SMTP password and for HMAC-signed mute links. Env values override UI values and lock those fields in the UI.
- **Ingest → alert engine:** done in M1. `LiveBus::subscribe_errors()` yields `ErrorSignal`s (5xx, without bodies, headers or IPs) after each successful flush; in split mode they arrive over Redis `<prefix>:errors`.
- **Alerts crate:** throttle per `error_fingerprint` (first occurrence immediately, then at most one per 15 min with digest counts), a project cap of 30/hour with a single "paused" email, regression after 24 h quiet, signed mute links (1 h / 24 h / forever), and a Postgres-backed send queue with 5 attempts and exponential backoff. Every send is logged and shown under Alerts → History.
- **Dev:** Mailpit in a `dev` compose profile; an integration test asserts the email arrives with the right subject and no body or header data.
- **Docs:** `docs/alerts.md` and `docs/configuration.md`.

## How to resume

1. `git log --oneline` and this file's status table tell you where things stand.
2. Recreate the dev containers above if they're missing (`docker ps -a`).
3. Run the everyday checks to confirm the tree is green before changing anything.
4. Continue with the first unchecked item in the plan; update this file when a milestone lands.
