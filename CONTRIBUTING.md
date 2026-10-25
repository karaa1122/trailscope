# Contributing to Logbox

Thanks for helping. Bug reports, fixes, docs and new SDK integrations are all welcome. For larger changes, open an issue first so we can agree on the approach.

## Development setup

You need Docker, Rust (the version in `rust-toolchain.toml` is picked up by rustup), Node 20+ with pnpm 9, and Python 3.10+ for the Python SDK.

```bash
# ClickHouse, Postgres, Mailpit and Redis on offset ports
docker compose -f deploy/docker-compose.dev.yml up -d

# Build the UI once, then run Logbox from source (UI on http://localhost:8090)
pnpm install && pnpm build
source scripts/dev/local.env && cargo run -- all

# Optional: demo traffic with a scripted incident (creates a "Demo" project)
cargo run -- demo --url http://localhost:8090
```

Open http://localhost:8090 and the setup wizard creates the owner account. For UI work, `pnpm --filter @logbox/web dev` runs Vite with hot reload and proxies the API to port 8090.

## Checks

CI runs these on every pull request; please run them before pushing:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # integration tests use the dev databases via scripts/dev/local.env
pnpm lint && pnpm typecheck && pnpm test && pnpm build
cd sdks/python && ruff check . && ruff format --check . && mypy && pytest -q
```

## Conventions

- **Commits** follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `refactor:`, `chore:`… with an imperative summary.
- **Rust:** no `unwrap()`/`expect()` outside tests and startup code (enforced by the workspace lints); errors are typed with `thiserror` in libraries.
- **TypeScript:** strict mode; no `any` without a comment saying why.
- **Wording:** Logbox is domain-neutral. Examples use generic SaaS terms (projects, tasks, orders, accounts), never industry-specific ones.
- **Decisions:** a choice that others will wonder about goes in `docs/DECISIONS.md` (context, choice, alternatives).
- **SDK changes:** add a changeset (`pnpm changeset`) for the Node packages; keep `docs/sdk-spec.md` true for every SDK.
- **Dependencies:** pin exact versions and check the current stable release before adding one.

## Where things are

| Path | What |
|---|---|
| `crates/logbox-server` | the `logbox` binary (roles, config, migrations, embedded UI) |
| `crates/logbox-ingest` | `/v1/ingest`, OTLP, validation, redaction, the write pipeline |
| `crates/logbox-query` | the query language: parser, compiler, evaluator |
| `crates/logbox-api` | the HTTP API used by the UI |
| `crates/logbox-alerts` | alert rules, Email on 5xx, notification delivery |
| `web/` | the React UI |
| `sdks/node`, `sdks/python` | the SDKs |
| `examples/` | small apps wired to the SDKs |
| `docs/` | user and operator documentation |

See [docs/architecture.md](docs/architecture.md) for how the pieces fit together.

## Reporting security issues

Please don't open public issues for vulnerabilities; see [SECURITY.md](SECURITY.md).
