"""Request, trace and span IDs, and W3C `traceparent`."""

from __future__ import annotations

import re
import secrets
import uuid
from dataclasses import dataclass

_TRACEPARENT = re.compile(r"^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$")
_REQUEST_ID = re.compile(r"^[A-Za-z0-9._:-]{1,128}$")


def new_request_id() -> str:
    """32 hex characters, like the server's generated IDs."""
    return uuid.uuid4().hex


def new_trace_id() -> str:
    return secrets.token_hex(16)


def new_span_id() -> str:
    return secrets.token_hex(8)


@dataclass(frozen=True)
class TraceParent:
    trace_id: str
    parent_span_id: str
    sampled: bool


def parse_traceparent(header: str | None) -> TraceParent | None:
    """Parses a W3C `traceparent` header; None when it is missing or invalid."""
    if not header:
        return None
    m = _TRACEPARENT.match(header.strip().lower())
    if not m:
        return None
    version, trace_id, span_id, flags = m.groups()
    if version == "ff" or set(trace_id) == {"0"} or set(span_id) == {"0"}:
        return None
    return TraceParent(trace_id, span_id, (int(flags, 16) & 1) == 1)


def format_traceparent(trace_id: str, span_id: str, sampled: bool = True) -> str:
    return f"00-{trace_id}-{span_id}-{'01' if sampled else '00'}"


def accept_request_id(value: str | None) -> str | None:
    """A client-provided request ID, only if it looks sane."""
    if not value:
        return None
    v = value.strip()
    return v if _REQUEST_ID.match(v) else None
