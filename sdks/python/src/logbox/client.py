"""The Logbox client.

Every capture method is cheap and never blocks on the network: items go into a
bounded in-memory queue (the oldest are dropped when it is full) and a
background thread ships them as gzipped NDJSON. Redaction and serialization
happen in that thread, off the request path.
"""

from __future__ import annotations

import atexit
import functools
import gzip
import inspect
import json
import logging
import os
import random
import threading
import time
import traceback
import urllib.error
import urllib.request
import weakref
from collections import deque
from collections.abc import Awaitable, Callable, Iterator, Mapping
from contextlib import contextmanager
from dataclasses import dataclass
from re import Pattern
from types import TracebackType
from typing import Any, Literal, TypeVar, cast

from ._capture import BodyCapture, flatten_headers, serialize_body, should_capture_bodies, split_url
from ._context import LogboxContext, Scalar, current_context, reset_context, set_context
from ._ids import accept_request_id, new_request_id, new_span_id, new_trace_id, parse_traceparent
from ._redact import Redactor
from ._version import __version__

log = logging.getLogger("logbox")

Item = dict[str, Any]
Level = Literal["debug", "info", "warn", "error"]
SpanKind = Literal["http", "db", "cache", "queue", "fn", "external"]
F = TypeVar("F", bound=Callable[..., Any])

DEFAULT_IGNORE_ROUTES: tuple[str, ...] = ("/health", "/healthz", "/ready", "/readyz", "/livez")
_CONFIG_REFRESH_SECONDS = 300.0
_USER_AGENT = f"logbox-python/{__version__}"


def _env(name: str) -> str | None:
    v = os.environ.get(name)
    return v if v else None


def _now_ms() -> int:
    return int(time.time() * 1000)


def _clamp(rate: float) -> float:
    return min(1.0, max(0.0, float(rate)))


def error_fields(error: BaseException) -> dict[str, str]:
    """``errorType``, ``errorMessage`` and ``errorStack`` for the wire format."""
    return {
        "errorType": type(error).__name__,
        "errorMessage": str(error),
        "errorStack": "".join(traceback.format_exception(type(error), error, error.__traceback__)),
    }


@dataclass
class HttpExchange:
    """One finished HTTP exchange, as framework integrations report it."""

    #: Epoch milliseconds when the request started.
    timestamp: float
    duration_ms: float
    method: str
    #: Path with the query string, e.g. ``/orders/42?expand=items``.
    url: str
    status: int
    #: Route pattern, e.g. ``/orders/:id``. Falls back to the context's route.
    route: str | None = None
    req_headers: Mapping[str, str] | None = None
    res_headers: Mapping[str, str] | None = None
    req_body: Any = None
    res_body: Any = None
    ip: str | None = None
    req_size: int | None = None
    res_size: int | None = None
    #: Falls back to the context's error.
    error: BaseException | None = None


@dataclass
class ClientStats:
    queued: int
    dropped: int
    sent: int
    rejected: int
    failed_batches: int


