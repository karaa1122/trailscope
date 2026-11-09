# Alerts and email notifications

Trailscope has two ways to tell you something is wrong:

- **Alert rules**: a query, a condition and a time window, checked every minute. Each state change (firing, resolved) sends one notification to the rule's channels.
- **Email on 5xx**: a built-in rule per project that emails the people you choose within seconds of a server error, with noise control so one bug doesn't send 500 emails.

Both are managed under **Alerts** in the UI. Every notification, sent or failed, is listed under **Alerts → History**.

## Alert rules

A rule has:

| Field | Description |
|---|---|
| Condition | `5xx rate` (percent of matching requests with status ≥ 500), `p95 latency` (ms, of `duration_ms`), `Count` (matching requests, adjusted for sampling), `New error group` (groups first seen in the window), or `Spiking error group` (groups with at least 10 errors in the window and more than 5× their average rate over the previous 24 hours) |
| Filter | Any query in search, filters or SQL mode, the same as in Explore. Not used by the two error group conditions |
| Environment | Limits the rule to one environment, or all of them |
| Threshold | The rule fires when the value is **above** this number |
| Window | 1 to 1440 minutes |
| Cooldown | A new Firing notification waits at least this long after the rule's previous notification (default 10 minutes), so a flapping rule doesn't send a stream of messages |
| Channels | Where notifications go. A rule without channels still records its state |

Rules are evaluated every `alerts.interval_secs` (60 by default) as the read-only ClickHouse user. A rule is either `ok` or `firing`:

- `ok` → `firing` sends a **Firing** notification, unless the cooldown since the last notification hasn't passed yet.
- `firing` → `ok` sends a **Resolved** notification, but only if the firing was announced.
- Editing a rule's condition, filter, threshold, window or environment puts it back to `ok`.

**Check now** in the editor shows the rule's current value without saving it. If a rule fails to evaluate (for example, after a column was removed), the error appears under its name in the list.

Messages include the rule, the value and the threshold, and link to Explore with the rule's query and the window pre-filled (or to the Errors page for the error group conditions).

## Channels

Channels belong to a project and are managed by admins. **Send test** delivers a test message and shows the exact response.

