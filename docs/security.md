# Security

Trailscope stores what your APIs receive and send, so it is built to keep as little sensitive data as possible, to show it only to people allowed to see it, and to make the database hard to misuse even from the query bar. This page describes the controls and what operators should configure. To report a vulnerability, see [SECURITY.md](../SECURITY.md).

## Collecting less

- **Bodies only when they help.** SDKs capture request and response bodies for errors and slow requests by default (`bodyCapture: errors`), at most 8 KB each, and skip binary and multipart content.
- **Redaction before data leaves the process.** SDKs replace `authorization`, `cookie`, `set-cookie` and `x-api-key` headers and body keys such as `password`, `token`, `secret`, `card_number`, `iban` and `ssn` (at any depth) with `[REDACTED]`, plus your own keys, paths and patterns ([sdk-spec.md](sdk-spec.md#5-redaction)).
- **Redaction again on the server.** Ingest applies the same built-in rules and each project's custom rules (Settings → Redaction), so data from older SDKs, OpenTelemetry exporters or hand-written clients is covered too.
- **IP masking** (on by default, per project) keeps only the first three IPv4 octets or the first 48 bits of IPv6.
- **Shorter retention for bodies.** Bodies and headers expire after `retention.body_days` (default 7), everything else after `retention.days` (default 30).
- **Emails never carry bodies, headers or IPs.** The Email on 5xx engine never receives them.

## Who sees what

- **Roles:** owner, admin, member, viewer. Viewers read and save their views; members also edit alert rules and replay requests; admins manage projects, keys, members, channels and settings; only owners can create or change owners, and there is always at least one.
- **Can view bodies** is a separate per-member permission. Without it, bodies and headers are hidden in the UI and the API, and queries can't filter on them.
- **API tokens** have scopes (read, write, admin; admin only for admins), act with their creator's role, and can be bound to one project. Only their SHA-256 hash is stored; the token is shown once.
- **Ingest keys** are write-only per project: they can send data and read the project's SDK settings, nothing else. Only their hash is stored.

## Signing in

- Passwords are hashed with **argon2id**. Sign-in is rate limited per email (8 failures) and per IP (30) in 15 minutes, and unknown emails cost the same time as known ones.
- Sessions are random tokens in an `HttpOnly`, `SameSite=Lax` cookie (`Secure` when `public_url` is https); only their hash is stored. State-changing requests need a per-session **CSRF token**. Changing a password signs out other sessions.
- Optional **OpenID Connect** single sign-on uses the authorization code flow with PKCE, validates ID tokens against the provider's keys, and only links accounts by subject or a verified email ([configuration.md](configuration.md#oidc)).
- Invites are single-use links that expire after 7 days.

## Queries can't escape

- The query language is parsed into Trailscope's own syntax tree and compiled to SQL with every value as a bound parameter. Only registered columns and functions are allowed; there are no subqueries, casts, joins or table names ([query-language.md](query-language.md)).
- Every user-driven query runs as a dedicated **read-only ClickHouse user** limited by a settings profile (execution time, rows read, result rows, memory), and is always scoped to one project.

## Outbound requests

- **Replay** only sends to targets an admin configured, never follows redirects, drops authentication headers and redacted values, refuses truncated bodies, and is logged.
- **Webhooks** are signed with HMAC-SHA256 so receivers can verify them; Slack, Telegram and webhook URLs and tokens are stored encrypted.

## Secrets at rest

The SMTP password and channel secrets are encrypted with XChaCha20-Poly1305 using `TRAILSCOPE_SECRET_KEY`, which also signs the mute links in emails. Without that variable, Trailscope generates a key and stores it in Postgres, which protects much less: anyone with a database dump could decrypt the secrets.

## The web UI

The UI is served from the binary with a strict Content-Security-Policy (only same-origin scripts, no inline scripts), `X-Content-Type-Options: nosniff` and frame protection. It calls only the same-origin API.

## Operator checklist

- Put Trailscope behind TLS (a reverse proxy; see [self-hosting.md](self-hosting.md)) and set `TRAILSCOPE_SERVER_PUBLIC_URL` to the https URL, so cookies are `Secure`.
- Set `TRAILSCOPE_SECRET_KEY` to a long random value (`openssl rand -base64 48`) and keep it with your backups.
- Change the default database passwords: `CLICKHOUSE_PASSWORD`, `CLICKHOUSE_READER_PASSWORD` (`TRAILSCOPE_CLICKHOUSE_READER_PASSWORD`) and `POSTGRES_PASSWORD`.
- Don't publish the ClickHouse, Postgres or Redis ports; only Trailscope's port needs to be reachable (and only the ingest paths from your services, if you split them).
- Turn on `server.trust_proxy` only when a proxy you control sets `X-Forwarded-For`.
- Give "can view bodies" only to people who need it, and prefer project-bound, read-scoped API tokens for automation.
- Add redaction rules for any field specific to your data before sending production traffic.
