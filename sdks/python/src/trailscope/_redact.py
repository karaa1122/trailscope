"""Redaction of headers, bodies, query strings and IPs. Same rules as the Node SDK."""

from __future__ import annotations

import json
import re
from collections.abc import Iterable, Mapping
from re import Pattern
from typing import Any
from urllib.parse import unquote_plus

REDACTED = "[REDACTED]"

DEFAULT_REDACT_HEADERS = ("authorization", "cookie", "set-cookie", "x-api-key")

DEFAULT_REDACT_KEYS = (
    "password",
    "passwd",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "otp",
    "pin",
    "cvv",
    "card",
    "card_number",
    "iban",
    "ssn",
)

_IPV4 = re.compile(r"^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.\d{1,3}$")


def _norm_key(k: str) -> str:
    """``accessToken``, ``access-token`` and ``ACCESS_TOKEN`` all become ``accesstoken``."""
    return k.replace("_", "").replace("-", "").lower()


class Redactor:
    def __init__(
        self,
        *,
        headers: Iterable[str] = (),
        keys: Iterable[str] = (),
        patterns: Iterable[str | Pattern[str]] = (),
        mask_ip: bool = True,
        disable_defaults: bool = False,
    ) -> None:
        base_headers = () if disable_defaults else DEFAULT_REDACT_HEADERS
        base_keys: list[str] = [] if disable_defaults else list(DEFAULT_REDACT_KEYS)
        self._headers = {h.lower() for h in (*base_headers, *headers)}
        self._paths: list[list[str]] = []
        simple = list(base_keys)
        for k in keys:
            if "." in k:
                self._paths.append([_norm_key(p) for p in k.split(".")])
            elif k:
                simple.append(k)
        self._keys = {_norm_key(k) for k in simple}
        self._patterns = [re.compile(p) if isinstance(p, str) else p for p in patterns]
        self.mask_ip = mask_ip
        self._raw_keys: Pattern[str] | None = None
        if simple:
            alts = "|".join("[_-]?".join(re.escape(part) for part in k.split("_")) for k in simple)
            self._raw_keys = re.compile(
                r'("(?:' + alts + r')"\s*:\s*)("(?:[^"\\]|\\.)*"?|[^,}\]\s]+)', re.IGNORECASE
            )

    def redact_headers(self, headers: Mapping[str, str]) -> dict[str, str]:
        out: dict[str, str] = {}
        for name, value in headers.items():
            lower = name.lower()
            out[lower] = REDACTED if lower in self._headers else self.apply_patterns(value)
        return out

    def redact_body(self, body: str, content_type: str = "") -> str:
        if not body:
            return body
        t = body.lstrip()
        if t.startswith(("{", "[")):
            try:
                parsed = json.loads(body)
            except ValueError:
                return self._redact_raw_text(body)
            return json.dumps(
                self.redact_value(parsed, []), ensure_ascii=False, separators=(",", ":")
            )
        if "x-www-form-urlencoded" in content_type:
            return self.redact_urlencoded(body)
        return self._redact_raw_text(body)

    def redact_urlencoded(self, s: str) -> str:
        parts = []
        for pair in s.split("&"):
            k, sep, v = pair.partition("=")
            if not sep:
                parts.append(pair)
                continue
            try:
                decoded = unquote_plus(k)
            except Exception:
                decoded = k
            if _norm_key(decoded) in self._keys:
                parts.append(f"{k}={REDACTED}")
            else:
                parts.append(f"{k}={self.apply_patterns(v)}")
        return "&".join(parts)

    def redact_query(self, q: str) -> str:
        return self.redact_urlencoded(q.lstrip("?")) if q else q

    def redact_value(self, v: Any, path: list[str]) -> Any:
        """Redacts a structured value: JSON bodies and job payloads."""
        if isinstance(v, list):
            return [self.redact_value(item, [*path, "*"]) for item in v]
        if isinstance(v, dict):
            out: dict[str, Any] = {}
            for k, child in v.items():
                nk = _norm_key(str(k))
                p = [*path, nk]
                if (nk in self._keys or self._path_matches(p)) and child is not None:
                    out[k] = REDACTED
                else:
                    out[k] = self.redact_value(child, p)
            return out
        if isinstance(v, str):
            return self.apply_patterns(v)
        return v

    def apply_patterns(self, s: str) -> str:
        for p in self._patterns:
            s = p.sub(REDACTED, s)
        return s

    def ip(self, ip: str | None) -> str | None:
        if not ip or not self.mask_ip:
            return ip
        return mask_ip(ip)

    def _redact_raw_text(self, s: str) -> str:
        if self._raw_keys is not None:
            s = self._raw_keys.sub(lambda m: f'{m.group(1)}"{REDACTED}"', s)
        return self.apply_patterns(s)

    def _path_matches(self, path: list[str]) -> bool:
        return any(
            len(p) == len(path)
            and all(
                want == "*" or got == "*" or want == got for want, got in zip(p, path, strict=True)
            )
            for p in self._paths
        )


def mask_ip(raw: str) -> str:
    """Zeroes the last IPv4 octet, or keeps the first 48 bits of an IPv6 address."""
    ip = raw.strip()
    if ip.startswith("::ffff:") and "." in ip:
        ip = ip[7:]
    m = _IPV4.match(ip)
    if m:
        return f"{m.group(1)}.{m.group(2)}.{m.group(3)}.0"
    if ":" in ip:
        head = ip.split("::")[0]
        parts = [p for p in head.split(":") if p]
        return ":".join(parts[:3]) + "::"
    return ""
