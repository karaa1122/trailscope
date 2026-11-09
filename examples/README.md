# Examples

Small apps wired to the Trailscope SDKs. Each one serves the same few routes under `/projects/:projectId/…`: a list, a create that starts a background job, an endpoint that throws (a 500 with a stack), and a health check that isn't recorded.

| Example | Shows |
|---|---|
| [express](express) | middleware, error handler, `setUser`/`setCorrelation`, a span, an in-process job |
| [fastify](fastify) | plugin, context in handlers after body parsing, an in-process job |
| [nestjs](nestjs) | `TrailscopeModule`, exception filter, `TrailscopeLogger`, guards setting the user, BullMQ with `withTrailscopeContext` and `@TrackJob` (Express or Fastify platform) |
| [fastapi](fastapi) | ASGI middleware, `TrailscopeHandler`, a span, an async job |
| [django](django) | middleware and settings, `LOGGING`, Celery tasks with retries |

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
```

Then send some traffic and open Live tail:

```bash
curl -H 'x-user-id: u_1' localhost:3000/projects/p1/tasks
curl -H 'x-user-id: u_1' -H 'content-type: application/json' -d '{"title":"flaky"}' localhost:3000/projects/p1/tasks
curl localhost:3000/projects/p1/export
```

The NestJS and Django examples need Redis for their job queues. In those two, a task titled `flaky` fails its first notification attempt, so you can see a retry in the request's journey.
