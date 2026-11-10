-- Plans, usage and invoices (decision 071), managed by instance admins in the
-- admin console. Plans are data, so the operator can add their own; limits are
-- only enforced when `cloud.enabled` is on. Usage is metered either way.

CREATE TABLE plans (
    id               TEXT PRIMARY KEY CHECK (id ~ '^[a-z0-9][a-z0-9-]{0,39}$'),
    name             TEXT NOT NULL,
    -- Monthly price in Iraqi dinars (FIB bills in IQD only).
    price_iqd        BIGINT NOT NULL DEFAULT 0 CHECK (price_iqd >= 0),
    -- Requests, job runs and log lines per calendar month (UTC); NULL = unlimited.
    events_per_month BIGINT CHECK (events_per_month IS NULL OR events_per_month >= 0),
    -- TRUE: ingest stops at the limit until the next month. FALSE: it keeps
    -- accepting and the excess is overage.
    hard_limit       BOOLEAN NOT NULL DEFAULT TRUE,
    max_projects     INT CHECK (max_projects IS NULL OR max_projects >= 1),
    max_members      INT CHECK (max_members IS NULL OR max_members >= 1),
    -- Archived plans can't be given to more organizations; existing ones keep it.
    archived         BOOLEAN NOT NULL DEFAULT FALSE,
    position         INT NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The plans on trailscope.pro/pricing; prices are set by the operator.
INSERT INTO plans (id, name, events_per_month, hard_limit, max_projects, max_members, position) VALUES
    ('free', 'Free', 1000000, TRUE, 2, 3, 0),
    ('team', 'Team', 10000000, FALSE, NULL, NULL, 1),
    ('business', 'Business', 50000000, FALSE, NULL, NULL, 2);

ALTER TABLE organizations ADD COLUMN plan_id TEXT NOT NULL DEFAULT 'free' REFERENCES plans (id);
-- A suspended organization's ingest keys are refused; its members can still sign in.
ALTER TABLE organizations ADD COLUMN suspended_at TIMESTAMPTZ;
-- Only instance admins see this.
ALTER TABLE organizations ADD COLUMN admin_note TEXT NOT NULL DEFAULT '';

-- The admin console invites a new organization's first owner.
ALTER TABLE invites DROP CONSTRAINT invites_role_check;
ALTER TABLE invites ADD CONSTRAINT invites_role_check
    CHECK (role IN ('owner', 'admin', 'member', 'viewer'));

-- Accepted events per organization and UTC day, flushed by ingest every few seconds.
CREATE TABLE usage_daily (
    org_id  UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    day     DATE NOT NULL,
    events  BIGINT NOT NULL DEFAULT 0,
    bytes   BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (org_id, day)
);

-- Recorded by hand until FIB payments are wired in (Cloud milestone C4).
CREATE TABLE invoices (
    id            UUID PRIMARY KEY,
    org_id        UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    plan_id       TEXT NOT NULL REFERENCES plans (id),
    period_start  DATE NOT NULL,
    period_end    DATE NOT NULL CHECK (period_end >= period_start),
    amount_iqd    BIGINT NOT NULL CHECK (amount_iqd >= 0),
    status        TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'paid', 'void')),
    note          TEXT NOT NULL DEFAULT '',
    paid_at       TIMESTAMPTZ,
    created_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX invoices_org ON invoices (org_id, created_at DESC);
