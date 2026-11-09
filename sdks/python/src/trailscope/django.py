"""Django middleware (WSGI and ASGI).

Add it first in ``MIDDLEWARE`` and configure it in settings::

    MIDDLEWARE = ["trailscope.django.TrailscopeMiddleware", ...]
    TRAILSCOPE = {"service": "orders-api"}  # same keyword options as trailscope.Client

Without a ``TRAILSCOPE`` setting it uses the client from an earlier
``trailscope.init()``, or creates one from the ``TRAILSCOPE_*`` environment variables.
"""

from __future__ import annotations

import re
import time
from collections.abc import Awaitable, Callable
from typing import Any

from asgiref.sync import iscoroutinefunction, markcoroutinefunction

from ._capture import is_binary_content_type, should_capture_bodies
from ._context import TrailscopeContext, record_error, reset_context, set_context
from .client import Client, HttpExchange, get_client, init, log

_CONVERTER = re.compile(r"<(?:[^>:]+:)?([^>]+)>")
_NAMED_GROUP = re.compile(r"\(\?P<([^>]+)>[^)]*\)")
_CTX_ATTR = "_trailscope_context"


def route_template(pattern: str) -> str:
    """``projects/<int:project_id>/tasks`` -> ``/projects/:project_id/tasks``.

    Regex routes (``re_path``) keep their literal parts, with named groups as
    parameters.
    """
    p = _NAMED_GROUP.sub(r":\1", pattern)
    p = _CONVERTER.sub(r":\1", p).replace("^", "").replace("$", "")
    return "/" + p.lstrip("/")


_settings_client: Client | None = None


def _client_for_settings() -> Client:
    """The client configured by ``settings.TRAILSCOPE``, created once per process."""
    global _settings_client
    from django.conf import settings

    options = getattr(settings, "TRAILSCOPE", None)
    if isinstance(options, dict) and options:
        if _settings_client is None:
            _settings_client = init(**options)
        return _settings_client
    return get_client() or init()


def _user_id(request: Any) -> str | None:
    """The authenticated user, only if Django already loaded it (no extra query)."""
    user = getattr(request, "_cached_user", None) or getattr(request, "_acached_user", None)
    if user is not None and getattr(user, "is_authenticated", False):
        pk = getattr(user, "pk", None)
        return str(pk) if pk is not None else None
    return None


def _form_fields(request: Any) -> dict[str, Any] | None:
    """A multipart form's text fields, with files listed by name, size and type
    (never their content). Only if Django or DRF already parsed the form, so
    logging never triggers parsing a large upload."""
    post = getattr(request, "_post", None)
    files = getattr(request, "_files", None)
    out: dict[str, Any] = {}
    if post is not None:
        for key in post:
            values = post.getlist(key)
            out[key] = values[0] if len(values) == 1 else values
    if files is not None:
        for key in files:
            listed = [
                f"[file {f.name}, {f.size} bytes, {f.content_type}]" for f in files.getlist(key)
            ]
            out[key] = listed[0] if len(listed) == 1 else listed
    return out or None


def _request_body(request: Any, client: Client) -> bytes | dict[str, Any] | None:
    content_type = request.META.get("CONTENT_TYPE", "")
    if content_type.startswith("multipart/form-data"):
        return _form_fields(request)
    if is_binary_content_type(content_type):
        return None
    try:
        body = request.body
    except Exception:  # streamed already, or over DATA_UPLOAD_MAX_MEMORY_SIZE
        return None
    return bytes(body[: client.max_body_bytes + 1]) if body else None


def _response_body(response: Any, client: Client) -> bytes | None:
    if getattr(response, "streaming", False):
        return None
    try:
        content = response.content
    except Exception:
        return None
    return bytes(content[: client.max_body_bytes + 1]) if content else None


class TrailscopeMiddleware:
    sync_capable = True
    async_capable = True

    def __init__(self, get_response: Callable[[Any], Any]) -> None:
        self.get_response = get_response
        self.client = _client_for_settings()
        self._async = iscoroutinefunction(get_response)
        if self._async:
            markcoroutinefunction(self)

    def __call__(self, request: Any) -> Any:
        if self._async:
            return self.__acall__(request)
        if not self.client.enabled:
            return self.get_response(request)
        ctx, token, timestamp, t0 = self._start(request)
        try:
            response = self.get_response(request)
        except Exception as e:
            record_error(e, ctx)
            reset_context(token)
            raise
        self._finish(request, response, ctx, timestamp, t0)
        reset_context(token)
        return response

    async def __acall__(self, request: Any) -> Any:
        get_response: Callable[[Any], Awaitable[Any]] = self.get_response
        if not self.client.enabled:
            return await get_response(request)
        ctx, token, timestamp, t0 = self._start(request)
        try:
            response = await get_response(request)
        except Exception as e:
            record_error(e, ctx)
            reset_context(token)
            raise
        self._finish(request, response, ctx, timestamp, t0)
        reset_context(token)
        return response

    def process_exception(self, request: Any, exception: Exception) -> None:
        """Called by Django for exceptions raised in views, before the 500 response."""
        record_error(exception, getattr(request, _CTX_ATTR, None))

    def _start(self, request: Any) -> tuple[TrailscopeContext, Any, float, float]:
        headers = request.headers
        ctx = self.client.context_from_headers(lambda name: headers.get(name))
        setattr(request, _CTX_ATTR, ctx)
        return ctx, set_context(ctx), time.time() * 1000, time.perf_counter()

    def _finish(
        self, request: Any, response: Any, ctx: TrailscopeContext, timestamp: float, t0: float
    ) -> None:
        client = self.client
        try:
            if not response.has_header(client.request_id_header):
                response[client.request_id_header] = ctx.request_id
            match = getattr(request, "resolver_match", None)
            route = getattr(match, "route", None)
            if ctx.user_id is None:
                ctx.user_id = _user_id(request)
            meta = request.META
            xff = meta.get("HTTP_X_FORWARDED_FOR")
            ip = xff.split(",")[0].strip() if xff else meta.get("REMOTE_ADDR")
            res_headers = dict(response.headers.items())
            length = res_headers.get("Content-Length")
            duration_ms = (time.perf_counter() - t0) * 1000
            status = int(response.status_code)
            # Read bodies only when the capture policy keeps them.
            bodies = should_capture_bodies(
                client.capture_bodies, status, duration_ms, client.slow_ms
            )
            client.record_http(
                ctx,
                HttpExchange(
                    timestamp=timestamp,
                    duration_ms=duration_ms,
                    method=request.method or "GET",
                    url=request.get_full_path(),
                    status=status,
                    route=route_template(route) if isinstance(route, str) else None,
                    req_headers=dict(request.headers.items()),
                    res_headers=res_headers,
                    req_body=_request_body(request, client) if bodies else None,
                    res_body=_response_body(response, client) if bodies else None,
                    ip=ip or None,
                    req_size=int(meta["CONTENT_LENGTH"])
                    if str(meta.get("CONTENT_LENGTH", "")).isdigit()
                    else None,
                    res_size=int(length) if length and str(length).isdigit() else None,
                ),
            )
        except Exception:
            log.exception("trailscope: could not record request")
