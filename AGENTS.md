# AGENTS.md: notes for coding agents and contributors

Read this before changing Trailscope. It covers what the project is, its conventions, how to run it locally and where things live. Keep it current when the architecture changes.

## What Trailscope is

An open-source, self-hosted API request logging tool: a Rust server with ClickHouse and Postgres, a React UI embedded in the binary, and Node and Python SDKs. It must stay domain-neutral: no industry-specific wording in code, demo data, docs or UI copy.

- Original brief: [docs/agents/spec.md](docs/agents/spec.md). It defines the milestones M0–M8, the data model, the query language, the UI and the performance targets.
- Follow-up brief (SMTP email on 5xx): [docs/agents/spec-email-notifications.md](docs/agents/spec-email-notifications.md).
- Plan for the hosted, multi-tenant Trailscope Cloud (milestones C1–C7, not started): [docs/agents/spec-cloud.md](docs/agents/spec-cloud.md).
- Visual reference for the UI: [design/](design/) (HTML mockups; match them).
- Every judgement call goes in [docs/DECISIONS.md](docs/DECISIONS.md) (context, choice, alternatives).

## Conventions

- Record each judgement call in `docs/DECISIONS.md`, newest at the bottom.
- Quality bar: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, strict TypeScript, no `any` without a comment, no `unwrap()`/`expect()` in non-test Rust code except at startup (enforced by workspace lints; `clippy.toml` allows them in tests).
- Check the current stable version before pinning any crate, npm/PyPI package, Docker image or GitHub Action.
- Conventional commit messages (`feat:`, `fix:`, `docs:`, `chore:` …).
- Test coverage is uneven: ingest and the query language are covered by unit, property, differential and fuzz tests; the alerts, the SDKs and single sign-on mostly aren't yet (see "Open items").

## Status

| Milestone | State | Notes |
|---|---|---|
| M0 Scaffold | done | workspace, binary + config, compose, CI skeleton |
| M1 Ingest | done | 50k ev/s sustained at p99 17 ms on 4 CPUs (docs/benchmarks.md); Node core SDK; demo and load test |
| M2 Query language | done | parser, registry, compiler, evaluator, search, chips; injection, property, differential (vs ClickHouse) and fuzz tests; docs/query-language.md |
| M3 API | done | auth (sessions+CSRF, tokens, roles, invites, rate limit), setup, projects/keys/rules/targets, query endpoints, inspector/cURL/replay, live SSE, views, OpenAPI at /api/docs; docs/api.md |
| M4 Web part 1 | done | shell, palette, setup/login/invite, query bar (3 modes, quick filters, CodeMirror), Live tail, Explore, Inspector; UI embedded via rust-embed |
| M5 Web part 2 | done | Journeys, Errors list/detail, Routes (full heatmap + per-route series), release markers on every chart, Saved views, Settings |
| M6 Alerts + email-on-5xx | done | rules (count, 5xx rate, p95, new/spiking error group) with cooldown; webhook/Slack/Telegram/email channels; real-time Email on 5xx with throttle, digest, cap, regression and signed mute links; Postgres outbox = History; SMTP settings with env locking; encrypted secrets; docs/alerts.md. Verified by hand against Mailpit |
| M7 SDKs | done | `@trailscope/express`, `@trailscope/fastify`, `@trailscope/nestjs`, Python `trailscope` (Django, ASGI, Celery, logging handler); `examples/`; overhead benchmarks (all < 0.05 ms p99 added); docs/sdk-*.md |
| M8 Harden and ship | done | OTLP/HTTP receiver, OIDC sign-in, performance verified on 102M requests, README and docs, release workflow (GHCR, npm via changesets, PyPI), Dependabot |

Since M8: SDKs for Java (decision 063) and PHP/Laravel (decision 064); Explore also reads jobs and log events (decision 058); replay looks only at whether the request body was cut (decision 059); empty `TRAILSCOPE_*` variables count as unset, so the compose files pass the SMTP and OIDC settings through (decision 060). The project was renamed from Logbox, its working name, to Trailscope (decision 062). The website (landing page, planned Cloud pricing with a waitlist, and the docs built from `docs/`) lives in `website/` and is served from trailscope.pro (decision 065).

