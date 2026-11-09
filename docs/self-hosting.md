# Self-hosting

Trailscope runs as one container next to ClickHouse and Postgres. This page covers sizing, installing, TLS, upgrades, backups and monitoring. Settings are documented in [configuration.md](configuration.md); the security checklist is in [security.md](security.md#operator-checklist).

## Sizing

Disk is driven by request volume and retention. On the benchmark dataset a stored request takes **about 70 bytes** compressed in ClickHouse (headers, attributes and correlation keys; bodies only for errors and slow requests) and a span about 35 bytes. Real traffic with bigger headers and more captured bodies is often two or three times that, so plan for 70–200 bytes per request:

| Requests per day | Disk for 30 days of retention | Machine |
|---|---|---|
| up to 1 million | 2–6 GB | 2 vCPU, 4 GB RAM (the default low-memory ClickHouse config) |
| up to 10 million | 20–60 GB | 4 vCPU, 8 GB RAM |
| up to 100 million | 200–600 GB | 8+ vCPU, 32 GB RAM, fast SSD; consider split mode |

For reference, one `trailscope all` process limited to 4 CPUs ingested 50,000 events per second (about 4 billion a day) at p99 17 ms, and filtered queries over 100 million requests answer in well under half a second on a 16 GB machine ([benchmarks.md](benchmarks.md)). Shorter retention (`TRAILSCOPE_RETENTION_DAYS`) or lower sampling for healthy requests (Settings → Project) are the levers if disk runs short; bodies and headers already expire sooner (`TRAILSCOPE_RETENTION_BODY_DAYS`, default 7).

## Install with Docker Compose

```bash
git clone https://github.com/karaa1122/trailscope && cd trailscope/deploy
cp .env.example .env        # set TRAILSCOPE_PUBLIC_URL, TRAILSCOPE_SECRET_KEY and the three passwords
docker compose up -d
```

Open `TRAILSCOPE_PUBLIC_URL` (http://localhost:8080 by default). The setup wizard creates the owner account and the first project, shows its ingest key once, and has copy-paste snippets for every SDK. Migrations run automatically at startup.

Set `TRAILSCOPE_IMAGE=ghcr.io/karaa1122/trailscope:<version>` in `.env` (for example `0.1.0`; images for linux/amd64 and arm64 are listed on the [package page](https://github.com/karaa1122/trailscope/pkgs/container/trailscope)) to pull the published image. Without it, the compose file builds the image from source the first time, which takes several minutes.

ClickHouse is started with `deploy/clickhouse/config.d/low-memory.xml`, tuned for a 2–4 GB machine (smaller caches, fewer background threads, short internal logs). On a machine with 16 GB or more you can remove that mount.

## TLS behind a reverse proxy

Trailscope speaks plain HTTP; put a reverse proxy in front for TLS and set `TRAILSCOPE_PUBLIC_URL` to the https URL (session cookies then get the `Secure` flag). Live tail uses Server-Sent Events, so the proxy must not buffer `/api/v1/live`, and ingest batches can be up to 5 MB compressed.

**Caddy** (gets certificates automatically):

```
logs.example.com {
    reverse_proxy localhost:8080 {
        flush_interval -1
    }
}
```

**nginx:**

```nginx
server {
    listen 443 ssl http2;
    server_name logs.example.com;
    # ssl_certificate ... ; ssl_certificate_key ... ;

    client_max_body_size 6m;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/v1/live {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 1h;
    }
}
```

With a proxy that sets `X-Forwarded-For`, also set `TRAILSCOPE_SERVER_TRUST_PROXY=true` so login throttling sees real client IPs. Bind Trailscope to localhost (`TRAILSCOPE_PORT=127.0.0.1:8080` in `.env`) so only the proxy can reach it.

## Split mode

`deploy/docker-compose.split.yml` runs ingest, API and alerts as separate services with Redis for live tail, and a one-shot `migrate` service:

```bash
docker compose -f docker-compose.split.yml up -d
```

The UI and API are on port 8080 and ingest on 8081 (point SDKs there). Scale ingest and API replicas independently; keep **one** alerts replica ([alerts.md](alerts.md#running-in-split-mode)). Split mode is worth it when ingest load and UI traffic need separate machines; `trailscope all` handles tens of thousands of events per second on its own (see [benchmarks.md](benchmarks.md)).

## Upgrades

```bash
cd trailscope && git pull && cd deploy
docker compose up -d --build               # building from source
# or, with a published image: bump the version in TRAILSCOPE_IMAGE, then
docker compose pull trailscope && docker compose up -d
```

Migrations are applied at startup and are forward-only, so back up Postgres before upgrading across minor versions. Retention changes (`TRAILSCOPE_RETENTION_DAYS`, `TRAILSCOPE_RETENTION_BODY_DAYS`) are applied to the ClickHouse TTLs on the next start, without rewriting data.

## Backups

- **Postgres** holds everything that isn't telemetry: users, projects, keys, saved views, alert rules, settings. It is small and the one thing you must back up:

  ```bash
  docker compose exec -T postgres pg_dump -U trailscope -Fc trailscope > trailscope-$(date +%F).dump
  # restore: docker compose exec -T postgres pg_restore -U trailscope -d trailscope --clean < trailscope-....dump
  ```

- **`TRAILSCOPE_SECRET_KEY`**: keep it with the Postgres backup; stored SMTP and channel secrets can't be decrypted without it.
- **ClickHouse** holds the telemetry, which expires anyway. Back it up only if you need history to survive a disk loss, with ClickHouse's `BACKUP DATABASE trailscope TO Disk('backups', 'trailscope.zip')` (configure a backup disk) or a volume snapshot.

## Monitoring

| Endpoint | Use |
|---|---|
| `/healthz` | liveness: the process is up |
| `/readyz` | readiness: ClickHouse and Postgres are reachable (JSON with each check) |
| `/metrics` | Prometheus metrics |

Metrics worth alerting on: `trailscope_ingest_throttled_total` (ingest answering 429: the queue is full), `trailscope_ingest_flush_errors_total` and `trailscope_ingest_dropped_rows_total` (ClickHouse writes failing), `trailscope_ingest_queue_depth` and `trailscope_ingest_queue_bytes`, `trailscope_notifications_failed_total`. Also useful: `trailscope_ingest_request_seconds`, `trailscope_ingest_flush_seconds`, `trailscope_query_seconds`, `trailscope_live_clients`, `trailscope_otlp_requests_total`.

Logs go to stdout; set `TRAILSCOPE_LOG_FORMAT=json` (the compose default) for log shippers and `TRAILSCOPE_LOG_LEVEL=debug` when investigating.
