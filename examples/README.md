# Examples

Small apps wired to the Trailscope SDKs. Each one serves the same few routes under `/projects/:projectId/…`: a list, a create that starts a background job, an endpoint that throws (a 500 with a stack), and a health check that isn't recorded.

| Example | Shows |
|---|---|
| [express](express) | middleware, error handler, `setUser`/`setCorrelation`, a span, an in-process job |
| [fastify](fastify) | plugin, context in handlers after body parsing, an in-process job |
| [nestjs](nestjs) | `TrailscopeModule`, exception filter, `TrailscopeLogger`, guards setting the user, BullMQ with `withTrailscopeContext` and `@TrackJob` (Express or Fastify platform) |
| [fastapi](fastapi) | ASGI middleware, `TrailscopeHandler`, a span, an async job |
| [django](django) | middleware and settings, `LOGGING`, Celery tasks with retries |
| [laravel](laravel) | the auto-discovered package, the signed-in user picked up from Laravel's auth, a span, the `trailscope` log channel, a database queue (SQLite) with a retried job |
| [spring-boot](spring-boot) | the Spring Boot starter with properties, an interceptor setting the user, an `@Async` job with a retry, the Logback appender, a span |

## Running them

Start Trailscope (`docker compose up -d` in `deploy/`), create a project and copy its ingest key, then:

```bash
export TRAILSCOPE_ENDPOINT=http://localhost:8080 TRAILSCOPE_INGEST_KEY=trs_ingest_…

# Node examples (from the repository root)
pnpm install && pnpm --filter "./sdks/node/**" build
node examples/express/src/server.js
node examples/fastify/src/server.js
cd examples/nestjs && pnpm build && REDIS_URL=redis://localhost:6379 node dist/main.js

# Python examples
cd examples/fastapi && pip install -r requirements.txt && uvicorn main:app --port 3000
cd examples/django && pip install -r requirements.txt
REDIS_URL=redis://localhost:6379/0 celery -A app worker &
python manage.py runserver 3000

# Laravel example (PHP 8.3+ and Composer); put TRAILSCOPE_INGEST_KEY in .env,
# because `artisan serve` ignores shell variables when a .env file exists
cd examples/laravel && composer install && cp .env.example .env
touch database/database.sqlite && php artisan migrate
php artisan queue:work &
php artisan serve --port 3000

# Java example (JDK 17+ and Maven)
(cd sdks/java && mvn -q install)
cd examples/spring-boot && mvn spring-boot:run      # listens on port 3000
```

Then send some traffic and open Live tail:

```bash
curl -H 'x-user-id: u_1' localhost:3000/projects/p1/tasks
curl -H 'x-user-id: u_1' -H 'content-type: application/json' -d '{"title":"flaky"}' localhost:3000/projects/p1/tasks
curl localhost:3000/projects/p1/export
```

The NestJS and Django examples need Redis for their job queues, the Laravel example uses SQLite, and the Spring Boot example retries in-process. In those four, a task titled `flaky` fails its first notification attempt, so you can see a retry in the request's journey.
