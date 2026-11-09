# trailscope

The [Trailscope](https://github.com/karaa1122/trailscope) SDK for Python 3.10+: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, plus Celery jobs and log lines, without blocking your request path. No dependencies beyond the standard library.

```bash
pip install trailscope
```

## FastAPI / Starlette

```python
import trailscope
from trailscope.asgi import TrailscopeMiddleware

trailscope.init(service="orders-api")  # reads TRAILSCOPE_ENDPOINT and TRAILSCOPE_INGEST_KEY
app = FastAPI()
app.add_middleware(TrailscopeMiddleware)  # add it last, so it runs first
```

## Django

```python
MIDDLEWARE = ["trailscope.django.TrailscopeMiddleware", ...]  # first
TRAILSCOPE = {"service": "orders-api"}
```

## Celery and logging

```python
from trailscope.celery import instrument

instrument()  # in the web process and the worker, after trailscope.init()
logging.getLogger().addHandler(trailscope.TrailscopeHandler())
```

In request code: `trailscope.set_user(user_id)`, `trailscope.set_correlation("order_id", order_id)`.

Full documentation: [docs/sdk-python.md](https://github.com/karaa1122/trailscope/blob/main/docs/sdk-python.md).

## License

Apache-2.0
