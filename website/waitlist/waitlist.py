#!/usr/bin/env python3
"""Trailscope Cloud waitlist: POST /api/waitlist stores an email in SQLite.

Runs behind nginx on 127.0.0.1 (see ../deploy/). Standard library only, so the
server needs nothing but python3.

    waitlist.py serve  --db /var/lib/trailscope-waitlist/waitlist.db [--port 8790]
    waitlist.py export --db /var/lib/trailscope-waitlist/waitlist.db   # CSV on stdout
    waitlist.py delete --db ... someone@example.com                    # on request

Requests come as JSON from the page's script (answered with JSON) or as a plain
form post without JavaScript (answered with a redirect to /thanks/ or
/waitlist-error/). Nothing but the submitted fields and the time is stored; the
client address is used only for the in-memory rate limit.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sqlite3
import sys
import threading
import time
from datetime import datetime, timezone
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

PATH = "/api/waitlist"
MAX_BODY = 4096
EMAIL = re.compile(r"^[^@\s\"'<>(),;:\\]{1,64}@[A-Za-z0-9.-]{1,253}\.[A-Za-z]{2,63}$")
PLANS = {"free", "team", "business", "unsure"}
VOLUMES = {"unsure", "<1m", "1-10m", "10-50m", "50m+"}
RATE_LIMIT = 10  # submissions per address
RATE_WINDOW = 3600.0  # seconds

SCHEMA = """
CREATE TABLE IF NOT EXISTS signups (
    email       TEXT PRIMARY KEY,
    plan        TEXT NOT NULL,
    volume      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
)
"""


class Store:
    def __init__(self, path: str) -> None:
        self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.execute(SCHEMA)
        self._lock = threading.Lock()

    def add(self, email: str, plan: str, volume: str) -> None:
        now = datetime.now(timezone.utc).isoformat(timespec="seconds")
        with self._lock:
            self._db.execute(
                "INSERT INTO signups (email, plan, volume, created_at, updated_at)"
                " VALUES (?, ?, ?, ?, ?)"
                " ON CONFLICT (email) DO UPDATE SET plan = excluded.plan,"
                " volume = excluded.volume, updated_at = excluded.updated_at",
                (email, plan, volume, now, now),
            )

    def rows(self) -> list[tuple[str, str, str, str, str]]:
        with self._lock:
            cur = self._db.execute(
                "SELECT email, plan, volume, created_at, updated_at FROM signups"
                " ORDER BY created_at"
            )
            return list(cur.fetchall())

    def delete(self, email: str) -> int:
        with self._lock:
            return self._db.execute("DELETE FROM signups WHERE email = ?", (email,)).rowcount


class RateLimiter:
    def __init__(self) -> None:
        self._hits: dict[str, list[float]] = {}
        self._lock = threading.Lock()

    def allow(self, key: str) -> bool:
        now = time.monotonic()
        with self._lock:
            if len(self._hits) > 10_000:  # forget everyone rather than grow without bound
                self._hits.clear()
            hits = [t for t in self._hits.get(key, []) if now - t < RATE_WINDOW]
            allowed = len(hits) < RATE_LIMIT
            if allowed:
                hits.append(now)
            self._hits[key] = hits
            return allowed


def parse(body: bytes, content_type: str) -> dict[str, str]:
    if content_type.startswith("application/json"):
        data = json.loads(body.decode("utf-8"))
        if not isinstance(data, dict):
            raise ValueError("expected an object")
        return {str(k): v for k, v in data.items() if isinstance(v, str)}
    form = parse_qs(body.decode("utf-8"), max_num_fields=10)
    return {k: v[0] for k, v in form.items() if v}


def validate(fields: dict[str, str]) -> tuple[str, str, str]:
    email = fields.get("email", "").strip().lower()
    if len(email) > 254 or not EMAIL.match(email):
        raise ValueError("Please enter a valid email address.")
    plan = fields.get("plan", "unsure")
    volume = fields.get("volume", "unsure")
    return email, plan if plan in PLANS else "unsure", volume if volume in VOLUMES else "unsure"


def make_handler(store: Store, limiter: RateLimiter) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        server_version = "trailscope-waitlist"
        sys_version = ""

        def log_message(self, format: str, *args: object) -> None:
            # nginx already logs requests; don't write addresses or emails twice.
            pass

        def _wants_json(self) -> bool:
            accept = self.headers.get("Accept", "")
            ctype = self.headers.get("Content-Type", "")
            return "application/json" in accept or ctype.startswith("application/json")

        def _reply(self, status: HTTPStatus, ok: bool, error: str | None = None) -> None:
            if self._wants_json():
                payload = json.dumps({"ok": ok} | ({"error": error} if error else {})).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(payload)
            else:
                self.send_response(HTTPStatus.SEE_OTHER)
                self.send_header("Location", "/thanks/" if ok else "/waitlist-error/")
                self.send_header("Content-Length", "0")
                self.end_headers()

        def do_GET(self) -> None:
            if self.path == PATH + "/health":
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"ok")
            else:
                self.send_error(HTTPStatus.NOT_FOUND)

        def do_POST(self) -> None:
            if self.path != PATH:
                self.send_error(HTTPStatus.NOT_FOUND)
                return
            client = self.headers.get("X-Real-IP") or self.client_address[0]
            if not limiter.allow(client):
                self._reply(HTTPStatus.TOO_MANY_REQUESTS, False, "Too many tries. Please wait.")
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                length = -1
            if length < 0 or length > MAX_BODY:
                self._reply(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, False, "Request too large.")
                return
            try:
                fields = parse(self.rfile.read(length), self.headers.get("Content-Type", ""))
                if fields.get("website"):  # the hidden field only bots fill in
                    self._reply(HTTPStatus.OK, True)
                    return
                email, plan, volume = validate(fields)
            except (ValueError, UnicodeDecodeError) as e:
                message = str(e) if str(e).startswith("Please") else "Invalid request."
                self._reply(HTTPStatus.BAD_REQUEST, False, message)
                return
            try:
                store.add(email, plan, volume)
            except sqlite3.Error as e:
                print(f"waitlist: storing a signup failed: {e}", file=sys.stderr, flush=True)
                self._reply(HTTPStatus.INTERNAL_SERVER_ERROR, False, "Please try again later.")
                return
            self._reply(HTTPStatus.OK, True)

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    serve = sub.add_parser("serve", help="run the HTTP endpoint")
    serve.add_argument("--db", required=True)
    serve.add_argument("--host", default="127.0.0.1")
    serve.add_argument("--port", type=int, default=8790)
    export = sub.add_parser("export", help="print all signups as CSV")
    export.add_argument("--db", required=True)
    delete = sub.add_parser("delete", help="remove one email address")
    delete.add_argument("--db", required=True)
    delete.add_argument("email")
    args = parser.parse_args()

    store = Store(args.db)
    if args.command == "export":
        writer = csv.writer(sys.stdout)
        writer.writerow(["email", "plan", "volume", "created_at", "updated_at"])
        writer.writerows(store.rows())
    elif args.command == "delete":
        removed = store.delete(args.email.strip().lower())
        print(f"removed {removed} signup(s)")
    else:
        server = ThreadingHTTPServer((args.host, args.port), make_handler(store, RateLimiter()))
        print(f"waitlist: listening on {args.host}:{args.port}", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
