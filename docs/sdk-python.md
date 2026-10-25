# Python: `logbox-sdk`

Request logging for Django and ASGI apps (FastAPI, Starlette), job events for Celery, and a `logging` handler. Python 3.10+, standard library only. Shared behaviour is described in [sdk-spec.md](sdk-spec.md).

```bash
pip install logbox-sdk
```

The package is published as `logbox-sdk` and imported as `logbox`.

## FastAPI and Starlette

```python
import logbox
from logbox.asgi import LogboxMiddleware

logbox.init(service="orders-api")      # reads LOGBOX_ENDPOINT and LOGBOX_INGEST_KEY

app = FastAPI()

@app.get("/orders/{order_id}")
async def get_order(order_id: str):
    logbox.set_correlation("order_id", order_id)
    return await load_order(order_id)

app.add_middleware(LogboxMiddleware)   # last, so it runs first
```

Starlette runs the most recently added middleware outermost. Add `LogboxMiddleware` **after** your other middleware so it wraps them: otherwise context set in an authentication middleware (`set_user()`) is lost and their time isn't counted.

`LogboxMiddleware(app, client=None, **options)` takes a `Client`, or keyword options to create the process-wide client with `init()`; without either it uses the client from `init()`. It works with any ASGI app (`app = LogboxMiddleware(app)`). Routes come from `scope["route"]` (`/orders/{order_id}` becomes `/orders/:order_id`, with mount prefixes). Unhandled exceptions are recorded with their stack and re-raised; a cancelled request (client gone) is recorded with status 499.

## Django

```python
# settings.py
MIDDLEWARE = [
    "logbox.django.LogboxMiddleware",   # first, so it times everything
    # ...
]
LOGBOX = {"service": "orders-api"}      # keyword options for logbox.Client
```

The middleware works under WSGI and ASGI. Routes come from `resolver_match.route` (`orders/<int:order_id>` becomes `/orders/:order_id`). Exceptions raised in views are recorded through `process_exception`. The authenticated user is recorded automatically if the request already loaded it (no extra query); otherwise call `logbox.set_user()`. Request bodies are read only when the capture policy keeps them, and not for multipart or binary content; streaming responses are skipped.

## Celery

```python
import logbox
from celery.signals import worker_init
from logbox.celery import instrument

instrument()   # import this module in the web process and the worker

@worker_init.connect
def start_logbox(**_):
    logbox.init(service="orders-worker")
```

`instrument()` adds the current request's context to each task message (a `logbox` header) and records every run in the worker as a `kind=job` event with the task name, duration, outcome and attempt number (`retries + 1`), linked to the request that queued it. Retries are recorded as errors with level `warn`. The client handles preforked workers: each child starts its own flush thread.

## Logging

```python
logging.getLogger().addHandler(logbox.LogboxHandler(level=logging.INFO))
```

Each record becomes a `log` event named after its logger, with the module and line as attributes, the exception from `exc_info` as the error, and the current request's or job's context. Records from Logbox's own `logbox` logger are skipped. In Django, add it through `LOGGING` (`"class": "logbox.LogboxHandler"`).

## Client

```python
client = logbox.init(service="orders-api")   # process-wide; logbox.Client(...) for an independent one

# Context of the current request (no-ops outside one)
logbox.set_user(user_id); logbox.set_correlation("account_id", account_id); logbox.set_attribute("plan", "pro")
logbox.record_error(exc); logbox.current_context()

# Events, logs and spans
client.capture_event("export.created", attributes={"rows": 1200})
client.log("warn", "cache miss storm", attributes={"keyspace": "sessions"})
client.capture_exception(exc)
with client.span("db", "SELECT orders"):
    rows = query()

# Jobs: a context manager or a decorator (sync or async); inside a request it belongs to it
with client.track_job("report.build", attempt=1):
    build()
carried = logbox.serialize_context()          # store with the job...
with client.track_job("report.build", context=logbox.deserialize_context(carried)):  # ...restore in the worker
    build()

client.flush(); client.shutdown(); client.stats()
```

### Options

| Option | Env | Default | |
|---|---|---|---|
| `endpoint` | `LOGBOX_ENDPOINT` | `http://localhost:8080` | |
| `ingest_key` | `LOGBOX_INGEST_KEY` | | Without one nothing is sent |
| `service` | `LOGBOX_SERVICE` | `unknown-service` | |
| `environment` | `LOGBOX_ENVIRONMENT` | `production` | |
| `release` | `LOGBOX_RELEASE` | | |
| `enabled` | `LOGBOX_ENABLED=0` disables | `True` | |
| `debug` | `LOGBOX_DEBUG=1` | `False` | Logs SDK diagnostics on the `logbox` logger |
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
