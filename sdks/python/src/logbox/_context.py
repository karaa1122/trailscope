"""Request-scoped context carried through threads and async tasks with contextvars."""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from contextvars import ContextVar, Token
from dataclasses import dataclass, field
from typing import Any

Scalar = str | int | float | bool


@dataclass
class LogboxContext:
    request_id: str
    trace_id: str
    span_id: str
    parent_span_id: str | None = None
    user_id: str | None = None
    #: Journey keys, e.g. ``{"account_id": "acc_1"}``.
    correlation: dict[str, str] = field(default_factory=dict)
    attributes: dict[str, Scalar] = field(default_factory=dict)
    #: Route pattern, once the framework has matched one.
    route: str | None = None
    #: The exception that failed the request, recorded by an error hook.
    error: BaseException | None = None


_current: ContextVar[LogboxContext | None] = ContextVar("logbox_context", default=None)


def current_context() -> LogboxContext | None:
    return _current.get()


def set_context(ctx: LogboxContext | None) -> Token[LogboxContext | None]:
    return _current.set(ctx)


def reset_context(token: Token[LogboxContext | None]) -> None:
    try:
        _current.reset(token)
    except ValueError:
        # Reset from another context (e.g. a generator finalized elsewhere).
        _current.set(None)


@contextmanager
def use_context(ctx: LogboxContext) -> Iterator[LogboxContext]:
    """Runs a block inside ``ctx``."""
    token = _current.set(ctx)
    try:
        yield ctx
    finally:
        reset_context(token)


def set_user(user_id: str | int | None) -> None:
    """Attaches a user ID to the current request."""
    ctx = _current.get()
    if ctx is not None and user_id is not None and user_id != "":
        ctx.user_id = str(user_id)


def set_correlation(key: str, value: str | int) -> None:
    """Adds a correlation (journey) key to the current request and everything it triggers."""
    ctx = _current.get()
    if ctx is not None and key:
        ctx.correlation[key] = str(value)


def set_attribute(key: str, value: Scalar) -> None:
    """Adds a free-form attribute to the current request."""
    ctx = _current.get()
    if ctx is not None and key:
        ctx.attributes[key] = value


def record_error(error: BaseException, ctx: LogboxContext | None = None) -> None:
    """Attaches the exception that failed the request (kept when the response is a 5xx)."""
    ctx = ctx or _current.get()
    if ctx is not None and ctx.error is None:
        ctx.error = error


def serialize_context(ctx: LogboxContext | None = None) -> dict[str, Any] | None:
    """The part of a context that travels with a background job."""
    ctx = ctx or _current.get()
    if ctx is None:
        return None
    out: dict[str, Any] = {
        "requestId": ctx.request_id,
        "traceId": ctx.trace_id,
        "spanId": ctx.span_id,
    }
    if ctx.user_id:
        out["userId"] = ctx.user_id
    if ctx.correlation:
        out["correlation"] = dict(ctx.correlation)
    return out


def deserialize_context(value: object) -> dict[str, Any] | None:
    """Validates a serialized context (from job data or message headers)."""
    if not isinstance(value, dict):
        return None

    def s(key: str) -> str | None:
        v = value.get(key)
        return v if isinstance(v, str) and len(v) <= 128 else None

    correlation: dict[str, str] = {}
    raw = value.get("correlation")
    if isinstance(raw, dict):
        for k, v in raw.items():
            if isinstance(k, str) and isinstance(v, (str, int)):
                correlation[k] = str(v)
    return {
        "request_id": s("requestId"),
        "trace_id": s("traceId"),
        "span_id": s("spanId"),
        "user_id": s("userId"),
        "correlation": correlation,
    }
