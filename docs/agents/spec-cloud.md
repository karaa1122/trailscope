# Brief: Trailscope Cloud (hosted, multi-tenant)

Plan for turning Trailscope into a hosted service, written 2026-11-09 after the website started collecting a waitlist with planned prices (decision 065). It follows the same rules as the other briefs: build it milestone by milestone, record judgement calls in [docs/DECISIONS.md](../DECISIONS.md), keep self-hosted Trailscope working unchanged at every step. See [/AGENTS.md](../../AGENTS.md) for how the code is laid out.

## Goal

Anyone can sign up at **cloud.trailscope.pro**, create an organization, get an ingest key and see their requests within minutes, on the Free plan or a paid one (Team $19, Business $99 a month, planned prices on trailscope.pro/pricing). The Cloud runs the same open-source code as self-hosted Trailscope; Cloud-only behaviour (sign-up, plans, billing, limits) is switched on by configuration, not a fork.

Today cloud.trailscope.pro runs one ordinary single-company install for the owner's own projects. The Cloud replaces it in place: its projects become the first organization.

## Principles

1. **One binary, one schema.** `[cloud] enabled = true` (`TRAILSCOPE_CLOUD_ENABLED`) turns on sign-up, plans, metering and billing. With it off, Trailscope behaves exactly as today: one organization, created by the setup wizard, no limits, no organization UI unless a second organization exists.
2. **Tenant isolation is enforced in one place.** Every Postgres query that touches tenant data filters by `org_id`, through helpers that make the unscoped version hard to write; every ClickHouse query is already scoped to `project_id`, and the project must belong to the caller's organization. A cross-tenant test suite covers every API route.
3. **The ingest path stays fast.** Plan and usage checks happen against the in-memory key cache (key → project → org → plan, usage), refreshed every few seconds, never a database round trip per batch.
4. **Fail toward the customer's app.** When an organization is over its limit, ingest answers `429` with `Retry-After`; the SDKs already back off and drop quietly. A limit must never slow down or break the customer's service.
5. **Collect less.** The Cloud stores what self-hosted stores, with the same double redaction; bodies keep a shorter retention than metadata, as on the pricing page.

## Milestones

Each milestone ends with everything building, linting and its tests passing (cross-tenant tests are mandatory from C1 on, whatever the general test policy), then a commit.

### C1 · Organizations (about 2 weeks)

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

### C3 · Plans, metering and limits (about 1.5 weeks)

- `plans` are code, not data: Free (1M events a month, 7 days, 3 days of bodies, 2 projects, 3 members), Team (10M, 30 days, 7 days of bodies, unlimited projects and members), Business (50M, 90 days, 14 days of bodies, SSO). An event is a request, a job run or a log line; spans and errors inside a request are free.
- Metering: the ingest pipeline counts accepted events per organization and flushes the counters to Postgres (`usage_daily (org_id, day, events, bytes)`) every 10 s; a ClickHouse rollup per project and day is the source for reconciliation.
- Limits at ingest, from the key cache: Free stops accepting at 100% of the month (`429`, `Retry-After` to the next month); paid plans keep accepting and count overage. Emails at 80% and 100%.
- Fair sharing: a per-organization token bucket in front of the shared ingest queue (burst sized by plan), so one organization can't fill the queue for everyone.
- Retention per plan: add `expires_at` and `body_expires_at` columns to `requests`, `spans` and `events`, set at insert from the plan, and change the table TTLs to use them (`TTL expires_at DELETE`, body columns cleared at `body_expires_at`). Self-hosted sets them from `retention.days` as today.
- Usage page in Settings: events this month, by project, against the plan.
- **Done when:** an organization over its Free limit gets `429`s while another organization on the same server is unaffected, and rows of a 7-day organization disappear while a 30-day organization's rows stay.

### C4 · Billing (about 1.5 weeks)

