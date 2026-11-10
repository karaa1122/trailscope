# Brief: Trailscope Cloud (hosted, multi-tenant)

Plan for turning Trailscope into a hosted service, written 2026-11-09 after the website started collecting a waitlist with planned prices (decision 065). It follows the same rules as the other briefs: build it milestone by milestone, record judgement calls in [docs/DECISIONS.md](../DECISIONS.md), keep self-hosted Trailscope working unchanged at every step. See [/AGENTS.md](../../AGENTS.md) for how the code is laid out.

## Goal

Anyone can sign up at **cloud.trailscope.pro**, create an organization, get an ingest key and see their requests within minutes, on the Free plan or a paid one (Team $19, Business $99 a month, planned prices on trailscope.pro/pricing), paid in Iraqi dinars through FIB (see C4). The Cloud runs the same open-source code as self-hosted Trailscope; Cloud-only behaviour (sign-up, plans, billing, limits) is switched on by configuration, not a fork.

Today cloud.trailscope.pro runs one ordinary single-company install for the owner's own projects. The Cloud replaces it in place: its projects become the first organization.

## Principles

1. **One binary, one schema.** `[cloud] enabled = true` (`TRAILSCOPE_CLOUD_ENABLED`) turns on sign-up, plans, metering and billing. With it off, Trailscope behaves exactly as today: one organization, created by the setup wizard, no limits, no organization UI unless a second organization exists.
2. **Tenant isolation is enforced in one place.** Every Postgres query that touches tenant data filters by `org_id`, through helpers that make the unscoped version hard to write; every ClickHouse query is already scoped to `project_id`, and the project must belong to the caller's organization. A cross-tenant test suite covers every API route.
3. **The ingest path stays fast.** Plan and usage checks happen against the in-memory key cache (key → project → org → plan, usage), refreshed every few seconds, never a database round trip per batch.
4. **Fail toward the customer's app.** When an organization is over its limit, ingest answers `429` with `Retry-After`; the SDKs already back off and drop quietly. A limit must never slow down or break the customer's service.
5. **Collect less.** The Cloud stores what self-hosted stores, with the same double redaction; bodies keep a shorter retention than metadata, as on the pricing page.

## Milestones

Each milestone ends with everything building, linting and its tests passing (cross-tenant tests are mandatory from C1 on, whatever the general test policy), then a commit.

### C1 · Organizations (about 2 weeks) · done (decision 070)

- Postgres: `organizations (id, slug, name, plan, created_at, …)`, `memberships (org_id, user_id, role, can_view_bodies)` replacing `users.role` and `users.can_view_bodies`; `org_id` on `projects`, `invites`, `api_tokens`, `alert_channels`, `email_rules` and organization-level `settings`. A migration creates one organization ("Default"), moves every project, user, invite and token into it, and copies each user's role into their membership.
- Sessions carry the current organization; a user can belong to several and switch in the UI (organization switcher in the shell, hidden with one organization). API tokens belong to one organization.
- `auth::require(Role, Scope)` checks the membership in the current organization. Every route that takes a project ID checks the project's organization.
- Self-hosted SMTP and OIDC settings stay instance-wide (Settings → Instance, owner of the first organization only).
- **Done when:** a test creates two organizations with a project each and proves, route by route (generated from the OpenAPI document), that a member of one can't read, list, change or query anything of the other, including live tail, saved views, journeys, replay and alerts.

### C2 · Sign-up and accounts (about 1 week)

- `/signup` (email, password, organization name) when Cloud is enabled; the email must be verified before ingest keys work. Password reset by email. Emails go through the instance SMTP settings (Resend in production).
- Every new organization starts on Free with one project and one ingest key, and the setup wizard's "send your first request" step.
- Abuse limits: sign-ups per IP per hour, a disposable-domain blocklist, Cloudflare Turnstile on the form (one script, only on that page).
- Account deletion and organization deletion (with a 7-day grace period, then hard delete in Postgres and `DELETE … WHERE project_id IN …` in ClickHouse).
- **Done when:** a new user can go from the sign-up form to their first request in the live tail without anyone's help, and the deletion job leaves no rows behind.

### C3 · Plans, metering and limits (about 1.5 weeks) · partly done (decision 071)