## Running it locally

Needs Rust (the version in `rust-toolchain.toml`), Node 24 with pnpm 9, Python 3.10+ and Docker.

```bash
docker compose -f deploy/docker-compose.dev.yml up -d                        # ClickHouse :18123, Postgres :25432, Redis :26379, Mailpit :8025
source scripts/dev/local.env && cargo run -- all                            # Trailscope from source; UI at http://localhost:8090
docker compose -f deploy/docker-compose.dev.yml --profile app up -d --build  # or Trailscope in a container too (stop the one from source first: same port)
cargo run -- demo --url http://localhost:8090                               # demo traffic with a scripted incident (creates a "Demo" project)
```

In development, configure email under Settings → Notifications with host `mailpit` (Trailscope in a container) or `127.0.0.1` (from source), port 1025, security none; mail shows at http://localhost:8025.

Python SDK: `python -m venv sdks/python/.venv && sdks/python/.venv/bin/pip install -e "sdks/python[dev]"`. Checks: `cd sdks/python && .venv/bin/ruff check . && .venv/bin/ruff format --check . && .venv/bin/mypy`.

Everyday checks: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`, then `pnpm install && pnpm build && pnpm lint && pnpm typecheck`. Without Rust or Python on the host, `scripts/dev/rust.sh` runs commands in a Rust container.

Things that bite:

- The ingest wire format is camelCase (`requestId`, `durationMs`, `errorType`…); snake_case fields are silently ignored.
- In ClickHouse a `SELECT` alias is visible in `WHERE`: `SELECT * REPLACE (… AS timestamp) … WHERE timestamp …` filters on the alias (decision 026). Filter in a subquery.
- Several crates have new majors whose APIs differ from older docs: clickhouse 0.15 (`client.insert::<T>(t).await?`, params via `.param()`), sqlx 0.9, reqwest 0.13, rand 0.10, sha2 0.11, utoipa 6, tower-http 0.7, sqlparser 0.63. Read the crate source under `~/.cargo/registry/src/*/<crate>-<ver>/` when unsure.
- Ingest keys are shown once; to send traffic to an existing project, create a key under Settings → Ingest keys.

## Pinned versions

Rust 1.99.1 (`rust-toolchain.toml`), ClickHouse 26.8 LTS, Postgres 18, Redis 8, Node 24, pnpm 9.15.9, TypeScript 6.0.3 (TS 7 isn't supported by typescript-eslint yet), Vite 8, React 19.3, Python 3.10+. Workspace crate versions are in the root `Cargo.toml` `[workspace.dependencies]`.

## Architecture

- `crates/trailscope-core`: `config` (defaults → `trailscope.toml` → `TRAILSCOPE_<SECTION>_<KEY>` env, empty values ignored), `wire` (ingest JSON types), `rows` (ClickHouse row structs, `LiveRequest`, `ErrorSignal`), `ch` (writer and reader clients, migration runner, reader user and settings profile, TTL sync), `pg` (pool and sqlx migrations), `live` (`LiveBus`: broadcast, or Redis pub/sub in split mode, with separate `requests` and `errors` topics), `secrets` (token generation and SHA-256), `admin` (create project, create ingest key), `project` (`ProjectSettings` JSONB), `testing` (per-test databases from `TRAILSCOPE_TEST_*`).
- `crates/trailscope-ingest`: `/v1/ingest`, `/v1/releases`, `/v1/config`, OTLP (`otlp.rs`). Modules: `keys` (key cache), `decode` (gzip/zstd with a cap, array/NDJSON split), `normalize` (validation, route templating, truncation), `redact`, `fingerprint`, `pipeline` (queue bounded by rows and bytes; writer with 4 concurrent inserts; publishes live and 5xx signals after each flush).
- `crates/trailscope-demo`: a deterministic generic SaaS traffic generator (`generator.rs`), backfill, live traffic and a scripted incident (release v1.43.0 breaks `GET /projects/:id/tasks`, v1.43.1 fixes it), and `--load` with `--target-eps` for open-loop latency tests.
- `crates/trailscope-server`: the `trailscope` binary. `serve.rs` wires the roles; `ops.rs` serves `/healthz`, `/readyz` and `/metrics`; `ui.rs` serves `web/dist` (rust-embed, SPA fallback, CSP). Uses mimalloc. Shutdown order: stop accepting, then drain the writer.
- `migrations/clickhouse/NNNN_*.sql` (embedded with `include_str!` in `ch.rs`; add new files to its `MIGRATIONS` list) and `migrations/postgres/` (sqlx).
- `crates/trailscope-api`: `AppState` (config, ClickHouse, Postgres, LiveBus, optional ingest KeyCache, login limiter, replay HTTP client, secrets, outbox). Modules: `auth` (session cookie + CSRF or Bearer API token; `require(Role, Scope)`, `can_view_bodies`), `setup`, `users`, `projects`, `query` (`prepare` → `Prepared`; `fetch_json` runs as the reader), `inspector` (incl. cURL and replay), `live`, `views`, `overview`, `errors`, `journeys`, `alerts`, `oidc`, `openapi`. Integration test: `tests/api.rs`.
- `crates/trailscope-alerts`: `crypto` (XChaCha20-Poly1305 for stored secrets, HMAC tokens), `smtp` (stored + configured settings merged per field, `locked_fields`), `channels` (webhook with `X-Trailscope-Signature`, Slack, Telegram, email), `outbox` (`notifications` table: queue with lease and backoff, also the history), `rules` (60 s evaluator, ok/firing with cooldown), `throttle` (pure noise-control state), `email5xx` (`Engine` on `LiveBus::subscribe_errors()`), `render`, `mute` (signed links). Started by `serve.rs` for the `all` and `alerts` roles.
- `crates/trailscope-query`: `registry.rs` (columns for `requests` and `events`, the single source of truth), `parse.rs` (sqlparser ClickHouse dialect → typed AST; `resolve_column` enforces the sensitive and live rules), `ast.rs`, `compile.rs` (values are always `{pN:Type}` params), `eval.rs` (in-memory evaluator for live filters), `regex.rs`, `search.rs`, `chips.rs`. Tests: `tests/injection.rs`, `tests/props.rs`, `tests/clickhouse.rs` (differential; widen with `TRAILSCOPE_DIFF_CASES=4000 TRAILSCOPE_DIFF_RANDOM=1`), and `fuzz/` (`cargo +nightly fuzz run parse`).
- `sdks/node/packages/node`: `@trailscope/node` core (queue, transport, redaction, context, sampling, `trackJob`, remote config from `/v1/config`, `recordHttp`/`observeHttp` for integrations, benchmark in `bench/`). `sdks/node/packages/{express,fastify,nestjs}`: thin integrations (NestJS: module, interceptor, opt-in exception filter, Logger bridge, BullMQ helpers).
- `sdks/php` (decision 064): one Composer package `trailscope/trailscope`: core `src/` (`Client`, `Trailscope` statics, `Context`, `Redactor`, `BodyCapture`, `StateFile` shared between PHP-FPM processes, `Transport/` curl with a streams fallback, `Monolog/TrailscopeHandler`) and `src/Laravel/` (`TrailscopeServiceProvider`, `TrailscopeMiddleware`, `QueueIntegration`). No PHP on the host needed: `sdks/php/scripts/check.sh` runs `php -l` (8.2 and 8.5), Pint and PHPStan level 10 in Docker. Published through the mirror in `.github/workflows/php-split.yml`.
- `sdks/java` (decision 063): Maven modules `trailscope-java` (core, no dependencies: `TrailscopeClient`, `Trailscope` statics, `TrailscopeContext`, hand-written `Json`, `Redactor`), `trailscope-servlet` (`TrailscopeFilter`, bounded request/response capture), `trailscope-spring-boot-starter` (auto-configuration, `TrailscopeProperties`, exception resolver, user interceptor, `TaskDecorator`) and `trailscope-logback` (`TrailscopeAppender`); `bench/` is never published. Build with `mvn -B verify` (JDK 17+; compiler warnings are errors).
- `sdks/python/src/trailscope`: `client.py` (Client, transport thread, `record_http`, `track_job`, `span`, remote config, fork handling), `django.py`, `asgi.py`, `celery.py`, `handler.py` (`TrailscopeHandler`). Benchmark: `sdks/python/bench/overhead.py`.
- `examples/`: Express, Fastify, NestJS (+BullMQ), FastAPI, Django (+Celery), Spring Boot, Laravel (database queue on SQLite). See examples/README.md.
- `web/src`: `router.tsx` (code-based TanStack routes, typed search params), `lib/` (`api.ts` typed client + CSRF, `types.ts`, `session.tsx`, `query.ts`, `live.ts`, `hotkeys.ts`, `format.ts`), `components/` (`Shell`, `CommandPalette`, `QueryBar`, `SqlEditor`, `charts.tsx`, `TimeRange`, `ui.tsx`), `pages/` (`LiveTail`, `Explore`, `Inspector`, `Journeys`, `Errors`, `Routes`, `Views`, `Settings`, `SettingsNotifications`, `Alerts`, `Auth`). Styles: `styles/tokens.css` (dark + light tokens), `styles/app.css`.
- `e2e/screenshots.mjs`: Playwright screenshots of a running instance (`BASE_URL=… OUT=… node screenshots.mjs`).
- `website/` (decision 065): the public site, Astro + Starlight. `src/pages/` (landing, pricing, privacy, thank-you pages), `src/components/` (`Waitlist`, `CodeTabs`, Starlight overrides `DocsNav` and `DocsFooter` with the Mermaid loader), `scripts/sync-docs.mjs` (turns `docs/*.md` into `src/content/docs/docs/`, generated and git-ignored: edit `docs/`, never the copies), `domain.txt` (the only place the domain is set), `waitlist/waitlist.py` (stdlib POST `/api/waitlist` → SQLite) and `deploy/` (`nginx.conf`, the systemd unit, `setup-server.sh`, `deploy.sh`). `pnpm --filter trailscope-website dev` serves it locally; screenshots come from `docs/images`.

Lessons from building the SDKs: Starlette runs the last-added middleware first (add `TrailscopeMiddleware` last); BullMQ 6 needs `ioredis` installed by the app; Nest's `ExceptionsHandler` log line would duplicate error groups, so the Logger bridge ignores it; Nest on Fastify needs the `preHandler` hook for guards to see the context.

## Open items

Roughly in priority order:

1. **First release:** own the npm scope (or rename the Node packages), add the `NPM_TOKEN` secret, configure PyPI trusted publishing for `trailscope` (environment `pypi`), then push a `v0.1.0` tag.
2. **Missing tests** (decisions 028, 052, 055): Email on 5xx throttle unit tests and the Mailpit integration test, SDK test suites, a Playwright e2e suite against the compose stack.
3. **Journeys on low-cardinality keys** read every matching row (about 0.7 s over 7 days for a key with a few hundred values on 100M requests); a per-key rollup or a capped, time-ordered read would help.
4. The ingest benchmark numbers come from an older development machine; rerun `trailscope demo --load` to refresh them.
5. Journey header: when the 2,000-item cap is hit, "Quiet since" refers to the last loaded item, not the real last activity.
6. **Website:** each new waitlist signup is emailed to hello@ through Resend when `RESEND_API_KEY` is in `/etc/trailscope-waitlist.env` on the server (root-only); all signups: `sudo python3 /opt/trailscope-waitlist/waitlist.py export --db /var/lib/trailscope-waitlist/waitlist.db`.

## Getting oriented

1. `git log --oneline` and the Status table tell you where things stand.
2. Start the dev services (above) and run the everyday checks to confirm the tree is green before changing anything.
3. Make the change, record any judgement call in `docs/DECISIONS.md`, and update this file when the architecture changes.
