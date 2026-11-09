# Python: `trailscope`

Request logging for Django and ASGI apps (FastAPI, Starlette), job events for Celery, and a `logging` handler. Python 3.10+, standard library only. Shared behaviour is described in [sdk-spec.md](sdk-spec.md).

```bash
pip install trailscope
```

The package is published and imported as `trailscope`.

## FastAPI and Starlette

```python
import trailscope
from trailscope.asgi import TrailscopeMiddleware

trailscope.init(service="orders-api")      # reads TRAILSCOPE_ENDPOINT and TRAILSCOPE_INGEST_KEY

app = FastAPI()

@app.get("/orders/{order_id}")
async def get_order(order_id: str):
    trailscope.set_correlation("order_id", order_id)
    return await load_order(order_id)

app.add_middleware(TrailscopeMiddleware)   # last, so it runs first
```

Starlette runs the most recently added middleware outermost. Add `TrailscopeMiddleware` **after** your other middleware so it wraps them: otherwise context set in an authentication middleware (`set_user()`) is lost and their time isn't counted.

`TrailscopeMiddleware(app, client=None, **options)` takes a `Client`, or keyword options to create the process-wide client with `init()`; without either it uses the client from `init()`. It works with any ASGI app (`app = TrailscopeMiddleware(app)`). Routes come from `scope["route"]` (`/orders/{order_id}` becomes `/orders/:order_id`, with mount prefixes). Unhandled exceptions are recorded with their stack and re-raised; a cancelled request (client gone) is recorded with status 499.

## Django

```python
# settings.py
MIDDLEWARE = [
    "trailscope.django.TrailscopeMiddleware",   # first, so it times everything
    # ...
]
TRAILSCOPE = {"service": "orders-api"}      # keyword options for trailscope.Client
```

The middleware works under WSGI and ASGI. Routes come from `resolver_match.route` (`orders/<int:order_id>` becomes `/orders/:order_id`). Exceptions raised in views are recorded through `process_exception`. The authenticated user is recorded automatically if the request already loaded it (no extra query); otherwise call `trailscope.set_user()`. Request bodies are read only when the capture policy keeps them. For multipart forms (Postman form-data, file uploads), the text fields Django or DRF parsed are kept and files are listed by name, size and type; other binary bodies and streaming responses are skipped.

## Celery

```python
import trailscope
from celery.signals import worker_init
from trailscope.celery import instrument

instrument()   # import this module in the web process and the worker

@worker_init.connect
def start_trailscope(**_):
    trailscope.init(service="orders-worker")
```

`instrument()` adds the current request's context to each task message (a `trailscope` header) and records every run in the worker as a `kind=job` event with the task name, duration, outcome and attempt number (`retries + 1`), linked to the request that queued it. Retries are recorded as errors with level `warn`. The client handles preforked workers: each child starts its own flush thread.

## Logging

```python
logging.getLogger().addHandler(trailscope.TrailscopeHandler(level=logging.INFO))
```

Each record becomes a `log` event named after its logger, with the module and line as attributes, the exception from `exc_info` as the error, and the current request's or job's context. Records from Trailscope's own `trailscope` logger are skipped. In Django, add it through `LOGGING` (`"class": "trailscope.TrailscopeHandler"`).

## Client

```python
client = trailscope.init(service="orders-api")   # process-wide; trailscope.Client(...) for an independent one

# Context of the current request (no-ops outside one)
trailscope.set_user(user_id); trailscope.set_correlation("account_id", account_id); trailscope.set_attribute("plan", "pro")
trailscope.record_error(exc); trailscope.current_context()

# Events, logs and spans
client.capture_event("export.created", attributes={"rows": 1200})
client.log("warn", "cache miss storm", attributes={"keyspace": "sessions"})
client.capture_exception(exc)
with client.span("db", "SELECT orders"):
    rows = query()

# Jobs: a context manager or a decorator (sync or async); inside a request it belongs to it
with client.track_job("report.build", attempt=1):
    build()
carried = trailscope.serialize_context()          # store with the job...
with client.track_job("report.build", context=trailscope.deserialize_context(carried)):  # ...restore in the worker
    build()

client.flush(); client.shutdown(); client.stats()
```

### Options

| Option | Env | Default | |
|---|---|---|---|
| `endpoint` | `TRAILSCOPE_ENDPOINT` | `http://localhost:8080` | |
| `ingest_key` | `TRAILSCOPE_INGEST_KEY` | | Without one nothing is sent |
| `service` | `TRAILSCOPE_SERVICE` | `unknown-service` | |
| `environment` | `TRAILSCOPE_ENVIRONMENT` | `production` | |
| `release` | `TRAILSCOPE_RELEASE` | | |
| `enabled` | `TRAILSCOPE_ENABLED=0` disables | `True` | |
| `debug` | `TRAILSCOPE_DEBUG=1` | `False` | Logs SDK diagnostics on the `trailscope` logger |
| `max_queue_size` | | `10000` | Oldest dropped beyond this |
| `flush_interval` / `flush_at` / `max_batch_size` | | `1.0` s / `500` / `1000` | |
| `request_timeout` / `shutdown_timeout` | | `10.0` / `5.0` s | |
| `sample_rate` / `route_sample_rates` | | `1.0` / `{}` | Errors and slow requests are always kept |
| `slow_ms` | | `1000` | |
| `capture_bodies` | | `"errors"` | `"always"`, `"never"` |
| `max_body_bytes` | | `8192` | Per body |
| `ignore_routes` | | health and readiness routes | Strings (exact) or compiled regexes |
| `request_id_header` | | `x-request-id` | |
| `redact_headers` / `redact_keys` / `redact_patterns` | | | Added to the built-in rules; keys may be dotted paths with `*` |
| `mask_ip` | | `True` | |
| `disable_default_redaction` | | `False` | Not recommended |
| `remote_config` | | `True` | Load sampling and capture settings from `/v1/config` |
| `before_send` | | | Edit an item dict or return `None` to drop it; runs in the flush thread |

Sampling and capture options set here win over the project settings from the server.

Runnable apps: [examples/fastapi](../examples/fastapi) and [examples/django](../examples/django) (with Celery).
