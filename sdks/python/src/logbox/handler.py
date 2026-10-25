"""A ``logging`` handler that sends records to Logbox as ``log`` events."""

from __future__ import annotations

import logging

from .client import Client, Level, get_client

_LEVELS: tuple[tuple[int, Level], ...] = (
    (logging.ERROR, "error"),
    (logging.WARNING, "warn"),
    (logging.INFO, "info"),
)


class LogboxHandler(logging.Handler):
    """Sends log records to Logbox, linked to the current request or job.

    ::

        logging.getLogger().addHandler(LogboxHandler(level=logging.INFO))

    The record's logger name becomes the event name, ``exc_info`` becomes the
    error type, message and stack, and records from Logbox's own logger are
    skipped.
    """

    def __init__(self, level: int = logging.INFO, client: Client | None = None) -> None:
        super().__init__(level)
        self._client = client

    def emit(self, record: logging.LogRecord) -> None:
        if record.name == "logbox" or record.name.startswith("logbox."):
            return
        client = self._client or get_client()
        if client is None:
            return
        try:
            level: Level = next((name for no, name in _LEVELS if record.levelno >= no), "debug")
            error = record.exc_info[1] if record.exc_info and record.exc_info[1] else None
            client.log(
                level,
                record.getMessage(),
                name=record.name,
                attributes={"module": record.module, "line": record.lineno},
                error=error,
            )
        except Exception:
            self.handleError(record)