Done: plans as editable tiers of limits (data, not code), metering into `usage_daily`, the hard monthly limit and suspension at ingest, project and member caps, the usage panel in Settings, and a management API for them (decision 072). Prices, invoices and the admin dashboard live in the private `trailscope-admin` repository. Left: per-plan retention, the 80%/100% emails, the per-organization token bucket, and ClickHouse reconciliation of the counters.

- `plans` are code, not data: Free (1M events a month, 7 days, 3 days of bodies, 2 projects, 3 members), Team (10M, 30 days, 7 days of bodies, unlimited projects and members), Business (50M, 90 days, 14 days of bodies, SSO). An event is a request, a job run or a log line; spans and errors inside a request are free.
- Metering: the ingest pipeline counts accepted events per organization and flushes the counters to Postgres (`usage_daily (org_id, day, events, bytes)`) every 10 s; a ClickHouse rollup per project and day is the source for reconciliation.
- Limits at ingest, from the key cache: Free stops accepting at 100% of the month (`429`, `Retry-After` to the next month); paid plans keep accepting and count overage. Emails at 80% and 100%.
- Fair sharing: a per-organization token bucket in front of the shared ingest queue (burst sized by plan), so one organization can't fill the queue for everyone.
- Retention per plan: add `expires_at` and `body_expires_at` columns to `requests`, `spans` and `events`, set at insert from the plan, and change the table TTLs to use them (`TTL expires_at DELETE`, body columns cleared at `body_expires_at`). Self-hosted sets them from `retention.days` as today.
- Usage page in Settings: events this month, by project, against the plan.
- **Done when:** an organization over its Free limit gets `429`s while another organization on the same server is unaffected, and rows of a 7-day organization disappear while a 30-day organization's rows stay.

### C4 · Billing with FIB (about 1.5 weeks)

The owner bills as an individual through **FIB online payments** (First Iraqi Bank). What FIB's developer documentation says (fib.iq/developers/web-payment, read 2026-11-09):

- OAuth2 client credentials (`client_id`, `client_secret`); access tokens live about 60 seconds, so fetch one per operation and cache it briefly. The credentials decide test or live mode; the sandbox is `https://fib.stage.fib.iq`, the production host comes with the production credentials.
- `POST /protected/v1/payments` with an amount in **IQD only** (no USD), an optional description (50 characters) and an optional `statusCallbackUrl`. The answer has `paymentId`, a QR code, a readable code, app links and `validUntil`. The customer pays in the FIB app by scanning the code, typing the readable code or tapping the link.
- `GET /protected/v1/payments/{id}/status`: `PAID`, `UNPAID` or `DECLINED` (with a reason: expired, cancelled, server failure). `POST …/cancel` cancels an unpaid payment.
- The callback is a POST with only `id` and `status`, **without a signature**. There is no refund API and **no recurring or saved payment**: every period is a new payment the customer approves in the app.

So the Cloud bills **prepaid, one invoice per period**:

- Prices are set in IQD by the owner (the USD prices on the site are a guide until then); monthly or yearly with a discount, since every payment is a manual step for the customer.
- Postgres `invoices (id, org_id, plan, period_start, period_end, amount_iqd, fib_payment_id, status, valid_until, paid_at, callback_token)`. Upgrading creates an invoice and an FIB payment; the billing page shows the QR code, the readable code and the app link, and polls until paid.
- A callback is only a hint: the URL carries a random per-invoice token (unknown tokens are ignored), and any callback, plus a sweep of open invoices every 5 minutes, triggers a status call to FIB with our own token. Only that answer changes an invoice. Payments are matched by `paymentId` and amount.
- Renewal: 7 days before the period ends, the next invoice is created and its pay link emailed (Resend); 7 days of grace after the due date, then Free limits; data beyond Free retention is kept 14 more days in case they pay.
- Overage (events above the plan) is counted during the period and added to the next invoice.
- Refunds are manual (bank transfer by the owner) and recorded on the invoice.
- **Done when:** in FIB's sandbox an organization can upgrade by paying a QR code, renew, go past due and fall back to Free; every state change comes from a status call; a callback for an unknown or unpaid payment changes nothing.

