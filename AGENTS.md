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
| M1 Ingest | done | 50k ev/s sustained at p99 17 ms on 4 CPUs (docs/benchmarks.md); Node core SDK; demo and load test |
| M2 Query language | **next** | |
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

- `crates/logbox-core`: `config` (defaults → `logbox.toml` → `LOGBOX_<SECTION>_<KEY>` env), `wire` (ingest JSON types), `rows` (ClickHouse row structs, `LiveRequest`, `ErrorSignal`), `ch` (writer and reader clients, migration runner, reader user and settings profile, TTL sync), `pg` (pool and sqlx migrations), `live` (`LiveBus`: broadcast, or Redis pub/sub in split mode, with separate `requests` and `errors` topics), `secrets` (token generation and SHA-256), `admin` (create project, create ingest key), `project` (`ProjectSettings` JSONB), `testing` (per-test databases from `LOGBOX_TEST_*`).
- `crates/logbox-ingest`: `/v1/ingest`, `/v1/releases`, `/v1/config`. Modules: `keys` (key cache), `decode` (gzip/zstd with a cap, array/NDJSON split), `normalize` (validation, route templating, truncation), `redact`, `fingerprint`, `pipeline` (queue bounded by rows and bytes; writer with 4 concurrent inserts; publishes live and 5xx signals after each flush).
- `crates/logbox-demo`: a deterministic generic SaaS traffic generator (`generator.rs`), backfill, live traffic and a scripted incident (release v1.43.0 breaks `GET /projects/:id/tasks`, v1.43.1 fixes it), and `--load` with `--target-eps` for open-loop latency tests.
- `crates/logbox-server`: the `logbox` binary. `serve.rs` wires the roles; `ops.rs` serves `/healthz`, `/readyz` and `/metrics`. Uses mimalloc. Shutdown order: stop accepting, then drain the writer.
- `migrations/clickhouse/0001_init.sql` (embedded with `include_str!` in `ch.rs`; add new files to its `MIGRATIONS` list) and `migrations/postgres/0001_init.sql` (sqlx).
- `sdks/node/packages/node`: `@logbox/node` core (queue, transport, redaction, context, sampling, `trackJob`, benchmark in `bench/`).
- `scripts/dev/rust.sh` runs a command in the Rust container with the test env vars; `scripts/dev/env.sh` holds the env for running `logbox` against the dev services.
- Not built yet: `logbox-query`, `logbox-api` and `logbox-alerts` are empty stubs, `web/` is a placeholder, and the Python SDK is a skeleton.

## Plan for M2 (query language)

All in `crates/logbox-query`, with no database dependency:

1. `registry.rs`: the column registry (name, ClickHouse type, logical type, description, filterable/facetable/groupable, table) for `requests` and `events`. The single source of truth, also served at `GET /api/v1/schema`.
2. `ast.rs`: Logbox's own small typed AST, with limits of 256 nodes and depth 32.
3. `parse.rs`: normalize (optional leading `WHERE`, trailing `;`, reject comments and trailing statements, 4,000-character cap), parse with `sqlparser` `Parser::parse_expr` using the ClickHouse dialect, then convert the allowed subset to our AST. Errors carry line, column and length.
4. Type-check against the registry. `compile.rs` turns the AST into ClickHouse SQL with every literal as a `{pN:Type}` parameter, wrapped in `project_id = {pid} AND timestamp BETWEEN {from} AND {to} AND (...)`.
5. `eval.rs`: an in-memory evaluator over a `Record` trait (implemented for `LiveRequest`), for the live tail and real-time 5xx rules.
6. `search.rs` (`status:>=500 route:/checkout/* "free text"`) and `chips.rs` (AST ↔ a `{field, op, value}` list).
7. Tests: unit tests for every operator and error, proptest round-trips, a `fuzz/` cargo-fuzz target, and the injection suite from the spec. Then `docs/query-language.md`.

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
