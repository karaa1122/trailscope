#!/usr/bin/env bash
# Runs a command in the `logbox-rust` dev container with the integration-test
# environment set. Example: scripts/dev/rust.sh cargo test --workspace
set -euo pipefail
export MSYS_NO_PATHCONV=1
exec docker exec \
  -e LOGBOX_TEST_CLICKHOUSE_URL=http://logbox-dev-ch:8123 \
  -e LOGBOX_TEST_CLICKHOUSE_USER=logbox \
  -e LOGBOX_TEST_CLICKHOUSE_PASSWORD=logbox \
  -e LOGBOX_TEST_POSTGRES_URL=postgres://logbox:logbox@logbox-dev-pg:5432/logbox \
  -e LOGBOX_TEST_REDIS_URL=redis://logbox-dev-redis:6379 \
  -e LOGBOX_TEST_MAILPIT_URL=http://logbox-dev-mailpit:8025 \
  -e LOGBOX_TEST_SMTP_HOST=logbox-dev-mailpit \
  -e CARGO_TERM_COLOR=never \
  -w /work logbox-rust "$@"
