-- Alerts: rules, channels, the built-in "Email on 5xx" rule, the notification
-- outbox (also the send history) and per-error-group email state.

CREATE TABLE alert_channels (
    id          UUID PRIMARY KEY,
    project_id  UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('webhook', 'slack', 'telegram', 'email')),
    -- Non-secret settings shown in the UI (e.g. Telegram chat ID, email recipients).
    config      JSONB NOT NULL DEFAULT '{}',
    -- Encrypted JSON with secrets (webhook URL and signing secret, Slack URL, bot token).
    secret      BYTEA,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX alert_channels_project ON alert_channels (project_id);

CREATE TABLE alert_rules (
    id                UUID PRIMARY KEY,
    project_id        UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    kind              TEXT NOT NULL CHECK (kind IN ('count', 'error_rate', 'p95', 'new_error', 'spiking_error')),
    -- {"table": "requests", "input": <QueryInput>}
    query             JSONB NOT NULL,
    environment       TEXT NOT NULL DEFAULT '',
    threshold         DOUBLE PRECISION NOT NULL DEFAULT 0,
    window_minutes    INT NOT NULL DEFAULT 5 CHECK (window_minutes BETWEEN 1 AND 1440),
    cooldown_minutes  INT NOT NULL DEFAULT 10 CHECK (cooldown_minutes BETWEEN 0 AND 10080),
    channel_ids       UUID[] NOT NULL DEFAULT '{}',
    enabled           BOOLEAN NOT NULL DEFAULT true,
    state             TEXT NOT NULL DEFAULT 'ok' CHECK (state IN ('ok', 'firing')),
    last_value        DOUBLE PRECISION,
    last_error        TEXT,
    last_evaluated_at TIMESTAMPTZ,
    state_changed_at  TIMESTAMPTZ,
    last_notified_at  TIMESTAMPTZ,
    created_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX alert_rules_project ON alert_rules (project_id);

-- The built-in "Email on 5xx" rule, one per project. It only sends to the
-- listed recipients: members (by ID) and plain addresses.
CREATE TABLE email_rules (
    project_id        UUID PRIMARY KEY REFERENCES projects (id) ON DELETE CASCADE,
    enabled           BOOLEAN NOT NULL DEFAULT true,
    filter            TEXT NOT NULL DEFAULT 'status >= 500 AND environment = ''production''',
    member_ids        UUID[] NOT NULL DEFAULT '{}',
    emails            TEXT[] NOT NULL DEFAULT '{}',
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO email_rules (project_id) SELECT id FROM projects ON CONFLICT DO NOTHING;

-- Per error group state for the 5xx emails (throttling, regressions, mutes).
CREATE TABLE email_groups (
    project_id     UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    fingerprint    TEXT NOT NULL,
    first_seen_at  TIMESTAMPTZ NOT NULL,
    last_seen_at   TIMESTAMPTZ NOT NULL,
    last_sent_at   TIMESTAMPTZ,
    total          BIGINT NOT NULL DEFAULT 0,
    -- 'infinity' mutes forever.
    muted_until    TIMESTAMPTZ,
    PRIMARY KEY (project_id, fingerprint)
);

-- Outbox and history: one row per message per recipient or channel.
CREATE TABLE notifications (
    id               UUID PRIMARY KEY,
    project_id       UUID REFERENCES projects (id) ON DELETE CASCADE,
    -- 'rule', 'email_5xx' or 'test'
    source           TEXT NOT NULL,
    rule_id          UUID REFERENCES alert_rules (id) ON DELETE SET NULL,
    rule_name        TEXT NOT NULL DEFAULT '',
    channel_id       UUID REFERENCES alert_channels (id) ON DELETE SET NULL,
    channel_kind     TEXT NOT NULL,
    -- Recipient address or channel name.
    target           TEXT NOT NULL,
    -- 'first', 'digest', 'regression', 'paused', 'firing', 'resolved' or 'test'
    reason           TEXT NOT NULL,
    fingerprint      TEXT NOT NULL DEFAULT '',
    subject          TEXT NOT NULL,
    -- The rendered message (no secrets).
    payload          JSONB NOT NULL,
    status           TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'failed')),
    attempts         INT NOT NULL DEFAULT 0,
    next_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error       TEXT,
    response         TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at          TIMESTAMPTZ
);
CREATE INDEX notifications_due ON notifications (next_attempt_at) WHERE status = 'pending';
CREATE INDEX notifications_project ON notifications (project_id, created_at DESC);
