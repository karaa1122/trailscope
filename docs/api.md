# HTTP API

The web UI is built entirely on the public API, so anything the UI does can be scripted. A running server publishes:

- the OpenAPI 3.1 document at **`/api/openapi.json`**, and
- an interactive reference at **`/api/docs`**.

Ingest (`/v1/ingest`, `/v1/releases`, `/v1/config`) is separate and uses ingest keys; see [ingest-protocol.md](ingest-protocol.md).

## Authentication

| Method | How | Use for |
|---|---|---|
| Session | `POST /api/v1/auth/login` sets an HttpOnly `trailscope_session` cookie (SameSite=Lax, Secure on https) and returns a `csrfToken` | The web UI |
| API token | `Authorization: Bearer trs_api_…` | Scripts, CI, dashboards |

With a session, every `POST`, `PUT`, `PATCH` and `DELETE` must send the CSRF token in `X-CSRF-Token`. `GET /api/v1/auth/me` returns it again after a page reload. API tokens don't need it.

Failed sign-ins are rate limited: 8 per email and 30 per client IP in 15 minutes, after which the response is `429`. Behind a reverse proxy, set `server.trust_proxy = true` so the client IP comes from `X-Forwarded-For`.

### Organizations

Projects, members, invites and API tokens belong to an **organization**. A person can belong to several organizations, with a role in each, and a browser session works in one at a time: everything the API returns, from the project list to query results, is limited to that organization. A project of another organization is answered as `404`, even for someone who is a member there too. `POST /api/v1/orgs/{id}/switch` moves the session to another of your organizations, and is remembered for the next sign-in. An API token always acts in the organization it was created in.

`GET /api/v1/auth/me` returns the current organization (`org`), every organization you belong to (`orgs`, with your role in each) and `instanceAdmin`. **Instance admins** (the person who ran setup, and on upgraded installs everyone who was an owner) manage server-wide settings (SMTP) and create organizations; everything else is per organization. An install that never creates a second organization works exactly as before, with all its projects and members in one organization.

### Management API

For the operator of a shared Trailscope (decision 072): `/api/v1/admin` lists every organization with its plan, members, projects and events this month; creates one for a customer, optionally with an owner invite link, without making anyone else a member; changes its plan, name or private note; suspends it (its ingest keys are refused when `cloud.enabled` is on); and invites anyone to it, owners included. Plans (`/api/v1/admin/plans`) are tiers of limits: events per month (`null` for unlimited), whether ingest stops at the limit or counts overage, and project and member caps; changes reach every organization on the plan within seconds. It answers only `Authorization: Bearer <TRAILSCOPE_ADMIN_TOKEN>` and is off when the token isn't set ([configuration.md](configuration.md#management-api)); sessions and API tokens get `401`/`403`. Trailscope has no screens for it: prices, invoices and payments belong in the operator's own tools, which call this API from their server.

### Roles

Roles are per organization: a member sees every project of their organization, and nothing of others.

| Role | Can |
|---|---|
| `viewer` | Read everything (except bodies, see below) and save private views |
| `member` | Also share views and replay requests |
| `admin` | Also manage projects, ingest keys, redaction rules, replay targets, members and invites |
| `owner` | Everything, including deleting projects and managing owners |

**Can view bodies** is a separate permission (owners always have it). Without it, request and response bodies and headers are left out of every response, can't be used in filters or as output columns, and replay is unavailable.

### API tokens

`POST /api/v1/tokens` with `{"name": "ci", "scopes": ["read"], "projectId": "…", "expiresInDays": 90}` returns the plaintext once. The scopes are:

- `read` — queries, the inspector and listings;
- `write` — also views and replays;
- `admin` — also administration, and only allowed for admins.

A token bound to a project can only see that project. A token can never act beyond its owner's role in its organization, and stops working when its owner leaves that organization.

## Errors

```json
{ "error": { "code": "invalid_query", "message": "unknown column `stauts`; did you mean `status`?",
             "details": { "message": "…", "line": 1, "col": 1, "len": 6 } } }
```

| Code | Status | Meaning |
|---|---|---|
| `bad_request` | 400 | Invalid input |
| `unauthorized`, `invalid_credentials` | 401 | Not signed in, or wrong credentials |
| `forbidden` | 403 | Role, scope, CSRF or permission check failed |
| `not_found` | 404 | |
| `conflict` | 409 | Already exists |
| `invalid_query` | 422 | The filter didn't parse; `details` has the position |
| `query_too_large` | 422 | The query hit the reader's row or memory limit |
| `rate_limited` | 429 | |
| `query_timeout` | 504 | The query hit the execution time limit |

## Main endpoints

