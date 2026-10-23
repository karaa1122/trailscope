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
- Quality bar: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, strict TypeScript, no `any` without a comment, no `unwrap()`/`expect()` in non-test Rust code except at startup (enforced by workspace lints; `clippy.toml` allows them in tests).
- Check the current stable version before pinning any crate, npm/PyPI package, Docker image or GitHub Action.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Status

| Milestone | State | Notes |
|---|---|---|
| M0 Scaffold | done (`7c6042c`) | workspace, binary + config, compose, CI skeleton |
| M1 Ingest | **in progress** | ClickHouse schema written (`migrations/clickhouse/0001_init.sql`); next: Postgres schema, migration runner, ingest pipeline, Node core SDK, demo |
| M2 Query language | todo | |
| M3 API | todo | |
| M4 Web part 1 | todo | |
| M5 Web part 2 | todo | |
| M6 Alerts + email-on-5xx | todo | includes the follow-up email brief |
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

- Port 8080 was held by a SigNoz stack from another project; the user asked for it to be stopped (`docker compose -p signoz stop`, containers and volumes kept). Another local stack (`eventy`: ports 8000, 15432, 6380, 27017) is running and must be left alone.
- Bash heredocs containing an apostrophe inside `<<'EOF'` blocks have broken the tool's quoting before; use the Write tool for files with prose.
- Several crates have new majors whose APIs differ from older docs: clickhouse 0.15 (`client.insert::<T>(t).await?`, params via `.param()`), sqlx 0.9, reqwest 0.13, rand 0.10, sha2 0.11, utoipa 6, tower-http 0.7, sqlparser 0.63. Read the source under `/usr/local/cargo/registry/src/*/<crate>-<ver>/` when unsure.

## Pinned versions

Rust 1.99.1 (`rust-toolchain.toml`), ClickHouse 26.8 LTS, Postgres 18, Redis 8, Node 24, pnpm 9.15.9, TypeScript 6.0.3 (TS 7 isn't supported by typescript-eslint yet), Vite 8, React 19.3, Python 3.10+ (tested on 3.13). Workspace crate versions are in the root `Cargo.toml` `[workspace.dependencies]`.

## Architecture as built so far

- `crates/logbox-core`: `Config` (defaults → `logbox.toml` → `LOGBOX_<SECTION>_<KEY>` env), errors, tracing init. Planned additions: event schema (wire + row types), ClickHouse/Postgres helpers and migration runner, live bus (broadcast or Redis), key generation and hashing.
- `crates/logbox-server`: the `logbox` binary (clap): `all` (default), `ingest`, `api`, `alerts`, `migrate`, `demo`, and a hidden `healthcheck` used by the distroless image.
- `migrations/clickhouse/0001_init.sql`: `requests`, `spans`, `events`, `route_minute` (+MV), `error_groups` (+MVs from requests and events), `error_minute` (+MVs). Placeholders `{{db}}`, `{{days}}`, `{{body_days}}` are substituted by the runner.
- `deploy/`: all-in-one and split compose files, ClickHouse low-memory config.
- `web/`, `sdks/node/packages/node`, `sdks/python`: skeletons only.

## Plan for the rest of M1

1. `migrations/postgres/0001_init.sql`: users, sessions, invites, projects, environments, ingest_keys, api_tokens, redaction_rules, releases, saved_views, replay targets and results, notification channels, alert rules and events, settings.
2. ClickHouse migration runner in core: creates the database and a `schema_migrations` table, applies embedded files in order, creates the `logbox_reader` user and a settings profile (readonly, execution and memory limits), and re-applies TTLs when the config changes.
3. `logbox-ingest`: `POST /v1/ingest` (Bearer `lbx_ingest_…`, SHA-256 lookup with an in-memory cache), gzip/zstd with a decompressed-size cap, NDJSON or JSON array, per-item validation with a 207-style response, server-side redaction, fingerprinting, body truncation, and a row-counted bounded queue (429 + `Retry-After` when full). The writer flushes every 1 s or 20k rows with retries. After a successful flush it publishes live summaries and, on a **separate error channel**, 5xx summaries for the real-time email engine (M6). Graceful shutdown drains the queue. `POST /v1/releases`. `/metrics`.
4. `@logbox/node` core client: bounded queue that drops the oldest, batch gzip transport with backoff, jitter and `Retry-After`, AsyncLocalStorage context, redaction, sampling, body capture policy.
5. `logbox demo` (backfill + live + scripted incident) and `--load` (events/s and latency percentiles).

## Email-on-5xx (follow-up brief) integration points

- **Config:** `smtp.*` exists in `Config`. Still needed: `smtp.from_name`, a top-level `secret_key` (env `LOGBOX_SECRET_KEY`) used for AES-256-GCM or XChaCha20-Poly1305 encryption of the stored SMTP password and for HMAC-signed mute links. Env values override UI values and lock those fields in the UI.
- **Ingest → alert engine:** after each successful flush, publish 5xx summaries (no bodies, headers or IPs) on an error channel: in-process in `all` mode, a separate Redis channel in split mode.
- **Alerts crate:** throttle per `error_fingerprint` (first occurrence immediately, then at most one per 15 min with digest counts), a project cap of 30/hour with a single "paused" email, regression after 24 h quiet, signed mute links (1 h / 24 h / forever), and a Postgres-backed send queue with 5 attempts and exponential backoff. Every send is logged and shown under Alerts → History.
- **Dev:** Mailpit in a `dev` compose profile; an integration test asserts the email arrives with the right subject and no body or header data.
- **Docs:** `docs/alerts.md` and `docs/configuration.md`.

## How to resume

1. `git log --oneline` and this file's status table tell you where things stand.
2. Recreate the dev containers above if they're missing (`docker ps -a`).
3. Run the everyday checks to confirm the tree is green before changing anything.
4. Continue with the first unchecked item in the plan; update this file when a milestone lands.
