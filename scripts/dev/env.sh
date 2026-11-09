#!/usr/bin/env bash
# Environment for running `trailscope` inside the trailscope-rust dev container
# against the dev ClickHouse/Postgres. Source it inside the container:
#   docker exec -it trailscope-rust bash -c 'source scripts/dev/env.sh && /target/debug/trailscope all'
export TRAILSCOPE_CLICKHOUSE_URL=http://trailscope-dev-ch:8123
export TRAILSCOPE_CLICKHOUSE_USER=trailscope
export TRAILSCOPE_CLICKHOUSE_PASSWORD=trailscope
export TRAILSCOPE_CLICKHOUSE_DATABASE=trailscope
export TRAILSCOPE_POSTGRES_URL=postgres://trailscope:trailscope@trailscope-dev-pg:5432/trailscope
export TRAILSCOPE_SERVER_LISTEN=0.0.0.0:8080
