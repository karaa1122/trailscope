-- Organizations (decision 070): several companies on one Trailscope, each
-- seeing only its own projects, members, invites and API tokens. A user can
-- belong to several organizations, with a role in each; sessions and API
-- tokens act within one organization at a time.
--
-- Existing installs: everything moves into one organization named "Default",
-- every user keeps their role there, and today's owners become instance admins
-- (the people who manage server-wide settings such as SMTP).

CREATE TABLE organizations (
    id          UUID PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
    org_id          UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    can_view_bodies BOOLEAN NOT NULL DEFAULT FALSE,
    disabled        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);
CREATE INDEX memberships_user ON memberships (user_id);

ALTER TABLE users ADD COLUMN instance_admin BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN last_org_id UUID REFERENCES organizations (id) ON DELETE SET NULL;

ALTER TABLE projects ADD COLUMN org_id UUID REFERENCES organizations (id) ON DELETE CASCADE;
ALTER TABLE invites ADD COLUMN org_id UUID REFERENCES organizations (id) ON DELETE CASCADE;
ALTER TABLE api_tokens ADD COLUMN org_id UUID REFERENCES organizations (id) ON DELETE CASCADE;
-- The organization a browser session is working in.
ALTER TABLE sessions ADD COLUMN org_id UUID REFERENCES organizations (id) ON DELETE SET NULL;

DO $$
DECLARE
    org UUID := gen_random_uuid();
BEGIN
    IF EXISTS (SELECT 1 FROM users) OR EXISTS (SELECT 1 FROM projects) THEN
        INSERT INTO organizations (id, slug, name) VALUES (org, 'default', 'Default');
        INSERT INTO memberships (org_id, user_id, role, can_view_bodies, disabled, created_at)
            SELECT org, id, role, can_view_bodies, disabled, created_at FROM users;
        UPDATE users SET instance_admin = TRUE, last_org_id = org WHERE role = 'owner';
        UPDATE users SET last_org_id = org;
        UPDATE projects SET org_id = org;
        UPDATE invites SET org_id = org;
        UPDATE api_tokens SET org_id = org;
        UPDATE sessions SET org_id = org;
    END IF;
END $$;

ALTER TABLE projects ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE invites ALTER COLUMN org_id SET NOT NULL;
ALTER TABLE api_tokens ALTER COLUMN org_id SET NOT NULL;
CREATE INDEX projects_org ON projects (org_id);
CREATE INDEX api_tokens_org ON api_tokens (org_id);

-- Project slugs are unique within an organization, not across the server.
ALTER TABLE projects DROP CONSTRAINT projects_slug_key;
ALTER TABLE projects ADD CONSTRAINT projects_org_slug UNIQUE (org_id, slug);

-- Roles and the body permission now live on the membership.
ALTER TABLE users DROP COLUMN role;
ALTER TABLE users DROP COLUMN can_view_bodies;