Open with FIB before building: that an individual can get production credentials (their docs speak of shops and business/corporate apps), fees per payment, the production host, how long a payment stays valid, and whether the callback is retried.

### C5 · Safety (about 1 week)

- Replay: resolve the target's address and refuse loopback, private, link-local, carrier-grade NAT and cloud metadata ranges (IPv4 and IPv6), checked again on connect; replay targets only on paid plans.
- Per-organization limits on concurrent queries and API requests (on top of the existing ClickHouse reader caps).
- Webhook alert channels get the same address check as replay.
- An audit log per organization: sign-ins, member and role changes, keys and tokens created or revoked, exports, deletions.
- **Done when:** the replay and webhook checks are covered by tests with every blocked range, and a security review of the diff since C1 finds no open issue.

### C6 · Operations (about 1 week, in parallel with C4/C5)

- Hosting: the owner's Oracle Cloud account, region Frankfurt (EU, as the pricing page promises). Not the VM that already runs the owner's other production sites (its two cores are often saturated by another project): a separate Ampere A1 instance, which needs the Pay As You Go account because the free A1 allowance is used. Start with 4 OCPU / 24 GB and a block volume of 200 GB or more for ClickHouse, running Trailscope in split mode (ingest, API and alerts as separate containers, Redis for live tail) with ClickHouse and Postgres on the same instance; grow the shape, then split ClickHouse out, when load requires it. Set an Oracle budget alert.
- Backups: ClickHouse `BACKUP … TO S3` nightly and Postgres base backups plus WAL, both to Oracle Object Storage (S3-compatible API) in another region; a monthly restore test.
- Monitoring: `/metrics` into Prometheus and Grafana, uptime checks from outside, a public status page (status.trailscope.pro). The Cloud logs its own API into a second Trailscope instance.
- Deploys: the GHCR image from the release workflow (decision 068), pinned by tag, rolled out with `docker compose pull && up -d`; ingest stays up during deploys (the SDKs retry, and nginx in front can queue for a few seconds).
- **Done when:** a restore from last night's backup has been done once on a spare machine, and an outage page fires within 2 minutes of stopping the API.

### C7 · Legal and launch (about 1 week, mostly writing)

- Terms of service, an updated privacy policy, a data processing agreement (GDPR Art. 28) with the list of subprocessors (Oracle Cloud for hosting and backups, FIB for payments, Resend for email), and a short security page.
- Support: hello@trailscope.pro (Resend inbox), response targets per plan.
- Launch order: private beta for the waitlist on Free and Team without charging (after C1–C3, C5, C6), then paid plans (after C4, C7). The owner decides when the landing page changes from the waitlist to sign-up; until then trailscope.pro stays as it is.

## Rough cost and price check

- Storage is small: about 70 bytes per request in ClickHouse (docs/benchmarks.md), so a Team organization at its full 10M events for 30 days keeps under 1 GB of metadata plus bodies for 7 days.
- The fixed cost dominates at the start: one dedicated EU server and object storage. A handful of Team subscriptions covers it; Free organizations cost mainly disk, bounded by their 7-day retention and 1M events.
- FIB's fee per payment comes out of the price (ask FIB). Selling as an individual: check with an accountant what to declare, and whether selling to customers in the EU creates VAT obligations.
- Only FIB users can pay at first, so the paying market is Iraq; customers elsewhere can use Free or self-host until a second payment provider is added.

## Decisions

Made by the owner on 2026-11-09:

1. **License:** the server is AGPL-3.0, the SDKs stay Apache-2.0 (decision 069).
2. **Hosting:** the owner's Oracle Cloud account, Frankfurt.
3. **Who bills:** the owner, as an individual.
4. **Payments:** FIB online payments, in IQD (C4).

Still open:

1. The IQD price list (and the overage price per million events).
2. What happens to Free organizations that only send noise, and when inactive Free organizations are deleted.
3. A second payment provider for customers outside Iraq, if there is demand.
4. FIB's answers on individual merchants, fees and the production host (C4).

## Order and estimate

C1 → C2 → C3 → C5 with C6 alongside: private beta after about 6 weeks of focused work. C4 and C7: paid launch about 2 weeks later, once FIB has issued production credentials.
