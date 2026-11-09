import os
from typing import Any

from celery import Celery
from celery.signals import worker_init

import trailscope
from trailscope.celery import instrument

os.environ.setdefault("DJANGO_SETTINGS_MODULE", "app.settings")

celery_app = Celery("app", broker=os.environ.get("REDIS_URL", "redis://localhost:6379/0"))
celery_app.conf.broker_connection_retry_on_startup = True
celery_app.autodiscover_tasks(["app"])

# Web process: sends the request context with each task. Worker: records each run.
# Log lines reach Trailscope through settings.LOGGING, which Celery applies in the worker too.
instrument()


@worker_init.connect
def start_trailscope(**_: Any) -> None:
    trailscope.init(service="tasks-worker-django", endpoint=os.environ.get("TRAILSCOPE_ENDPOINT", "http://localhost:8080"))
