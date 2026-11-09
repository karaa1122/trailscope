"""Minimal FastAPI app wired to Trailscope.

Start Trailscope, then:

    pip install -r requirements.txt
    TRAILSCOPE_INGEST_KEY=trs_ingest_... uvicorn main:app --port 3000
    curl localhost:3000/projects/p1/tasks
"""

import asyncio
import logging
import os

from fastapi import FastAPI, Request

import trailscope
from trailscope.asgi import TrailscopeMiddleware

client = trailscope.init(
    service="tasks-api-fastapi",
    endpoint=os.environ.get("TRAILSCOPE_ENDPOINT", "http://localhost:8080"),
)
# Application log lines become events linked to the current request.
logging.basicConfig(level=logging.INFO)
logging.getLogger().addHandler(trailscope.TrailscopeHandler())
logger = logging.getLogger("tasks")

app = FastAPI()


@app.middleware("http")
async def pretend_auth(request: Request, call_next):  # type: ignore[no-untyped-def]
    user = request.headers.get("x-user-id")
    if user:
        trailscope.set_user(user)
    return await call_next(request)


# Added last so it runs first: Starlette runs the most recently added middleware outermost.
app.add_middleware(TrailscopeMiddleware, client=client)


@app.get("/health")
async def health() -> str:  # ignored by default
    return "ok"


@app.get("/projects/{project_id}/tasks")
async def list_tasks(project_id: str) -> list[dict[str, object]]:
    trailscope.set_correlation("project_id", project_id)
    logger.info("listing tasks of %s", project_id)
    return [{"id": "t1", "title": "Write the brief", "done": False}]


@app.post("/projects/{project_id}/tasks", status_code=201)
async def create_task(project_id: str, body: dict[str, object]) -> dict[str, object]:
    trailscope.set_correlation("project_id", project_id)
    task = {"id": "t2", **body}

    # A background job linked to this request (same request ID, user and project).
    @client.track_job("task.notify")
    async def notify() -> None:
        await asyncio.sleep(0.05)
        logger.info("notified watchers of %s", task["id"])

    asyncio.create_task(notify())
    return task


@app.get("/projects/{project_id}/report")
async def report(project_id: str) -> dict[str, int]:
    trailscope.set_correlation("project_id", project_id)
    with client.span("db", "SELECT tasks"):
        await asyncio.sleep(0.03)
    return {"rows": 42}


@app.get("/projects/{project_id}/export")
async def export(project_id: str) -> None:
    raise TypeError("'NoneType' object is not subscriptable")
