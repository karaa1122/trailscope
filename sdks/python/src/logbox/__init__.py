"""Logbox SDK for Python: non-blocking request logging for Django, FastAPI/Starlette and Celery."""

from ._context import (
    LogboxContext,
    current_context,
    deserialize_context,
    record_error,
    serialize_context,
    set_attribute,
    set_correlation,
    set_user,
    use_context,
)
from ._ids import format_traceparent, new_request_id, new_span_id, new_trace_id, parse_traceparent
from ._redact import REDACTED, Redactor, mask_ip
from ._version import __version__
from .client import (
    DEFAULT_IGNORE_ROUTES,
    Client,
    ClientStats,
    HttpExchange,
    JobScope,
    get_client,
    init,
)
from .handler import LogboxHandler

__all__ = [
    "DEFAULT_IGNORE_ROUTES",
    "REDACTED",
    "Client",
    "ClientStats",
    "HttpExchange",
    "JobScope",
    "LogboxContext",
    "LogboxHandler",
    "Redactor",
    "__version__",
    "current_context",
    "deserialize_context",
    "format_traceparent",
    "get_client",
    "init",
    "mask_ip",
    "new_request_id",
    "new_span_id",
    "new_trace_id",
    "parse_traceparent",
    "record_error",
    "serialize_context",
    "set_attribute",
    "set_correlation",
    "set_user",
    "use_context",
]
