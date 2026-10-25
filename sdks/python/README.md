# logbox-sdk

The [Logbox](https://github.com/karaa1122/LogBox) SDK for Python 3.10+: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, plus Celery jobs and log lines, without blocking your request path. No dependencies beyond the standard library.

```bash
pip install logbox-sdk   # imported as `logbox`
```

## FastAPI / Starlette

```python
import logbox
from logbox.asgi import LogboxMiddleware

logbox.init(service="orders-api")  # reads LOGBOX_ENDPOINT and LOGBOX_INGEST_KEY
app = FastAPI()
app.add_middleware(LogboxMiddleware)  # add it last, so it runs first
```

## Django

```python
MIDDLEWARE = ["logbox.django.LogboxMiddleware", ...]  # first
LOGBOX = {"service": "orders-api"}
```

## Celery and logging

```python
from logbox.celery import instrument

instrument()  # in the web process and the worker, after logbox.init()
logging.getLogger().addHandler(logbox.LogboxHandler())
```

In request code: `logbox.set_user(user_id)`, `logbox.set_correlation("order_id", order_id)`.

Full documentation: [docs/sdk-python.md](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-python.md).

## License

Apache-2.0
