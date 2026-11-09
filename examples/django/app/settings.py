"""Minimal Django settings: no database, one app module, Trailscope first in MIDDLEWARE."""

import os

SECRET_KEY = "example-only-not-secret"
DEBUG = False
ALLOWED_HOSTS = ["*"]
ROOT_URLCONF = "app.urls"
INSTALLED_APPS: list[str] = []
DATABASES: dict[str, dict[str, str]] = {}
USE_TZ = True

MIDDLEWARE = [
    "trailscope.django.TrailscopeMiddleware",
    "django.middleware.common.CommonMiddleware",
]

# Keyword options for trailscope.Client; the ingest key comes from TRAILSCOPE_INGEST_KEY.
TRAILSCOPE = {
    "service": "tasks-api-django",
    "endpoint": os.environ.get("TRAILSCOPE_ENDPOINT", "http://localhost:8080"),
}

# Application log lines become events linked to the current request.
LOGGING = {
    "version": 1,
    "disable_existing_loggers": False,
    "handlers": {
        "console": {"class": "logging.StreamHandler"},
        "trailscope": {"class": "trailscope.TrailscopeHandler", "level": "INFO"},
    },
    "loggers": {"tasks": {"handlers": ["console", "trailscope"], "level": "INFO"}},
}