class Client:
    """Ships requests, spans, job runs and log lines to Logbox.

    Every option falls back to an environment variable where one exists:
    ``LOGBOX_ENDPOINT``, ``LOGBOX_INGEST_KEY``, ``LOGBOX_SERVICE``,
    ``LOGBOX_ENVIRONMENT``, ``LOGBOX_RELEASE``, ``LOGBOX_ENABLED=0`` and
    ``LOGBOX_DEBUG=1``.
    """

    def __init__(
        self,
        *,
        endpoint: str | None = None,
        ingest_key: str | None = None,
        service: str | None = None,
        environment: str | None = None,
        release: str | None = None,
        enabled: bool | None = None,
        max_queue_size: int = 10_000,
        flush_interval: float = 1.0,
        flush_at: int = 500,
        max_batch_size: int = 1000,
        request_timeout: float = 10.0,
        shutdown_timeout: float = 5.0,
        sample_rate: float | None = None,
        route_sample_rates: Mapping[str, float] | None = None,
        slow_ms: float | None = None,
        capture_bodies: BodyCapture | None = None,
        max_body_bytes: int | None = None,
        ignore_routes: list[str | Pattern[str]] | None = None,
        request_id_header: str = "x-request-id",
        redact_headers: list[str] | None = None,
        redact_keys: list[str] | None = None,
        redact_patterns: list[str | Pattern[str]] | None = None,
        mask_ip: bool | None = None,
        disable_default_redaction: bool = False,
        remote_config: bool = True,
        before_send: Callable[[Item], Item | None] | None = None,
        debug: bool | None = None,
    ) -> None:
        self.endpoint = (endpoint or _env("LOGBOX_ENDPOINT") or "http://localhost:8080").rstrip("/")
        self.ingest_key = ingest_key or _env("LOGBOX_INGEST_KEY") or ""
        self.service = service or _env("LOGBOX_SERVICE") or "unknown-service"
        self.environment = environment or _env("LOGBOX_ENVIRONMENT") or "production"
        self.release = release or _env("LOGBOX_RELEASE") or ""
        self.enabled = enabled if enabled is not None else _env("LOGBOX_ENABLED") != "0"
        self.max_queue_size = max(1, max_queue_size)
        self.flush_interval = max(0.01, flush_interval)
        self.flush_at = max(1, flush_at)
        self.max_batch_size = min(10_000, max(1, max_batch_size))
        self.request_timeout = request_timeout
        self.shutdown_timeout = shutdown_timeout
        self.sample_rate = _clamp(sample_rate) if sample_rate is not None else 1.0
        self.route_sample_rates = {k: _clamp(v) for k, v in (route_sample_rates or {}).items()}
        self.slow_ms = slow_ms if slow_ms is not None else 1000.0
        self.capture_bodies: BodyCapture = capture_bodies or "errors"
        self.max_body_bytes = max_body_bytes if max_body_bytes is not None else 8192
        self.ignore_routes: list[str | Pattern[str]] = (
            list(ignore_routes) if ignore_routes is not None else list(DEFAULT_IGNORE_ROUTES)
        )
        self.request_id_header = request_id_header.lower()
        self.remote_config = remote_config
        self.before_send = before_send
        self._redact_args: dict[str, Any] = {
            "headers": redact_headers or [],
            "keys": redact_keys or [],
            "patterns": redact_patterns or [],
            "mask_ip": mask_ip if mask_ip is not None else True,
            "disable_defaults": disable_default_redaction,
        }
        self.redactor = Redactor(**self._redact_args)
        # Options the application set itself win over /v1/config.
        self._explicit = {
            name
            for name, value in (
                ("sample_rate", sample_rate),
                ("route_sample_rates", route_sample_rates),
                ("slow_ms", slow_ms),
                ("capture_bodies", capture_bodies),
                ("max_body_bytes", max_body_bytes),
                ("ignore_routes", ignore_routes),
                ("mask_ip", mask_ip),
            )
            if value is not None
        }
        if debug is None:
            debug = _env("LOGBOX_DEBUG") == "1"
        if debug:
            log.setLevel(logging.DEBUG)
            if not log.handlers:
                log.addHandler(logging.StreamHandler())

        self._queue: deque[Item] = deque()
        self._dropped = 0
        self._sent = 0
        self._rejected = 0
        self._failed_batches = 0
        self._failures = 0
        self._backoff_until = 0.0
        self._warned_auth = False
        self._closed = False
        self._stopping = False
        self._thread: threading.Thread | None = None
        self._lock = threading.Lock()
        self._send_lock = threading.Lock()
        self._wake = threading.Event()
        self._pid = os.getpid()
        if self.enabled and not self.ingest_key:
            log.debug("no ingest key configured (LOGBOX_INGEST_KEY); events will not be sent")
        if hasattr(os, "register_at_fork"):
            ref = weakref.WeakMethod(self._after_fork)

            def after_fork() -> None:
                method = ref()
                if method is not None:
                    method()

            os.register_at_fork(after_in_child=after_fork)
        # Start now so /v1/config is loaded before the first request; after a
        # fork the thread is restarted by the first capture.
        if self.enabled and self.ingest_key:
            with self._lock:
                self._start_worker()

    # ---- capture ----------------------------------------------------------------

    def capture_request(self, item: Mapping[str, Any]) -> None:
        """Queues a request in the wire format (camelCase keys, see docs/ingest-protocol.md)."""
        self._enqueue({**item, "type": "request", "service": item.get("service") or self.service})

    def capture_event(
        self,
        name: str,
        *,
        kind: Literal["job", "log", "custom"] = "custom",
        level: Level | None = None,
        message: str | None = None,
        status: Literal["ok", "warn", "error"] | None = None,
        duration_ms: float | None = None,
        attempt: int | None = None,
        attributes: Mapping[str, Scalar] | None = None,
        error: BaseException | None = None,
        timestamp: float | None = None,
        context: LogboxContext | None = None,
    ) -> None:
        """Records an event; IDs, user and correlation come from the current context."""
        ctx = context or current_context()
        item: Item = {
            "type": "event",
            "service": self.service,
            "kind": kind,
            "name": name,
            "level": level,
            "message": message,
            "status": status,
            "durationMs": duration_ms,
            "attempt": attempt,
            "attributes": dict(attributes) if attributes else None,
            "timestamp": timestamp,
        }
        if ctx is not None:
            item["requestId"] = ctx.request_id
            item["traceId"] = ctx.trace_id
            item["userId"] = ctx.user_id
            item["correlation"] = dict(ctx.correlation) if ctx.correlation else None
        if error is not None:
            item.update(error_fields(error))
        self._enqueue(item)

    def log(
        self,
        level: Level,
        message: str,
        *,
        name: str = "log",
        attributes: Mapping[str, Scalar] | None = None,
        error: BaseException | None = None,
    ) -> None:
        """An application log line, linked to the current request if there is one."""
        self.capture_event(
            name, kind="log", level=level, message=message, attributes=attributes, error=error
        )

    def capture_exception(
        self,
        error: BaseException,
        *,
        name: str = "exception",
        attributes: Mapping[str, Scalar] | None = None,
    ) -> None:
        """Reports an exception as an error event (outside an HTTP request)."""
        self.capture_event(
            name,
            kind="log",
            level="error",
            status="error",
            message=str(error),
            attributes=attributes,
            error=error,
        )

    @contextmanager
    def span(
        self, kind: SpanKind, name: str, attributes: Mapping[str, Scalar] | None = None
    ) -> Iterator[None]:
        """Times a block as a span of the current request."""
        ctx = current_context()
        start = time.time() * 1000
        t0 = time.perf_counter()
        status = "ok"
        try:
            yield
        except BaseException:
            status = "error"
            raise
        finally:
            self._enqueue(
                {
                    "type": "span",
                    "service": self.service,
                    "traceId": ctx.trace_id if ctx else new_trace_id(),
                    "spanId": new_span_id(),
                    "parentSpanId": ctx.span_id if ctx else None,
                    "requestId": ctx.request_id if ctx else None,
                    "kind": kind,
                    "name": name,
                    "start": round(start, 3),
                    "durationMs": round((time.perf_counter() - t0) * 1000, 2),
                    "status": status,
                    "attributes": dict(attributes) if attributes else None,
                }
            )

    def track_job(
        self,
        name: str,
        *,
        attempt: int | None = None,
        context: LogboxContext | Mapping[str, Any] | None = None,
        attributes: Mapping[str, Scalar] | None = None,
    ) -> JobScope:
        """Runs a background job inside a context and records it as a ``kind=job`` event.

        Use it as a context manager or a decorator (sync or async)::

            with client.track_job("report.build", attempt=1):
                build()

        ``context`` restores a serialized request context (see
        ``serialize_context``); by default a job started inside a request
        belongs to that request.
        """
        return JobScope(self, name, attempt, context, attributes)

    # ---- helpers for framework integrations ---------------------------------------

    def context_from_headers(self, get: Callable[[str], str | None]) -> LogboxContext:
        """Builds the context of an incoming request from its headers."""
        tp = parse_traceparent(get("traceparent"))
        return LogboxContext(
            request_id=accept_request_id(get(self.request_id_header)) or new_request_id(),
            trace_id=tp.trace_id if tp else new_trace_id(),
            span_id=new_span_id(),
            parent_span_id=tp.parent_span_id if tp else None,
        )

    def is_ignored(self, route_or_path: str | None) -> bool:
        if not route_or_path:
            return False
        return any(
            r == route_or_path if isinstance(r, str) else r.search(route_or_path) is not None
            for r in self.ignore_routes
        )

    def sample(self, route: str, status: int, duration_ms: float, has_error: bool = False) -> float:
        """The sample rate to record when the request is kept, or 0 when it is dropped.

        Errors (status >= 400) and slow requests are always kept.
        """
        if status >= 400 or has_error or duration_ms >= self.slow_ms:
            return 1.0
        rate = self.route_sample_rates.get(route, self.sample_rate)
        if rate >= 1:
            return 1.0
        return rate if random.random() < rate else 0.0

    def record_http(self, ctx: LogboxContext, ex: HttpExchange) -> None:
        """Records a finished HTTP exchange.

        Applies the ignore list, sampling and the body capture policy. Error
        details are kept only for 5xx responses; a handled 4xx is an expected
        outcome.
        """
        if not self.enabled or self._closed:
            return
        path, query = split_url(ex.url)
        route = ex.route or ctx.route
        if self.is_ignored(route) or self.is_ignored(path):
            return
        status = ex.status
        duration_ms = round(ex.duration_ms, 2)
        failed = (ex.error or ctx.error) if status >= 500 else None
        rate = self.sample(route or path, status, duration_ms, failed is not None)
        if rate == 0:
            return
        req_headers = flatten_headers(ex.req_headers)
        res_headers = flatten_headers(ex.res_headers)
        item: Item = {
            "type": "request",
            "service": self.service,
            "timestamp": int(ex.timestamp),
            "requestId": ctx.request_id,
            "traceId": ctx.trace_id,
            "spanId": ctx.span_id,
            "method": ex.method.upper(),
            "route": route,
            "path": path,
            "query": query or None,
            "status": status,
            "durationMs": duration_ms,
            "reqSize": ex.req_size,
            "resSize": ex.res_size,
            "userId": ctx.user_id,
            "ip": ex.ip,
            "userAgent": req_headers.get("user-agent"),
            "reqHeaders": req_headers,
            "resHeaders": res_headers,
            "attributes": dict(ctx.attributes) if ctx.attributes else None,
            "correlation": dict(ctx.correlation) if ctx.correlation else None,
            "sampleRate": rate if rate < 1 else None,
        }
        if should_capture_bodies(self.capture_bodies, status, duration_ms, self.slow_ms):
            limit = self.max_body_bytes
            rq = serialize_body(ex.req_body, req_headers.get("content-type"), limit)
            encoding = res_headers.get("content-encoding")
            rs = (
                None
                if encoding and encoding != "identity"
                else serialize_body(ex.res_body, res_headers.get("content-type"), limit)
            )
            item["reqBody"] = rq[0] if rq else None
            item["resBody"] = rs[0] if rs else None
            if (rq and rq[1]) or (rs and rs[1]):
                item["bodyTruncated"] = True
        if failed is not None:
            item.update(error_fields(failed))
        self._enqueue(item)

    # ---- lifecycle --------------------------------------------------------------

    def stats(self) -> ClientStats:
        return ClientStats(
            len(self._queue), self._dropped, self._sent, self._rejected, self._failed_batches
        )

    def flush(self) -> None:
        """Sends everything queued, in the calling thread."""
        if self.enabled:
            self._drain()

    def shutdown(self, timeout: float | None = None) -> None:
        """Stops the background thread after a final flush, waiting at most ``timeout`` seconds."""
        if self._closed:
            return
        self._closed = True
        thread = self._thread
        if thread is None or not thread.is_alive():
            return
        self._backoff_until = 0.0
        self._stopping = True
        self._wake.set()
        thread.join(self.shutdown_timeout if timeout is None else timeout)

    def refresh_config(self) -> None:
        """Fetches the project's SDK settings from ``/v1/config`` and applies them."""
        req = urllib.request.Request(
            f"{self.endpoint}/v1/config",
            headers={"authorization": f"Bearer {self.ingest_key}", "user-agent": _USER_AGENT},
        )
        try:
            with urllib.request.urlopen(req, timeout=self.request_timeout) as resp:
                config = json.loads(resp.read())
        except (urllib.error.URLError, OSError, ValueError) as e:
            log.debug("could not load /v1/config: %s", e)
            return
        if isinstance(config, dict):
            self.apply_remote_config(config)

    def apply_remote_config(self, c: Mapping[str, Any]) -> None:
        """Applies project settings to every option the application didn't set itself."""

        def num(v: object) -> bool:
            return isinstance(v, (int, float)) and not isinstance(v, bool)

        set_ = self._explicit
        if "sample_rate" not in set_ and num(c.get("sampleRate")):
            self.sample_rate = _clamp(c["sampleRate"])
        rates = c.get("routeSampleRates")
        if "route_sample_rates" not in set_ and isinstance(rates, dict):
            self.route_sample_rates = {str(k): _clamp(v) for k, v in rates.items() if num(v)}
        if "slow_ms" not in set_ and num(c.get("slowMs")) and c["slowMs"] > 0:
            self.slow_ms = float(c["slowMs"])
        if "capture_bodies" not in set_ and c.get("bodyCapture") in ("errors", "always", "never"):
            self.capture_bodies = c["bodyCapture"]
        if "max_body_bytes" not in set_ and num(c.get("maxBodyBytes")) and c["maxBodyBytes"] >= 0:
            self.max_body_bytes = int(c["maxBodyBytes"])
        routes = c.get("ignoreRoutes")
        if "ignore_routes" not in set_ and isinstance(routes, list):
            self.ignore_routes = [r for r in routes if isinstance(r, str)]
        mask = c.get("maskIp")
        if "mask_ip" not in set_ and isinstance(mask, bool) and mask != self.redactor.mask_ip:
            self._redact_args["mask_ip"] = mask
            self.redactor = Redactor(**self._redact_args)

    # ---- internals --------------------------------------------------------------

    def _enqueue(self, item: Item) -> None:
        if not self.enabled or self._closed:
            return
        if item["type"] != "span":
            if not item.get("environment"):
                item["environment"] = self.environment
            if self.release and not item.get("release"):
                item["release"] = self.release
            if item.get("timestamp") is None:
                item["timestamp"] = _now_ms()
        with self._lock:
            if len(self._queue) >= self.max_queue_size:
                self._queue.popleft()
                self._dropped += 1
            self._queue.append(item)
            size = len(self._queue)
            if self._thread is None:
                self._start_worker()
        if size >= self.flush_at:
            self._wake.set()

    def _start_worker(self) -> None:
        """Starts the flush thread; the caller holds ``self._lock``."""
        if not self.enabled or self._thread is not None:
            return
        self._thread = threading.Thread(target=self._run, name="logbox-flush", daemon=True)
        self._thread.start()

    def _after_fork(self) -> None:
        # The parent ships what it queued; the child starts empty with fresh locks.
        self._lock = threading.Lock()
        self._send_lock = threading.Lock()
        self._wake = threading.Event()
        self._queue = deque()
        self._thread = None
        self._pid = os.getpid()

    def _run(self) -> None:
        next_config = 0.0
        while True:
            if self.remote_config and self.ingest_key and time.monotonic() >= next_config:
                self.refresh_config()
                next_config = time.monotonic() + _CONFIG_REFRESH_SECONDS
            self._wake.wait(self.flush_interval)
            self._wake.clear()
            try:
                self._drain()
            except Exception:  # never let the flush thread die
                log.exception("logbox flush failed")
            if self._stopping:
                return

    def _take(self, n: int) -> list[Item]:
        with self._lock:
            count = min(n, len(self._queue))
            return [self._queue.popleft() for _ in range(count)]

    def _put_back(self, batch: list[Item]) -> None:
        """Puts a failed batch back at the front, as far as room allows (keeping the newest)."""
        with self._lock:
            room = self.max_queue_size - len(self._queue)
            keep = batch[max(0, len(batch) - room) :]
            self._dropped += len(batch) - len(keep)
            self._queue.extendleft(reversed(keep))

    def _drain(self) -> None:
        with self._send_lock:
            while True:
                if not self.ingest_key:
                    with self._lock:
                        self._queue.clear()
                    return
                if time.monotonic() < self._backoff_until:
                    return
                batch = self._take(self.max_batch_size)
                if not batch:
                    return
                prepared = self._prepare(batch)
                if not prepared:
                    continue
                result, retry_after = self._send(prepared)
                if result == "ok":
                    self._failures = 0
                    continue
                if result == "drop":
                    continue
                self._failed_batches += 1
                self._failures += 1
                self._put_back(batch)
                backoff = min(60.0, 2.0 ** (self._failures - 1))
                jittered = backoff / 2 + random.random() * backoff / 2
                self._backoff_until = time.monotonic() + max(retry_after or 0.0, jittered)
                log.debug(
                    "flush failed; retrying in %.1f s", self._backoff_until - time.monotonic()
                )
                return

    def _prepare(self, batch: list[Item]) -> list[Item]:
        """Redaction, ``before_send`` and removal of empty fields, off the request path."""
        r = self.redactor
        out: list[Item] = []
        for original in batch:
            item: Item = {k: v for k, v in original.items() if v is not None}
            if item["type"] == "request":
                ct = item.get("reqHeaders", {}).get("content-type", "")
                res_ct = item.get("resHeaders", {}).get("content-type", "")
                if "reqHeaders" in item:
                    item["reqHeaders"] = r.redact_headers(item["reqHeaders"])
                if "resHeaders" in item:
                    item["resHeaders"] = r.redact_headers(item["resHeaders"])
                if "reqBody" in item:
                    item["reqBody"] = r.redact_body(item["reqBody"], ct)
                if "resBody" in item:
                    item["resBody"] = r.redact_body(item["resBody"], res_ct)
                if "query" in item:
                    item["query"] = r.redact_query(item["query"])
                if "ip" in item:
                    item["ip"] = r.ip(item["ip"])
                if "errorMessage" in item:
                    item["errorMessage"] = r.apply_patterns(item["errorMessage"])
            elif item["type"] == "event":
                for key in ("message", "errorMessage"):
                    if key in item:
                        item[key] = r.apply_patterns(item[key])
            result: Item | None = item
            if self.before_send is not None:
                try:
                    result = self.before_send(item)
                except Exception as e:
                    log.debug("before_send raised: %s", e)
            if result:
                out.append(result)
        return out

    def _send(self, items: list[Item]) -> tuple[Literal["ok", "retry", "drop"], float | None]:
        try:
            lines = "\n".join(
                json.dumps(i, ensure_ascii=False, default=str, separators=(",", ":")) for i in items
            )
            body = gzip.compress(lines.encode("utf-8"), compresslevel=6)
        except (TypeError, ValueError) as e:
            log.debug("could not serialize batch: %s", e)
            return "drop", None
        req = urllib.request.Request(
            f"{self.endpoint}/v1/ingest",
            data=body,
            method="POST",
            headers={
                "authorization": f"Bearer {self.ingest_key}",
                "content-type": "application/x-ndjson",
                "content-encoding": "gzip",
                "user-agent": _USER_AGENT,
            },
        )
        retry_after: str | None = None
        payload = b""
        try:
            with urllib.request.urlopen(req, timeout=self.request_timeout) as resp:
                status = resp.status
                payload = resp.read()
        except urllib.error.HTTPError as e:
            status = e.code
            retry_after = e.headers.get("retry-after") if e.headers else None
            e.close()
        except (urllib.error.URLError, OSError, ValueError) as e:
            log.debug("network error: %s", e)
            return "retry", None
        if status in (200, 207):
            try:
                parsed = json.loads(payload) if payload else {}
            except ValueError:
                parsed = {}
            self._sent += int(parsed.get("accepted", len(items)))
            rejected = int(parsed.get("rejected", 0))
            if rejected:
                self._rejected += rejected
                errors = parsed.get("errors") or [{}]
                log.debug("server rejected %d items: %s", rejected, errors[0].get("message", ""))
            return "ok", None
        if status == 429 or status >= 500:
            try:
                after = float(retry_after) if retry_after else None
            except ValueError:
                after = None
            return "retry", after
        if status in (401, 403) and not self._warned_auth:
            self._warned_auth = True
            log.warning("logbox: ingest key rejected (HTTP %d); check LOGBOX_INGEST_KEY", status)
        self._rejected += len(items)
        log.debug("batch dropped: HTTP %d", status)
        return "drop", None


