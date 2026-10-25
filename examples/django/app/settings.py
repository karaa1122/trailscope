"""Minimal Django settings: no database, one app module, Logbox first in MIDDLEWARE."""

import os

SECRET_KEY = "example-only-not-secret"
DEBUG = False
ALLOWED_HOSTS = ["*"]
ROOT_URLCONF = "app.urls"
INSTALLED_APPS: list[str] = []
DATABASES: dict[str, dict[str, str]] = {}
USE_TZ = True

MIDDLEWARE = [
    "logbox.django.LogboxMiddleware",
    "django.middleware.common.CommonMiddleware",
]

# Keyword options for logbox.Client; the ingest key comes from LOGBOX_INGEST_KEY.
LOGBOX = {
    "service": "tasks-api-django",
    "endpoint": os.environ.get("LOGBOX_ENDPOINT", "http://localhost:8080"),
}

# Application log lines become events linked to the current request.
LOGGING = {
    "version": 1,
    "disable_existing_loggers": False,
    "handlers": {
        "console": {"class": "logging.StreamHandler"},
        "logbox": {"class": "logbox.LogboxHandler", "level": "INFO"},
    },
    "loggers": {"tasks": {"handlers": ["console", "logbox"], "level": "INFO"}},
}
