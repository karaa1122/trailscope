import logging
from typing import Any

from .celery import celery_app

logger = logging.getLogger("tasks")


@celery_app.task(bind=True, max_retries=2, default_retry_delay=1)
def notify(self: Any, task_id: str, flaky: bool = False) -> None:
    # A task titled "flaky" fails its first attempt, to show retries.
    if flaky and self.request.retries == 0:
        raise self.retry(exc=RuntimeError("mail server busy"))
    logger.info("notified watchers of %s", task_id)