class JobScope:
    """A job run: a context manager and a decorator for sync and async functions."""

    def __init__(
        self,
        client: Client,
        name: str,
        attempt: int | None,
        context: LogboxContext | Mapping[str, Any] | None,
        attributes: Mapping[str, Scalar] | None,
    ) -> None:
        self._client = client
        self._name = name
        self._attempt = attempt
        self._context = context
        self._attributes = attributes
        self._token: Any = None
        self._ctx: LogboxContext | None = None
        self._t0 = 0.0
        self._timestamp = 0

    def _fresh(self) -> JobScope:
        return JobScope(self._client, self._name, self._attempt, self._context, self._attributes)

    def __enter__(self) -> LogboxContext:
        parent: LogboxContext | Mapping[str, Any] | None = self._context or current_context()
        if isinstance(parent, LogboxContext):
            p: Mapping[str, Any] = {
                "request_id": parent.request_id,
                "trace_id": parent.trace_id,
                "span_id": parent.span_id,
                "user_id": parent.user_id,
                "correlation": parent.correlation,
            }
        else:
            p = parent or {}
        self._ctx = LogboxContext(
            request_id=p.get("request_id") or new_request_id(),
            trace_id=p.get("trace_id") or new_trace_id(),
            span_id=new_span_id(),
            parent_span_id=p.get("span_id"),
            user_id=p.get("user_id"),
            correlation=dict(p.get("correlation") or {}),
        )
        self._token = set_context(self._ctx)
        self._timestamp = _now_ms()
        self._t0 = time.perf_counter()
        return self._ctx

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> Literal[False]:
        duration = round((time.perf_counter() - self._t0) * 1000, 2)
        failed = exc is not None and not isinstance(
            exc, (GeneratorExit, KeyboardInterrupt, SystemExit)
        )
        self._client.capture_event(
            self._name,
            kind="job",
            level="error" if failed else "info",
            status="error" if failed else "ok",
            message=f"{self._name} failed: {exc}" if failed else f"{self._name} completed",
            duration_ms=duration,
            attempt=self._attempt,
            attributes=self._attributes,
            error=exc if failed else None,
            timestamp=self._timestamp,
            context=self._ctx,
        )
        if self._token is not None:
            reset_context(self._token)
        return False

    def __call__(self, fn: F) -> F:
        if inspect.iscoroutinefunction(fn):
            afn = cast(Callable[..., Awaitable[Any]], fn)

            @functools.wraps(fn)
            async def async_wrapper(*args: Any, **kwargs: Any) -> Any:
                with self._fresh():
                    return await afn(*args, **kwargs)

            return cast(F, async_wrapper)

        @functools.wraps(fn)
        def wrapper(*args: Any, **kwargs: Any) -> Any:
            with self._fresh():
                return fn(*args, **kwargs)

        return cast(F, wrapper)


_global: Client | None = None
_global_lock = threading.Lock()


def init(**options: Any) -> Client:
    """Creates the process-wide client used by the framework integrations.

    Takes the same keyword arguments as :class:`Client`. A previous client is
    shut down. The client flushes when the interpreter exits.
    """
    global _global
    with _global_lock:
        previous = _global
        _global = Client(**options)
        client = _global
    if previous is not None:
        previous.shutdown()
    atexit.register(client.shutdown)
    return client


def get_client() -> Client | None:
    return _global
