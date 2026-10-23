#!/usr/bin/env bash
# Environment for running `logbox` inside the logbox-rust dev container
# against the dev ClickHouse/Postgres. Source it inside the container:
#   docker exec -it logbox-rust bash -c 'source scripts/dev/env.sh && /target/debug/logbox all'
export LOGBOX_CLICKHOUSE_URL=http://logbox-dev-ch:8123
export LOGBOX_CLICKHOUSE_USER=logbox
export LOGBOX_CLICKHOUSE_PASSWORD=logbox
export LOGBOX_CLICKHOUSE_DATABASE=logbox
export LOGBOX_POSTGRES_URL=postgres://logbox:logbox@logbox-dev-pg:5432/logbox
export LOGBOX_SERVER_LISTEN=0.0.0.0:8080