- Stripe Checkout for upgrades, the Stripe Customer Portal for card changes, invoices and cancellation; Stripe Tax for EU VAT.
- Webhooks (`checkout.session.completed`, `customer.subscription.*`, `invoice.payment_failed`) update `organizations.plan`, `subscription_status` and `current_period_end`; signed and idempotent.
- Overage: a daily job reports each paid organization's events above its plan as Stripe metered usage ($1.50 per million on Team, $1.20 on Business).
- Failed payments: 7 days of grace, then Free limits; data beyond Free retention is kept 14 more days in case they pay.
- **Done when:** a test-mode card can upgrade, be charged overage, downgrade and cancel, and the plan in Trailscope always matches Stripe.

### C5 · Safety (about 1 week)

- Replay: resolve the target's address and refuse loopback, private, link-local, carrier-grade NAT and cloud metadata ranges (IPv4 and IPv6), checked again on connect; replay targets only on paid plans.
- Per-organization limits on concurrent queries and API requests (on top of the existing ClickHouse reader caps).
- Webhook alert channels get the same address check as replay.
- An audit log per organization: sign-ins, member and role changes, keys and tokens created or revoked, exports, deletions.
- **Done when:** the replay and webhook checks are covered by tests with every blocked range, and a security review of the diff since C1 finds no open issue.

### C6 · Operations (about 1 week, in parallel with C4/C5)

- Hosting: its own machine in the EU, not the shared server that runs other sites. Start with one dedicated server (8+ cores, 64 GB RAM, NVMe) running Trailscope in split mode (ingest, API and alerts as separate containers, Redis for live tail) with ClickHouse and Postgres on the same box; split ClickHouse out when load requires it.
- Backups: ClickHouse `BACKUP … TO S3` nightly and Postgres base backups plus WAL, both to S3-compatible object storage in another location; a monthly restore test.
- Monitoring: `/metrics` into Prometheus and Grafana, uptime checks from outside, a public status page (status.trailscope.pro). The Cloud logs its own API into a second Trailscope instance.
- Deploys: the GHCR image from the release workflow (decision 068), pinned by tag, rolled out with `docker compose pull && up -d`; ingest stays up during deploys (the SDKs retry, and nginx in front can queue for a few seconds).
- **Done when:** a restore from last night's backup has been done once on a spare machine, and an outage page fires within 2 minutes of stopping the API.

### C7 · Legal and launch (about 1 week, mostly writing)

- Terms of service, an updated privacy policy, a data processing agreement (GDPR Art. 28) with the list of subprocessors (hosting provider, Stripe, Resend, object storage), and a short security page.
- Support: hello@trailscope.pro (Resend inbox), response targets per plan.
- Launch order: private beta for the waitlist on Free and Team without charging (after C1–C3, C5, C6), then paid plans (after C4, C7). The owner decides when the landing page changes from the waitlist to sign-up; until then trailscope.pro stays as it is.

## Rough cost and price check

- Storage is small: about 70 bytes per request in ClickHouse (docs/benchmarks.md), so a Team organization at its full 10M events for 30 days keeps under 1 GB of metadata plus bodies for 7 days.
- The fixed cost dominates at the start: one dedicated EU server and object storage. A handful of Team subscriptions covers it; Free organizations cost mainly disk, bounded by their 7-day retention and 1M events.
- Payment fees and VAT handling come out of the price; Stripe Tax handles the VAT rules.

## Decisions the owner has to make

1. **License** before announcing the Cloud: keep Apache-2.0 (anyone may host Trailscope as a competing service), or move to AGPL or FSL for new versions. Changing later only affects new releases and needs agreement from outside contributors.
2. **Hosting provider and region** for the Cloud server (EU required by the pricing page).
3. **Who bills:** Stripe needs a person or a company with a tax status; VAT registration depends on the country.
4. **Free plan abuse policy:** what happens to organizations that only send noise, and when inactive Free organizations are deleted.

## Order and estimate

C1 → C2 → C3 → C5 with C6 alongside: private beta after about 6 weeks of focused work. C4 and C7: paid launch about 2 weeks later.