| Area | Endpoints |
|---|---|
| Setup | `GET /api/v1/setup` (also says whether single sign-on is on), `POST /api/v1/setup` |
| Auth | `POST /api/v1/auth/login`, `POST /api/v1/auth/logout`, `GET /api/v1/auth/me`, `POST /api/v1/auth/password`; single sign-on: `GET /api/v1/auth/oidc/start` and `/callback` (browser redirects, see [configuration.md](configuration.md#oidc)) |
| Organizations | `GET`/`POST /api/v1/orgs`, `PATCH /api/v1/orgs/current`, `GET /api/v1/orgs/current/usage` (plan and this month's usage), `POST /api/v1/orgs/{id}/switch` |
| Management | `GET`/`POST /api/v1/admin/orgs`, `GET`/`PATCH /api/v1/admin/orgs/{id}`, `POST /api/v1/admin/orgs/{id}/invites`, `GET`/`POST /api/v1/admin/plans`, `PUT /api/v1/admin/plans/{id}`. Only with the admin token |
| Members | `GET /api/v1/members`, `PATCH`/`DELETE /api/v1/members/{id}`, `POST`/`GET /api/v1/invites`, `GET /api/v1/invites/lookup`, `POST /api/v1/invites/accept`. Members, invites and removals are per organization; someone who already has an account accepts an invite with its password, and removing the last membership of a member who isn't an instance admin deletes the account |
| Tokens | `GET`/`POST /api/v1/tokens`, `DELETE /api/v1/tokens/{id}` |
| Projects | `GET`/`POST /api/v1/projects`, `GET`/`PATCH`/`DELETE /api/v1/projects/{id}`, plus `/keys`, `/redaction-rules`, `/replay-targets`, `/environments`, `/services` and `/releases` under each project |
| Query | `POST /api/v1/query/validate`, `/requests`, `/histogram`, `/facets`, `/group`; `GET /api/v1/schema`, `/api/v1/values`, `/api/v1/keys` |
| Inspector | `GET /api/v1/requests/{id}`, `GET /api/v1/requests/{id}/curl`, `POST /api/v1/requests/{id}/replay` |
| Live | `GET /api/v1/live` (Server-Sent Events) |
| Views | `GET`/`POST /api/v1/views`, `PUT`/`DELETE /api/v1/views/{id}` |
| Overview | `GET /api/v1/overview`, `GET /api/v1/routes/heatmap`, `GET /api/v1/routes/series` |
| Errors | `GET /api/v1/errors`, `GET /api/v1/errors/{fingerprint}` |
| Journeys | `GET /api/v1/journeys` |
| Alerts | `/api/v1/alerts/rules` (CRUD, `/preview`), `/api/v1/alerts/channels` (CRUD, `/{id}/test`), `/api/v1/alerts/email-rule` (`GET`/`PUT`, `/test`), `/api/v1/alerts/history`, `/api/v1/alerts/mutes`, `/api/v1/alerts/mute` (signed link, no login). Details in [alerts.md](alerts.md#api) |
| Settings | `GET /api/v1/settings/retention`, `GET`/`PUT /api/v1/settings/smtp`, `POST /api/v1/settings/smtp/test` |

### Query endpoints

All query endpoints take the same scope:

```json
{
  "project": "01a16f0c-…",
  "table": "requests",
  "query": { "mode": "sql", "text": "status >= 500 AND route LIKE '/checkout/%'" },
  "from": 1767225600000,
  "to": 1767229200000,
  "environment": "production"
}
```

`query` can also be `{"mode": "search", "text": "status:5xx"}` or `{"mode": "filters", "chips": [...]}`; see [query-language.md](query-language.md).

- `/query/requests` takes `columns`, `limit` (up to `query.max_limit`), `order` (`desc` or `asc`) and `cursor`. It returns `rows` and a `nextCursor` for keyset pagination on `(timestamp, request_id)`.
- `/query/histogram` returns buckets (sized automatically for about 60 points), split by status class, with counts scaled by `1 / sample_rate`.
- `/query/facets` takes `fields: [{column, key?}]` and returns the top values with counts under the current filter.
- `/query/group` takes `by: {column, key?}` and returns count, error rate and p50/p95/p99 per group.

### Journeys

`GET /api/v1/journeys?project=…&key=…&value=…` returns every request and event carrying one value, merged into a single timeline. `key` is a correlation key (`account_id`, `session_id`, …) or one of `user_id`, `request_id`, `trace_id`. Events triggered by a matching request are included even when they don't carry the key themselves. The window defaults to the last 7 days (`from`/`to` in ms); `environment` narrows it.

The response has `summary` (counts, services, `inProgress` when there was activity in the last 5 minutes), `lanes` (one per service, background jobs as `jobs · <service>`), `items` (runs of more than 5 identical items in a lane are collapsed into one with `similar` and `until`), `context` (the most common correlation values, attributes, user and client), related `errors` and `traces`. At most 2,000 requests and 2,000 events are read; `truncated` says when that limit was hit.

### Routes

`GET /api/v1/routes/series?project=…&route=…&minutes=60` returns p50/p95/p99, requests, 4xx and 5xx per bucket for one route, from the per-minute rollup. Buckets are 1, 5, 15 or 60 minutes depending on the window (up to 7 days).

### Live tail

```
GET /api/v1/live?project=…&filter=status%20%3E%3D%20500&mode=sql&environment=production
Accept: text/event-stream
```

| Event | Data |
|---|---|
| `ready` | `{}` once, when the stream is subscribed |
| `request` | One request summary (JSON), matching the filter |
| `skipped` | `{"count": N}`: matching events dropped because the client exceeded `live.max_events_per_sec` or fell behind |

Pausing is client-side: keep the stream open and buffer or count events.

### Replay

Admins configure **replay targets** per project (`POST /api/v1/projects/{id}/replay-targets` with `{"name": "staging", "baseUrl": "https://staging.example.com"}`). A replay re-sends the stored request to one of those targets, never to an arbitrary URL, without following redirects. `authorization`, `cookie`, `x-api-key` and redacted headers are left out; send fresh values in `headers` if the target needs them. The result is stored and returned.
