"""Measures what the ASGI and Django middleware add to the request path.

Context setup, request ID header, body buffering and recording the request
(redaction, serialization and gzip happen later in the flush thread). Synthetic
requests keep network noise out. Run: python bench/overhead.py
"""

from __future__ import annotations

import asyncio
import json
import statistics
import sys
import time
from collections.abc import Awaitable, Callable
from typing import Any

import logbox
from logbox.asgi import LogboxMiddleware as AsgiMiddleware

N = 50_000
client = logbox.Client(
    ingest_key="lbx_ingest_bench",
    service="bench",
    endpoint="http://127.0.0.1:9",  # never reached: flushing is paused below
    flush_interval=3600,
    flush_at=10**9,
    max_queue_size=N * 3,
    remote_config=False,
)
PAYLOAD = json.dumps([{"id": "t1", "title": "Write the brief", "done": False}]).encode()
HEADERS = [
    (b"host", b"api.example.com"),
    (b"content-type", b"application/json"),
    (b"authorization", b"Bearer eyJhbGciOiJIUzI1NiJ9.bench"),
    (b"user-agent", b"Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15"),
    (b"content-length", b"64"),
]


class Route:
    path_format = "/projects/{project_id}/tasks"


async def app(
    scope: dict[str, Any],
    receive: Callable[[], Awaitable[Any]],
    send: Callable[[Any], Awaitable[None]],
) -> None:
    await receive()
    scope["route"] = Route()
    status = 500 if scope["path"].endswith("/fail") else 200
    await send(
        {
            "type": "http.response.start",
            "status": status,
            "headers": [(b"content-type", b"application/json")],
        }
    )
    await send({"type": "http.response.body", "body": PAYLOAD})


def pct(samples: list[float], p: float) -> float:
    return samples[min(len(samples) - 1, int(len(samples) * p))] * 1000


async def bench_asgi(wrapped: bool) -> list[float]:
    target = AsgiMiddleware(app, client=client) if wrapped else app
    body = {
        "type": "http.request",
        "body": b'{"title":"Draft roadmap","tags":["a","b"]}',
        "more_body": False,
    }

    async def receive() -> dict[str, Any]:
        return body

    async def send(_: Any) -> None:
        return None

    samples = []
    for i in range(N):
        scope = {
            "type": "http",
            "method": "POST",
            "path": f"/projects/prj_{i % 100}/tasks" + ("/fail" if i % 50 == 0 else ""),
            "query_string": b"expand=owner",
            "headers": HEADERS,
            "client": ("198.51.100.7", 50000),
            "root_path": "",
        }
        t = time.perf_counter()
        await target(scope, receive, send)
        samples.append(time.perf_counter() - t)
    return sorted(samples)


def bench_django(wrapped: bool) -> list[float]:
    from django.conf import settings

    if not settings.configured:
        settings.configure(ALLOWED_HOSTS=["*"], USE_TZ=True)
    import django

    django.setup()
    from django.http import HttpResponse
    from django.test import RequestFactory

    import logbox.django as lbd

    def view(request: Any) -> HttpResponse:
        status = 500 if request.path.endswith("/fail") else 200
        return HttpResponse(PAYLOAD, status=status, content_type="application/json")

    logbox.init(  # the Django middleware uses the process-wide client
        ingest_key="lbx_ingest_bench",
        service="bench",
        endpoint="http://127.0.0.1:9",
        flush_interval=3600,
        flush_at=10**9,
        max_queue_size=N * 3,
        remote_config=False,
    )
    target: Callable[[Any], Any] = lbd.LogboxMiddleware(view) if wrapped else view
    factory = RequestFactory()
    samples = []
    for i in range(N):
        path = f"/projects/prj_{i % 100}/tasks" + ("/fail" if i % 50 == 0 else "")
        request = factory.post(
            path + "?expand=owner", data=b'{"title":"Draft"}', content_type="application/json"
        )
        t = time.perf_counter()
        target(request)
        samples.append(time.perf_counter() - t)
    return sorted(samples)


def report(name: str, base: list[float], sdk: list[float]) -> dict[str, Any]:
    return {
        "name": name,
        "iterations": N,
        "baseline_p50_ms": round(pct(base, 0.5), 4),
        "baseline_p99_ms": round(pct(base, 0.99), 4),
        "with_sdk_p50_ms": round(pct(sdk, 0.5), 4),
        "with_sdk_p99_ms": round(pct(sdk, 0.99), 4),
        "added_p50_ms": round(pct(sdk, 0.5) - pct(base, 0.5), 4),
        "added_p99_ms": round(pct(sdk, 0.99) - pct(base, 0.99), 4),
        "added_mean_ms": round((statistics.fmean(sdk) - statistics.fmean(base)) * 1000, 4),
    }


def main() -> int:
    results = []
    asyncio.run(bench_asgi(True))  # warm up
    results.append(report("asgi", asyncio.run(bench_asgi(False)), asyncio.run(bench_asgi(True))))
    try:
        bench_django(True)  # warm up
        results.append(report("django", bench_django(False), bench_django(True)))
    except ImportError:
        print("django not installed; skipping", file=sys.stderr)
    print(json.dumps({"python": sys.version.split()[0], "results": results}, indent=2))
    return 1 if any(r["added_p99_ms"] >= 0.2 for r in results) else 0


if __name__ == "__main__":
    sys.exit(main())
