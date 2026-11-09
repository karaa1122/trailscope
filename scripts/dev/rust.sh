#!/usr/bin/env bash
# Runs a command in the `trailscope-rust` dev container with the integration-test
# environment set. Example: scripts/dev/rust.sh cargo test --workspace
set -euo pipefail
export MSYS_NO_PATHCONV=1
exec docker exec \
  -e TRAILSCOPE_TEST_CLICKHOUSE_URL=http://trailscope-dev-ch:8123 \
  -e TRAILSCOPE_TEST_CLICKHOUSE_USER=trailscope \
  -e TRAILSCOPE_TEST_CLICKHOUSE_PASSWORD=trailscope \
  -e TRAILSCOPE_TEST_POSTGRES_URL=postgres://trailscope:trailscope@trailscope-dev-pg:5432/trailscope \
  -e TRAILSCOPE_TEST_REDIS_URL=redis://trailscope-dev-redis:6379 \
  -e TRAILSCOPE_TEST_MAILPIT_URL=http://trailscope-dev-mailpit:8025 \
  -e TRAILSCOPE_TEST_SMTP_HOST=trailscope-dev-mailpit \
  -e CARGO_TERM_COLOR=never \
  -w /work trailscope-rust "$@"
