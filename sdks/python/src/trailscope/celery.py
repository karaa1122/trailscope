"""Celery integration: every task run becomes a ``kind=job`` event, linked to the
request that queued it.

Call it once in the web process (to send the context with each task) and in
the worker (to record runs), after ``trailscope.init()``::

    import trailscope
    from trailscope.celery import instrument

    trailscope.init(service="orders-worker")
    instrument()
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any

from celery import signals

from ._context import (
    TrailscopeContext,
    deserialize_context,
    reset_context,
    serialize_context,
    set_context,
)
from ._ids import new_request_id, new_span_id, new_trace_id
from .client import Client, get_client

#: Message header that carries the serialized context.
HEADER = "trailscope"


@dataclass
class _Run:
    client: Client
    ctx: TrailscopeContext
    token: Any
    t0: float
    timestamp: int
    error: BaseException | None = None
    attributes: dict[str, str] = field(default_factory=dict)


_runs: dict[str, _Run] = {}
_client: Client | None = None
_instrumented = False


def instrument(client: Client | None = None) -> None:
    """Connects the Celery signal handlers (idempotent)."""
    global _client, _instrumented
    _client = client
    if _instrumented:
        return
    _instrumented = True
    signals.before_task_publish.connect(_before_publish, weak=False)
    signals.task_prerun.connect(_prerun, weak=False)
    signals.task_failure.connect(_failure, weak=False)
    signals.task_retry.connect(_retry, weak=False)
    signals.task_postrun.connect(_postrun, weak=False)


def _before_publish(sender: Any = None, headers: Any = None, **_: Any) -> None:
    if isinstance(headers, dict):
        carried = serialize_context()
        if carried:
            headers[HEADER] = carried


def _prerun(sender: Any = None, task_id: str | None = None, task: Any = None, **_: Any) -> None:
    client = _client or get_client()
    if client is None or not client.enabled or task_id is None or task is None:
        return
    request = task.request
    parent = deserialize_context(getattr(request, HEADER, None)) or {}
    ctx = TrailscopeContext(
        request_id=parent.get("request_id") or new_request_id(),
        trace_id=parent.get("trace_id") or new_trace_id(),
        span_id=new_span_id(),
        parent_span_id=parent.get("span_id"),
        user_id=parent.get("user_id"),
        correlation=dict(parent.get("correlation") or {}),
    )
    attributes = {"task_id": task_id}
    delivery = getattr(request, "delivery_info", None) or {}
    queue = delivery.get("routing_key") if isinstance(delivery, dict) else None
    if queue:
        attributes["queue"] = str(queue)
    _runs[task_id] = _Run(
        client,
        ctx,
        set_context(ctx),
        time.perf_counter(),
        int(time.time() * 1000),
        None,
        attributes,
    )


def _failure(
    sender: Any = None, task_id: str | None = None, exception: BaseException | None = None, **_: Any
) -> None:
    run = _runs.get(task_id or "")
    if run is not None and exception is not None:
        run.error = exception


def _retry(sender: Any = None, request: Any = None, reason: Any = None, **_: Any) -> None:
    run = _runs.get(getattr(request, "id", None) or "")
    if run is not None and run.error is None:
        run.error = reason if isinstance(reason, BaseException) else RuntimeError(str(reason))


def _postrun(
    sender: Any = None,
    task_id: str | None = None,
    task: Any = None,
    state: str | None = None,
    **_: Any,
) -> None:
    run = _runs.pop(task_id or "", None)
    if run is None:
        return
    name = getattr(task, "name", None) or "task"
    retries = getattr(getattr(task, "request", None), "retries", 0) or 0
    failed = state in ("FAILURE", "RETRY") or (state != "SUCCESS" and run.error is not None)
    if failed:
        verb = "failed (will retry)" if state == "RETRY" else "failed"
        message = f"{name} {verb}: {run.error}" if run.error else f"{name} {verb}"
    else:
        message = f"{name} completed"
    run.client.capture_event(
        name,
        kind="job",
        level=("warn" if state == "RETRY" else "error") if failed else "info",
        status="error" if failed else "ok",
        message=message,
        duration_ms=round((time.perf_counter() - run.t0) * 1000, 2),
        attempt=int(retries) + 1,
        attributes=run.attributes,
        error=run.error if failed else None,
        timestamp=run.timestamp,
        context=run.ctx,
    )
    reset_context(run.token)
