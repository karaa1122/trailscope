"""ASGI middleware for Starlette, FastAPI and other ASGI apps.

FastAPI and Starlette::

    from trailscope.asgi import TrailscopeMiddleware

    app = FastAPI()
    app.add_middleware(TrailscopeMiddleware, service="orders-api")

Any ASGI app::

    app = TrailscopeMiddleware(app)
"""

from __future__ import annotations

import asyncio
import re
import time
from collections.abc import Awaitable, Callable, MutableMapping
from typing import Any

from ._context import TrailscopeContext, record_error, reset_context, set_context
from .client import Client, HttpExchange, get_client, init, log

Scope = MutableMapping[str, Any]
Message = MutableMapping[str, Any]
Receive = Callable[[], Awaitable[Message]]
Send = Callable[[Message], Awaitable[None]]
ASGIApp = Callable[[Scope, Receive, Send], Awaitable[None]]

_PARAM = re.compile(r"\{([^}:]+)(?::[^}]*)?\}")


def route_template(path: str) -> str:
    """``/items/{item_id:int}`` -> ``/items/:item_id``."""
    return _PARAM.sub(r":\1", path)


def _headers(scope: Scope) -> dict[str, str]:
    out: dict[str, str] = {}
    for k, v in scope.get("headers", []):
        name = k.decode("latin-1").lower()
        value = v.decode("latin-1")
        out[name] = f"{out[name]}, {value}" if name in out else value
    return out


def _client_ip(scope: Scope, headers: dict[str, str]) -> str | None:
    xff = headers.get("x-forwarded-for")
    if xff:
        return xff.split(",")[0].strip() or None
    client = scope.get("client")
    return str(client[0]) if client else None


def _route(scope: Scope, root_path: str) -> str | None:
    """The matched route template (Starlette and FastAPI set ``scope["route"]``)."""
    route = scope.get("route")
    path = getattr(route, "path_format", None) or getattr(route, "path", None)
    if not isinstance(path, str):
        return None
    # Mounted apps: the mount prefix is what routing added to root_path.
    current = scope.get("root_path", "")
    prefix = (
        current[len(root_path) :]
        if isinstance(current, str) and current.startswith(root_path)
        else ""
    )
    return route_template(prefix + path) or "/"


class TrailscopeMiddleware:
    """Records every HTTP request and runs the app inside its Trailscope context.

    Pass ``client``, or keyword options to create the process-wide client with
    :func:`trailscope.init`. Without either it uses the client from an earlier
    ``init()``, or creates one from the ``TRAILSCOPE_*`` environment variables.
    """

    def __init__(self, app: ASGIApp, client: Client | None = None, **options: Any) -> None:
        self.app = app
        if client is not None:
            self.client = client
        elif options:
            self.client = init(**options)
        else:
            self.client = get_client() or init()

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        client = self.client
        if scope["type"] != "http" or not client.enabled:
            await self.app(scope, receive, send)
            return
        headers = _headers(scope)
        ctx = client.context_from_headers(headers.get)
        token = set_context(ctx)
        timestamp = time.time() * 1000
        t0 = time.perf_counter()
        root_path = scope.get("root_path", "") or ""
        limit = client.max_body_bytes
        keep = client.capture_bodies != "never"
        req_body = bytearray()
        res_body = bytearray()
        state: dict[str, Any] = {"status": None, "headers": [], "size": 0}
        rid = client.request_id_header.encode("latin-1")

        async def receive_wrapper() -> Message:
            message = await receive()
            if keep and message["type"] == "http.request" and len(req_body) <= limit:
                req_body.extend(message.get("body", b"")[: limit + 1 - len(req_body)])
            return message

        async def send_wrapper(message: Message) -> None:
            if message["type"] == "http.response.start":
                hdrs = list(message.get("headers", []))
                if not any(k.lower() == rid for k, _ in hdrs):
                    hdrs.append((rid, ctx.request_id.encode("latin-1")))
                    message = {**message, "headers": hdrs}
                state["status"] = message["status"]
                state["headers"] = hdrs
            elif message["type"] == "http.response.body":
                body = message.get("body", b"")
                state["size"] += len(body)
                if keep and len(res_body) <= limit:
                    res_body.extend(body[: limit + 1 - len(res_body)])
            await send(message)

        status_if_unsent = 500
        try:
            await self.app(scope, receive_wrapper, send_wrapper)
        except asyncio.CancelledError:
            status_if_unsent = 499  # the client went away
            raise
        except Exception as e:
            record_error(e, ctx)
            raise
        finally:
            try:
                self._record(
                    scope,
                    headers,
                    ctx,
                    timestamp,
                    t0,
                    root_path,
                    state,
                    status_if_unsent,
                    req_body,
                    res_body,
                )
            except Exception:
                log.exception("trailscope: could not record request")
            reset_context(token)

    def _record(
        self,
        scope: Scope,
        headers: dict[str, str],
        ctx: TrailscopeContext,
        timestamp: float,
        t0: float,
        root_path: str,
        state: dict[str, Any],
        status_if_unsent: int,
        req_body: bytearray,
        res_body: bytearray,
    ) -> None:
        query = scope.get("query_string", b"").decode("latin-1")
        path = scope.get("path", "/")
        res_headers = {
            k.decode("latin-1").lower(): v.decode("latin-1") for k, v in state["headers"]
        }
        length = res_headers.get("content-length")
        self.client.record_http(
            ctx,
            HttpExchange(
                timestamp=timestamp,
                duration_ms=(time.perf_counter() - t0) * 1000,
                method=scope.get("method", "GET"),
                url=f"{path}?{query}" if query else path,
                status=state["status"] if state["status"] is not None else status_if_unsent,
                route=_route(scope, root_path),
                req_headers=headers,
                res_headers=res_headers,
                req_body=bytes(req_body) if req_body else None,
                res_body=bytes(res_body) if res_body else None,
                ip=_client_ip(scope, headers),
                req_size=int(headers["content-length"])
                if headers.get("content-length", "").isdigit()
                else None,
                res_size=int(length) if length and length.isdigit() else state["size"] or None,
            ),
        )
