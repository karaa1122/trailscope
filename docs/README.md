# Trailscope documentation

Trailscope is open-source, self-hosted API request logging. Your services send every request they handle (method, route, status, timing, headers, bodies if you allow it, the error and its stack) to your own Trailscope server, where you can watch them live, filter them with a SQL `WHERE` expression, open any one of them, and follow one customer's journey across services and background jobs.
## Start here

1. **Run it.** One `docker compose up -d` starts Trailscope with ClickHouse and Postgres; the setup wizard creates your account, your first project and its ingest key. [Self-hosting](self-hosting.md) covers sizing, TLS, upgrades and backups, and [configuration](configuration.md) lists every setting.
2. **Send requests.** Add the SDK for your framework, or point an OpenTelemetry exporter at Trailscope (see "Send data" below).
3. **Use it.** Watch the live tail, filter in Explore with the [query language](query-language.md), and set up [alerts and email on 5xx](alerts.md).

Before you put real traffic through it, read [security](security.md): it explains what is collected, how redaction works and what to configure.

## Send data

| Your stack | Page |
|---|---|
| Express | [Express](sdk-express.md) |
| Fastify | [Fastify](sdk-fastify.md) |
| NestJS (with the Logger bridge and BullMQ) | [NestJS](sdk-nestjs.md) |
| Any other Node.js code | [Node.js core](sdk-node.md) |
| Django, FastAPI, Starlette, Celery, Python logging | [Python](sdk-python.md) |
| Spring Boot, Jakarta Servlet apps, Logback | [Java](sdk-java.md) |
| Laravel (queues, log channel, Octane), plain PHP, Monolog | [PHP and Laravel](sdk-laravel.md) |
| Anything with OpenTelemetry, or raw HTTP | [Ingest protocol and OTLP](ingest-protocol.md) |

All SDKs behave the same way (batching, redaction, sampling, user and journey keys); [SDK behaviour](sdk-spec.md) describes it.

## Reference

- [Query language](query-language.md): filters, quick search and every column.
- [Alerts and email notifications](alerts.md): rules, channels and Email on 5xx.
- [HTTP API](api.md): everything the UI does, with API tokens and an OpenAPI document.
- [Architecture](architecture.md): how ingest, storage, queries and alerts fit together.
- [Benchmarks](benchmarks.md): ingest throughput, query latency and SDK overhead, with how they were measured.
- [Decisions](DECISIONS.md): why things are the way they are.
