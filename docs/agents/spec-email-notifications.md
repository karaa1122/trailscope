# Follow-up brief: SMTP email notifications for 5xx errors

Received during M1, kept verbatim. "Logbox" was the working name of what is now Trailscope (decision 062). Scheduled as part of M6 (alerts), with the ingest hook built in M1. See [/AGENTS.md](../../AGENTS.md) for how it fits the plan.

---

Add SMTP email notifications for 5xx errors. Don't ask questions; make your own calls and log them in docs/DECISIONS.md.

1. SMTP settings (Settings → Notifications → Email)
- Fields: host, port, security (STARTTLS / implicit TLS / none), username, password, from address, from name.
- Can also be set via env vars (LOGBOX_SMTP_HOST, LOGBOX_SMTP_PORT, LOGBOX_SMTP_SECURITY, LOGBOX_SMTP_USERNAME, LOGBOX_SMTP_PASSWORD, LOGBOX_SMTP_FROM). Env wins over UI values, and the UI shows those fields as locked.
- Store the password encrypted at rest in Postgres (AES-256-GCM or XChaCha20-Poly1305) with a key from LOGBOX_SECRET_KEY. Never return the password from the API; show only "set / not set".
- A "Send test email" button that reports the exact SMTP error on failure.
- Use lettre with tokio + rustls.

2. Built-in "Email on 5xx" rule
- Every new project gets this rule, enabled once SMTP is configured. Its filter is an editable query-language expression: `status >= 500 AND environment = 'production'`.
- Recipients: chosen project members and/or plain email addresses.
- It fires in real time, not on the 60 s alert loop. After a successful flush, ingest hands matching 5xx events to the alert engine (in-process channel in `all` mode, Redis in split mode). Target: email sent within 30 s of the error.

3. Noise control (grouped by error_fingerprint, so one bug ≠ 500 emails)
- First occurrence of an error group → send immediately.
- After that, at most one email per error group per 15 min (configurable). Send a follow-up digest only if it kept happening: "42 more in the last 15 min, 17 users affected".
- A global cap of 30 emails per project per hour (configurable). When the cap is hit, send one "notifications paused, N errors suppressed" email.
- Email again when a group that went quiet for 24 h comes back (a regression).
- Each email links to "Mute this error for 1h / 24h / forever" (signed link, works without logging in, expires).

4. Email content (HTML + plain-text parts, dark-on-light, readable on a phone)
- Subject: `[Logbox] 500 POST /orders/:id/checkout — PaymentTimeoutError · orders-svc` (route pattern, never a raw path with IDs).
- Body: time, environment, service, release, method + route, status, duration, request ID, user ID, error type + message, top 5 stack frames, occurrences and users affected so far, and a button to the request inspector plus a link to the error group.
- Never include request or response bodies, headers, or IP addresses in emails.

5. Delivery
- Send from a persistent queue (Postgres table) with retries: exponential backoff, max 5 attempts.
- Record every send (time, recipient, rule, group, result, SMTP response) and show the log under Alerts → History.

6. Dev and tests
- Add Mailpit to a `dev` profile in docker-compose so emails can be viewed locally.
- Write unit tests for throttling, digests, the cap and regressions.
- Write an integration test that ingests a 500 and asserts the email arrives in Mailpit with the right subject and no body/header data.
- Document everything in docs/alerts.md and docs/configuration.md.
