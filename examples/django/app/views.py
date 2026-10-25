import json
import logging
import time

from django.http import HttpRequest, HttpResponse, JsonResponse

import logbox

from .tasks import notify

logger = logging.getLogger("tasks")


def health(request: HttpRequest) -> HttpResponse:
    return HttpResponse("ok")


def tasks(request: HttpRequest, project_id: str) -> HttpResponse:
    # Pretend authentication: attach the caller to everything this request does.
    logbox.set_user(request.headers.get("x-user-id"))
    logbox.set_correlation("project_id", project_id)
    if request.method == "POST":
        body = json.loads(request.body or b"{}")
        task = {"id": f"t{int(time.time() * 1000)}", **body}
        # The Celery task carries this request's context, so its runs are linked to it.
        notify.delay(task["id"], flaky=body.get("title") == "flaky")
        return JsonResponse(task, status=201)
    logger.info("listing tasks of %s", project_id)
    return JsonResponse([{"id": "t1", "title": "Write the brief", "done": False}], safe=False)


def export(request: HttpRequest, project_id: str) -> HttpResponse:
    raise TypeError("'NoneType' object is not subscriptable")
