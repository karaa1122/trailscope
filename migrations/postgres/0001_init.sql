-- Trailscope metadata schema. Telemetry lives in ClickHouse; this holds users,
-- projects, keys, views and configuration.

CREATE TABLE users (
    id              UUID PRIMARY KEY,
    email           TEXT NOT NULL,
    name            TEXT NOT NULL DEFAULT '',
    password_hash   TEXT,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    can_view_bodies BOOLEAN NOT NULL DEFAULT FALSE,
    oidc_subject    TEXT UNIQUE,
    disabled        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at   TIMESTAMPTZ
);
CREATE UNIQUE INDEX users_email_lower ON users (lower(email));

CREATE TABLE sessions (
    -- SHA-256 of the session token; the token itself only lives in the cookie.
    id          TEXT PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    csrf_token  TEXT NOT NULL,
    user_agent  TEXT NOT NULL DEFAULT '',
    ip          TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

CREATE TABLE invites (
    token_hash      TEXT PRIMARY KEY,
    email           TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('admin', 'member', 'viewer')),
    can_view_bodies BOOLEAN NOT NULL DEFAULT FALSE,
    created_by      UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    accepted_at     TIMESTAMPTZ
);

CREATE TABLE projects (
    id          UUID PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    -- Sampling defaults advertised to SDKs, body capture policy, IP masking.
    settings    JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE environments (
    project_id  UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, name)
);

CREATE TABLE ingest_keys (
    id           UUID PRIMARY KEY,
    project_id   UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    -- First characters of the plaintext, shown in the UI to tell keys apart.
    prefix       TEXT NOT NULL,
    created_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);
CREATE INDEX ingest_keys_project ON ingest_keys (project_id);

CREATE TABLE api_tokens (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_id   UUID REFERENCES projects (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    prefix       TEXT NOT NULL,
    scopes       TEXT[] NOT NULL DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    expires_at   TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

CREATE TABLE redaction_rules (
    id          UUID PRIMARY KEY,
    project_id  UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    -- header: header name; body_key: JSON key (any depth) or dotted path; regex: value pattern
    kind        TEXT NOT NULL CHECK (kind IN ('header', 'body_key', 'regex')),
    pattern     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX redaction_rules_project ON redaction_rules (project_id);

CREATE TABLE releases (
    id           UUID PRIMARY KEY,
    project_id   UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    environment  TEXT NOT NULL DEFAULT '',
    service      TEXT NOT NULL,
    version      TEXT NOT NULL,
    commit_sha   TEXT,
    url          TEXT,
    released_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX releases_project_time ON releases (project_id, released_at DESC);

CREATE TABLE saved_views (
    id          UUID PRIMARY KEY,
    project_id  UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    owner_id    UUID REFERENCES users (id) ON DELETE SET NULL,
    name        TEXT NOT NULL,
    -- {"mode": "sql" | "search" | "filters", "text": "...", "chips": [...]}
    query       JSONB NOT NULL,
    -- {"relative": "15m"} or {"from": "...", "to": "..."}
    time_range  JSONB NOT NULL DEFAULT '{}'::jsonb,
    columns     JSONB NOT NULL DEFAULT '[]'::jsonb,
    shared      BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX saved_views_project ON saved_views (project_id);

CREATE TABLE replay_targets (
    id          UUID PRIMARY KEY,
    project_id  UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    base_url    TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE TABLE replays (
    id               UUID PRIMARY KEY,
    project_id       UUID NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    request_id       TEXT NOT NULL,
    target_id        UUID REFERENCES replay_targets (id) ON DELETE SET NULL,
    user_id          UUID REFERENCES users (id) ON DELETE SET NULL,
    method           TEXT NOT NULL,
    url              TEXT NOT NULL,
    response_status  INTEGER,
    response_headers JSONB,
    response_body    TEXT,
    duration_ms      DOUBLE PRECISION,
    error            TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX replays_request ON replays (project_id, request_id);

-- Install-wide key/value settings (e.g. setup state).
CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
