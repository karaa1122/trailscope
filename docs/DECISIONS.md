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
- **Choice:** `https://github.com/karaa1122/logbox`. CI publishes to `ghcr.io/${{ github.repository }}`, so forks work without edits. Compose builds `logbox:local` from source by default; `LOGBOX_IMAGE` selects a published image.
- **Alternatives:** A placeholder organisation.

## 006. Distroless runtime image and built-in healthcheck

- **Context:** The final image should be slim and non-root, and compose needs a healthcheck.
- **Choice:** `gcr.io/distroless/cc-debian13:nonroot`. It has no shell or curl, so the binary has a hidden `logbox healthcheck` subcommand that probes `/healthz` on the local listener.
- **Alternatives:** `debian:trixie-slim` with curl (larger image, more CVE surface).

## 007. Split-mode ports

- **Context:** In split mode ingest and api are separate processes.
- **Choice:** The api (with the UI) is published on 8080 and ingest on 8081. A one-shot `migrate` service runs first, and the other services wait for it to complete.
- **Alternatives:** A reverse proxy in front of both (one more moving part for small installs).
