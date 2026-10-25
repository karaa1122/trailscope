"""Body capture policy, serialization and truncation."""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Any, Literal

BodyCapture = Literal["errors", "always", "never"]

_BINARY_TYPES = (
    "multipart/",
    "application/octet-stream",
    "image/",
    "audio/",
    "video/",
    "font/",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-protobuf",
    "application/grpc",
)


def is_binary_content_type(content_type: str | None) -> bool:
    """Whether a body with this content type should never be captured."""
    if not content_type:
        return False
    return content_type.lower().startswith(_BINARY_TYPES)


def should_capture_bodies(
    policy: BodyCapture, status: int, duration_ms: float, slow_ms: float
) -> bool:
    if policy == "never":
        return False
    if policy == "always":
        return True
    return status >= 400 or duration_ms >= slow_ms


def truncate(text: str, limit: int) -> tuple[str, bool]:
    data = text.encode("utf-8")
    if len(data) <= limit:
        return text, False
    # Cut at a byte boundary and drop a trailing partial character.
    return data[:limit].decode("utf-8", errors="ignore"), True


def serialize_body(value: Any, content_type: str | None, limit: int) -> tuple[str, bool] | None:
    """Serializes a body-like value and truncates it to ``limit`` bytes.

    Returns None for binary or empty bodies.
    """
    if value is None or is_binary_content_type(content_type):
        return None
    if isinstance(value, (bytes, bytearray, memoryview)):
        raw = bytes(value[: limit + 4])
        if b"\x00" in raw[:512]:
            return None
        text = raw.decode("utf-8", errors="replace")
        if len(value) > limit:
            return truncate(text, limit)[0], True
    elif isinstance(value, str):
        text = value
    else:
        try:
            text = json.dumps(value, ensure_ascii=False, default=str, separators=(",", ":"))
        except (TypeError, ValueError):
            return None
    if not text:
        return None
    return truncate(text, limit)


def split_url(url: str) -> tuple[str, str]:
    """``/a/b?x=1`` -> (``/a/b``, ``x=1``)."""
    path, _, query = url.partition("?")
    return path or "/", query


def flatten_headers(
    headers: Mapping[str, Any] | Iterable[tuple[str, Any]] | None,
) -> dict[str, str]:
    """Lower-cased names; repeated headers joined with ``, ``."""
    out: dict[str, str] = {}
    if headers is None:
        return out
    items = headers.items() if isinstance(headers, Mapping) else headers
    for k, v in items:
        if v is None:
            continue
        name = (k.decode("latin-1") if isinstance(k, bytes) else str(k)).lower()
        val = (
            v.decode("latin-1")
            if isinstance(v, bytes)
            else ", ".join(v)
            if isinstance(v, list)
            else str(v)
        )
        out[name] = f"{out[name]}, {val}" if name in out else val
    return out
