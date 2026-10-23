-- Logbox ClickHouse schema.
-- Placeholders are substituted by `logbox migrate`:
--   {{db}}           database name (clickhouse.database)
--   {{days}}         raw row retention in days (retention.days)
--   {{body_days}}    body and header retention in days (retention.body_days)
-- Statements are separated by a line containing only `;`... no: by `;` at end of line.

CREATE TABLE IF NOT EXISTS {{db}}.requests
(
    project_id        LowCardinality(String),
    environment       LowCardinality(String),
    service           LowCardinality(String),
    release           LowCardinality(String),
    timestamp         DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)),

    request_id        String CODEC(ZSTD(1)),
    trace_id          String CODEC(ZSTD(1)),
    span_id           String CODEC(ZSTD(1)),

    method            LowCardinality(String),
    route             LowCardinality(String),
    path              String CODEC(ZSTD(1)),
    query             String CODEC(ZSTD(1)),
    status            UInt16,
    duration_ms       Float32,
    req_size          UInt32,
    res_size          UInt32,

    user_id           String CODEC(ZSTD(1)),
    client            LowCardinality(String),
    client_version    LowCardinality(String),
    ip                String CODEC(ZSTD(1)),
    user_agent        String CODEC(ZSTD(1)),

    req_headers       Map(LowCardinality(String), String) CODEC(ZSTD(3)) TTL toDateTime(timestamp) + INTERVAL {{body_days}} DAY,
    res_headers       Map(LowCardinality(String), String) CODEC(ZSTD(3)) TTL toDateTime(timestamp) + INTERVAL {{body_days}} DAY,
    req_body          String CODEC(ZSTD(3)) TTL toDateTime(timestamp) + INTERVAL {{body_days}} DAY,
    res_body          String CODEC(ZSTD(3)) TTL toDateTime(timestamp) + INTERVAL {{body_days}} DAY,
    body_truncated    UInt8,

    error_type        LowCardinality(String),
    error_message     String CODEC(ZSTD(1)),
    error_stack       String CODEC(ZSTD(3)),
    error_fingerprint UInt64,

    attributes        Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    correlation       Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    sample_rate       Float32 DEFAULT 1,

    INDEX idx_request_id request_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_user_id user_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_correlation mapValues(correlation) TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_status status TYPE minmax GRANULARITY 4,
    INDEX idx_duration duration_ms TYPE minmax GRANULARITY 4
)
ENGINE = MergeTree
PARTITION BY toYYYYMMDD(timestamp)
ORDER BY (project_id, service, route, timestamp)
TTL toDateTime(timestamp) + INTERVAL {{days}} DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS {{db}}.spans
(
    project_id        LowCardinality(String),
    trace_id          String CODEC(ZSTD(1)),
    span_id           String CODEC(ZSTD(1)),
    parent_span_id    String CODEC(ZSTD(1)),
    request_id        String CODEC(ZSTD(1)),
    service           LowCardinality(String),
    kind              LowCardinality(String),
    name              String CODEC(ZSTD(1)),
    start             DateTime64(6, 'UTC') CODEC(Delta, ZSTD(1)),
    duration_ms       Float32,
    status            LowCardinality(String),
    attributes        Map(LowCardinality(String), String) CODEC(ZSTD(1)),

    INDEX idx_request_id request_id TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = MergeTree
PARTITION BY toYYYYMMDD(start)
ORDER BY (project_id, trace_id, start)
TTL toDateTime(start) + INTERVAL {{days}} DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS {{db}}.events
(
    project_id        LowCardinality(String),
    environment       LowCardinality(String),
    service           LowCardinality(String),
    release           LowCardinality(String),
    timestamp         DateTime64(3, 'UTC') CODEC(Delta, ZSTD(1)),
    kind              LowCardinality(String),
    name              LowCardinality(String),
    level             LowCardinality(String),
    message           String CODEC(ZSTD(3)),
    status            LowCardinality(String),
    duration_ms       Float32,
    request_id        String CODEC(ZSTD(1)),
    trace_id          String CODEC(ZSTD(1)),
    user_id           String CODEC(ZSTD(1)),
    attempt           UInt16,
    error_type        LowCardinality(String),
    error_message     String CODEC(ZSTD(1)),
    error_stack       String CODEC(ZSTD(3)),
    error_fingerprint UInt64,
    attributes        Map(LowCardinality(String), String) CODEC(ZSTD(1)),
    correlation       Map(LowCardinality(String), String) CODEC(ZSTD(1)),

    INDEX idx_request_id request_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_user_id user_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_correlation mapValues(correlation) TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = MergeTree
PARTITION BY toYYYYMMDD(timestamp)
ORDER BY (project_id, service, kind, timestamp)
TTL toDateTime(timestamp) + INTERVAL {{days}} DAY
SETTINGS index_granularity = 8192, ttl_only_drop_parts = 1;

-- Per-minute rollup that powers tiles, sparklines and the route heatmap.
CREATE TABLE IF NOT EXISTS {{db}}.route_minute
(
    project_id   LowCardinality(String),
    environment  LowCardinality(String),
    service      LowCardinality(String),
    route        LowCardinality(String),
    method       LowCardinality(String),
    minute       DateTime('UTC'),
    requests     AggregateFunction(count),
    weighted     AggregateFunction(sum, Float64),
    errors_5xx   AggregateFunction(countIf, UInt8),
    errors_4xx   AggregateFunction(countIf, UInt8),
    slow         AggregateFunction(countIf, UInt8),
    latency      AggregateFunction(quantilesTDigest(0.5, 0.95, 0.99), Float32),
    releases     AggregateFunction(groupUniqArray(8), String)
)
ENGINE = AggregatingMergeTree
PARTITION BY toYYYYMM(minute)
ORDER BY (project_id, environment, service, route, method, minute)
TTL minute + INTERVAL {{days}} DAY;

CREATE MATERIALIZED VIEW IF NOT EXISTS {{db}}.route_minute_mv TO {{db}}.route_minute AS
SELECT
    project_id,
    environment,
    service,
    route,
    method,
    toStartOfMinute(timestamp) AS minute,
    countState() AS requests,
    sumState(toFloat64(1 / if(sample_rate > 0, sample_rate, 1))) AS weighted,
    countIfState(status >= 500) AS errors_5xx,
    countIfState(status >= 400) AS errors_4xx,
    countIfState(duration_ms >= 1000) AS slow,
    quantilesTDigestState(0.5, 0.95, 0.99)(duration_ms) AS latency,
    groupUniqArrayState(8)(toString(release)) AS releases
FROM {{db}}.requests
GROUP BY project_id, environment, service, route, method, minute;

-- Error groups: one row per (project, environment, fingerprint), fed from requests and events.
CREATE TABLE IF NOT EXISTS {{db}}.error_groups
(
    project_id        LowCardinality(String),
    environment       LowCardinality(String),
    error_fingerprint UInt64,
    first_seen        SimpleAggregateFunction(min, DateTime64(3, 'UTC')),
    last_seen         SimpleAggregateFunction(max, DateTime64(3, 'UTC')),
    count             SimpleAggregateFunction(sum, UInt64),
    error_type        SimpleAggregateFunction(anyLast, String),
    sample_message    SimpleAggregateFunction(anyLast, String),
    service           SimpleAggregateFunction(anyLast, String),
    route             SimpleAggregateFunction(anyLast, String),
    source            SimpleAggregateFunction(anyLast, String),
    sample_request_id AggregateFunction(argMax, String, DateTime64(3, 'UTC')),
    first_release     AggregateFunction(argMin, String, DateTime64(3, 'UTC')),
    users             AggregateFunction(uniq, String)
)
ENGINE = AggregatingMergeTree
ORDER BY (project_id, environment, error_fingerprint)
TTL toDateTime(last_seen) + INTERVAL {{days}} DAY;

CREATE MATERIALIZED VIEW IF NOT EXISTS {{db}}.error_groups_requests_mv TO {{db}}.error_groups AS
SELECT
    project_id,
    environment,
    error_fingerprint,
    min(timestamp) AS first_seen,
    max(timestamp) AS last_seen,
    toUInt64(count()) AS count,
    anyLast(toString(error_type)) AS error_type,
    anyLast(error_message) AS sample_message,
    anyLast(toString(service)) AS service,
    anyLast(toString(route)) AS route,
    'request' AS source,
    argMaxState(request_id, timestamp) AS sample_request_id,
    argMinState(toString(release), timestamp) AS first_release,
    uniqState(user_id) AS users
FROM {{db}}.requests
WHERE error_fingerprint != 0
GROUP BY project_id, environment, error_fingerprint;

CREATE MATERIALIZED VIEW IF NOT EXISTS {{db}}.error_groups_events_mv TO {{db}}.error_groups AS
SELECT
    project_id,
    environment,
    error_fingerprint,
    min(timestamp) AS first_seen,
    max(timestamp) AS last_seen,
    toUInt64(count()) AS count,
    anyLast(toString(error_type)) AS error_type,
    anyLast(if(error_message != '', error_message, message)) AS sample_message,
    anyLast(toString(service)) AS service,
    anyLast(toString(name)) AS route,
    anyLast(toString(kind)) AS source,
    argMaxState(request_id, timestamp) AS sample_request_id,
    argMinState(toString(release), timestamp) AS first_release,
    uniqState(user_id) AS users
FROM {{db}}.events
WHERE error_fingerprint != 0
GROUP BY project_id, environment, error_fingerprint;

-- Per-minute error counts for sparklines and spike detection.
CREATE TABLE IF NOT EXISTS {{db}}.error_minute
(
    project_id        LowCardinality(String),
    environment       LowCardinality(String),
    error_fingerprint UInt64,
    minute            DateTime('UTC'),
    count             UInt64
)
ENGINE = SummingMergeTree(count)
PARTITION BY toYYYYMM(minute)
ORDER BY (project_id, environment, error_fingerprint, minute)
TTL minute + INTERVAL {{days}} DAY;

CREATE MATERIALIZED VIEW IF NOT EXISTS {{db}}.error_minute_requests_mv TO {{db}}.error_minute AS
SELECT project_id, environment, error_fingerprint, toStartOfMinute(timestamp) AS minute, toUInt64(count()) AS count
FROM {{db}}.requests
WHERE error_fingerprint != 0
GROUP BY project_id, environment, error_fingerprint, minute;

CREATE MATERIALIZED VIEW IF NOT EXISTS {{db}}.error_minute_events_mv TO {{db}}.error_minute AS
SELECT project_id, environment, error_fingerprint, toStartOfMinute(timestamp) AS minute, toUInt64(count()) AS count
FROM {{db}}.events
WHERE error_fingerprint != 0
GROUP BY project_id, environment, error_fingerprint, minute;
