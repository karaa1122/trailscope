# syntax=docker/dockerfile:1.7
# Multi-stage build for the `trailscope` binary with the web UI embedded.
# Multi-arch: docker buildx build --platform linux/amd64,linux/arm64 .

ARG RUST_IMAGE=lukemathwalker/cargo-chef:latest-rust-1.99-trixie
ARG NODE_IMAGE=node:24-trixie-slim
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot

# ---- web UI -------------------------------------------------------------
FROM ${NODE_IMAGE} AS web
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json eslint.config.mjs ./
COPY web web
COPY sdks/node sdks/node
RUN --mount=type=cache,id=pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter @trailscope/web...
RUN pnpm --filter @trailscope/web build

# ---- rust: dependency recipe ---------------------------------------------
FROM ${RUST_IMAGE} AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- rust: build -----------------------------------------------------------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json --bin trailscope
COPY . .
COPY --from=web /src/web/dist web/dist
RUN cargo build --release --locked --bin trailscope \
    && cp target/release/trailscope /usr/local/bin/trailscope

# ---- runtime ---------------------------------------------------------------
FROM ${RUNTIME_IMAGE}
LABEL org.opencontainers.image.title="trailscope" \
      org.opencontainers.image.description="Self-hosted API request logging" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/karaa1122/trailscope"
COPY --from=builder /usr/local/bin/trailscope /usr/local/bin/trailscope
USER nonroot:nonroot
ENV TRAILSCOPE_SERVER_LISTEN=0.0.0.0:8080 \
    TRAILSCOPE_LOG_FORMAT=json
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --retries=5 CMD ["/usr/local/bin/trailscope", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/trailscope"]
CMD ["all"]
