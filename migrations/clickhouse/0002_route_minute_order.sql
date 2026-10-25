-- Re-sort the per-minute route rollup by time right after the project, so tiles,
-- the route heatmap and route charts read only their window instead of the whole
-- retention period. Its rows are large (latency digests), so granules are smaller.

CREATE TABLE IF NOT EXISTS {{db}}.route_minute_v2
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
ORDER BY (project_id, minute, environment, service, route, method)
TTL minute + INTERVAL {{days}} DAY
SETTINGS index_granularity = 1024;

-- Rows written between these statements would miss the rollup, but migrations run
-- before ingest starts.
DROP VIEW IF EXISTS {{db}}.route_minute_mv;

INSERT INTO {{db}}.route_minute_v2 SELECT * FROM {{db}}.route_minute;

EXCHANGE TABLES {{db}}.route_minute AND {{db}}.route_minute_v2;

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

DROP TABLE IF EXISTS {{db}}.route_minute_v2;