| Kind | Setup | What is sent |
|---|---|---|
| Slack | An [incoming webhook](https://api.slack.com/messaging/webhooks) URL | A short text message with a link |
| Webhook | Any http(s) URL | A JSON `POST`, signed (see below) |
| Telegram | A bot token from @BotFather and a chat ID | An HTML-formatted message |
| Email | Members and/or other addresses | One email per recipient, through the SMTP settings |

Webhook URLs, Slack URLs and bot tokens are stored encrypted (see [Secret key](#secret-key)) and are never returned by the API; the UI only shows the URL's host. Redirects are not followed and requests time out after 15 seconds.

### Webhook payload and signature

```json
{
  "type": "alert",
  "state": "firing",
  "rule": { "id": "01a1…", "name": "Checkout failing" },
  "project": "Shop",
  "environment": "production",
  "metric": "5xx rate (%)",
  "value": 7.4,
  "threshold": 5,
  "windowMinutes": 5,
  "query": "route = '/checkout'",
  "url": "https://logs.example.com/explore?mode=sql&q=…",
  "at": 1793513609000
}
```

A webhook channel gets a signing secret (`whsec_…`), shown once when the channel is created. Each request carries:

- `X-Trailscope-Timestamp`: unix seconds.
- `X-Trailscope-Signature`: `sha256=` followed by the hex HMAC-SHA256 of `timestamp + "." + body`, keyed with the signing secret.

Verify the signature over the raw body with a constant-time comparison, and reject timestamps more than a few minutes old to prevent replays. In Node:

```js
import { createHmac, timingSafeEqual } from 'node:crypto';

function verify(secret, timestamp, rawBody, header) {
  const expected = 'sha256=' + createHmac('sha256', secret).update(`${timestamp}.${rawBody}`).digest('hex');
  const fresh = Math.abs(Date.now() / 1000 - Number(timestamp)) < 300;
  return fresh && header.length === expected.length && timingSafeEqual(Buffer.from(header), Buffer.from(expected));
}
```

## Email on 5xx

Every project has a built-in **Email on 5xx** rule (Alerts → Email on 5xx). It is enabled by default but sends nothing until SMTP is configured and at least one recipient is listed.

- **Recipients:** any members (by account, so a changed address follows the account; disabled accounts are skipped) and any other addresses. Only the people listed here receive these emails. **Send a test email to them** sends a sample error email.
- **Filter:** a query-language expression over the error's fields, by default `status >= 500 AND environment = 'production'`. Usable columns: `status`, `environment`, `service`, `release`, `method`, `route`, `duration_ms`, `user_id`, `error_type`, `error_message`, `attributes[…]` and `correlation[…]`. Bodies, headers and IPs are not available here.
- **Real time:** after each successful write, ingest hands the 5xx rows to the alert engine (in process in `trailscope all`, over Redis in split mode), so the email usually goes out within a few seconds of the error. Changes to the rule or the SMTP settings take effect within 30 seconds.

### Noise control

Emails are grouped by error group (`error_fingerprint`; errors without one are grouped by status, method and route):

| Situation | What happens |
|---|---|
| First occurrence of a group | Emailed immediately |
| More occurrences | At most one email per group every 15 minutes (`alerts.email_throttle_minutes`). When the window ends, a digest is sent only if the error kept happening: "42 more in the last 15 min, 17 users affected" |
| A group quiet for 24 hours comes back | Emailed immediately as a regression (`alerts.regression_hours`) |
| More than 30 emails for the project in one clock hour (UTC) | One "notifications paused" email, then nothing until the next hour (`alerts.email_hourly_cap`). Suppressed occurrences are included in the group's next digest |
| A muted group | Not emailed until the mute ends; errors are still recorded |

One email to several recipients counts once toward the hourly cap.

### What an email contains

- Subject: `[Trailscope] 500 POST /orders/:id/checkout — PaymentTimeoutError · orders-svc`. Always the route pattern, never a raw path with IDs. Digests add "(N more)"; regressions start with "Regression:".
- Body (HTML and plain text, dark text on a light background, one column for phones): time, environment, service, release, method and route, status, duration, request ID, user ID, error type and message, the top 5 stack frames, occurrences and users affected so far, a button to the request inspector and a link to the error group.
- **Never** request or response bodies, headers or IP addresses: the engine only receives a reduced copy of the request without them.

### Mute links

Each email links to "Mute this error for 1 hour / 24 hours / forever". The links are signed with the [secret key](#secret-key), work without signing in, and expire after 7 days (`alerts.mute_link_days`). Opening one shows a confirmation page, and the mute applies only after the button is pressed, so link scanners in mail systems can't mute errors by fetching the URL. Mutes are listed under Alerts → Email on 5xx, where members can lift them.

## SMTP settings

Admins configure SMTP under **Settings → Notifications**: host, port, security (STARTTLS, implicit TLS or none), username, password, from address and from name. **Send test email** reports the exact SMTP error on failure (for example `permanent error (535): 5.7.8 authentication failed`).

The same settings can be set with environment variables or in `trailscope.toml` (see [configuration.md](configuration.md#smtp)):

```bash
TRAILSCOPE_SMTP_HOST=smtp.example.com
TRAILSCOPE_SMTP_PORT=587
TRAILSCOPE_SMTP_SECURITY=starttls   # starttls, tls or none
TRAILSCOPE_SMTP_USERNAME=trailscope
TRAILSCOPE_SMTP_PASSWORD=…
TRAILSCOPE_SMTP_FROM=trailscope@example.com
TRAILSCOPE_SMTP_FROM_NAME=Trailscope
```

A value set in configuration wins over the UI value for that field, and the UI shows the field as locked. The password entered in the UI is stored encrypted and never returned by the API, which only reports whether one is set. SMTP is considered configured when a host and a from address are set.

## Delivery and history

Notifications are written to a queue in Postgres and sent by a worker. A failed send is retried with exponential backoff (after 30 s, 1, 2 and 4 minutes) up to 5 attempts, then marked failed. Alerts → History shows every notification with its time, recipient or channel, rule, reason (`first`, `digest`, `regression`, `paused`, `firing`, `resolved` or `test`), error group, status, attempts and the server's answer or error. Test emails from Settings → Notifications aren't tied to a project, so they appear only in the result shown on that page.

Metrics on `/metrics`: `trailscope_notifications_sent_total{kind}`, `trailscope_notifications_failed_total{kind}` and `trailscope_alert_eval_seconds`.

## Secret key

`TRAILSCOPE_SECRET_KEY` (or `secret_key` at the top of `trailscope.toml`) encrypts the stored SMTP password and channel secrets (XChaCha20-Poly1305) and signs mute links (HMAC-SHA256). Use a long random value:

```bash
openssl rand -base64 48
```

If it isn't set, Trailscope generates a key on first start and keeps it in Postgres. That works, but anyone with a database dump can then decrypt the stored secrets, so set the variable in production; the UI and the logs warn while it's missing. Changing the key makes stored secrets unreadable (re-enter the SMTP password and recreate channels) and invalidates mute links already sent.

## Running in split mode

In split mode (`trailscope ingest`, `trailscope api`, `trailscope alerts`), run **exactly one** `alerts` process. It evaluates every rule and receives every 5xx through Redis, so a second one would send everything twice. The API process only queues and test-sends; it doesn't deliver queued notifications.

## Local testing with Mailpit

The compose file has a `dev` profile with [Mailpit](https://mailpit.axllent.org/), which catches all email:

```bash
docker compose --profile dev up -d
```

Then in Settings → Notifications use host `mailpit`, port `1025`, security `none`, and any from address. Sent emails appear at http://localhost:8025.

## API

All endpoints are in the OpenAPI document at `/api/docs` (tag `alerts`).

| Endpoint | Role |
|---|---|
| `GET`/`POST /api/v1/alerts/rules`, `PUT`/`DELETE /api/v1/alerts/rules/{id}` | viewer reads, member writes |
| `POST /api/v1/alerts/rules/preview` | viewer |
| `GET`/`POST /api/v1/alerts/channels`, `DELETE /api/v1/alerts/channels/{id}`, `POST …/{id}/test` | viewer reads, admin writes |
| `GET`/`PUT /api/v1/alerts/email-rule`, `POST /api/v1/alerts/email-rule/test` | viewer reads, admin writes |
| `GET`/`PUT /api/v1/settings/smtp`, `POST /api/v1/settings/smtp/test` | admin |
| `GET /api/v1/alerts/history` | viewer |
| `GET /api/v1/alerts/mutes`, `DELETE /api/v1/alerts/mutes/{fingerprint}` | viewer reads, member writes |
| `GET`/`POST /api/v1/alerts/mute?t=…` | none (signed link) |
