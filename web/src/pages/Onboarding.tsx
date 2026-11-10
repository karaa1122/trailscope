import { useState, type FormEvent } from 'react';

import { SecretOnce } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { useSession } from '../lib/session';
import { AuthCard } from './Auth';

const errText = (e: unknown) =>
  e instanceof ApiError || e instanceof Error ? e.message : 'Something went wrong';

/**
 * Shown instead of the app when there is nothing to show yet: the user
 * belongs to no organization, or their organization has no project. Admins
 * can fix that from here; everyone can switch organization or sign out.
 */
export function Onboarding() {
  const { me, refresh, signOut, switchOrg } = useSession();
  const [name, setName] = useState('');
  const [key, setKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const role = me?.user.role;
  const isAdmin = role === 'admin' || role === 'owner';
  const others = (me?.orgs ?? []).filter((o) => o.id !== me?.org?.id);

  const run = async (e: FormEvent, action: () => Promise<void>) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      await action();
    } catch (err) {
      setError(errText(err));
    } finally {
      setBusy(false);
    }
  };

  const footer = (
    <div className="row wrap" style={{ gap: 8, marginTop: 4 }}>
      {others.map((o) => (
        <button
          key={o.id}
          type="button"
          className="btn btn-sm"
          onClick={() => void switchOrg(o.id)}
        >
          Switch to {o.name}
        </button>
      ))}
      {me?.instanceAdmin && (
        <a href="/admin" className="btn btn-sm">
          Admin console
        </a>
      )}
      <button
        type="button"
        className="btn btn-sm"
        style={{ marginLeft: 'auto' }}
        onClick={() => void signOut()}
      >
        Sign out
      </button>
    </div>
  );

  if (!me?.org) {
    return (
      <AuthCard
        title="You're not in an organization yet"
        subtitle={
          me?.instanceAdmin
            ? 'Create one to start, then invite your team from Settings → Members.'
            : 'Ask an admin of your team for an invite link.'
        }
      >
        {me?.instanceAdmin && (
          <form
            onSubmit={(e) =>
              void run(e, async () => {
                await api.createOrg(name.trim());
                window.location.assign('/live');
              })
            }
            style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
          >
            <label className="field">
              Organization name
              <input
                className="input"
                value={name}
                onChange={(e) => setName(e.target.value)}
                required
                maxLength={100}
                autoFocus
              />
            </label>
            {error && (
              <span className="err-text" role="alert">
                {error}
              </span>
            )}
            <button type="submit" className="btn btn-primary" disabled={busy}>
              Create organization
            </button>
          </form>
        )}
        {footer}
      </AuthCard>
    );
  }

  if (key) {
    return (
      <AuthCard
        title="Project created"
        subtitle="Send requests with this ingest key. It is shown only once; more keys can be made in Settings."
      >
        <SecretOnce label="Ingest key." value={key} onDone={() => void refresh()} />
        <button type="button" className="btn btn-primary" onClick={() => void refresh()}>
          Continue
        </button>
      </AuthCard>
    );
  }

  return (
    <AuthCard
      title={`No projects in ${me.org.name} yet`}
      subtitle={
        isAdmin
          ? 'Create the first one; each project has its own ingest keys and data.'
          : 'Ask an admin to create one.'
      }
    >
      {isAdmin && (
        <form
          onSubmit={(e) =>
            void run(e, async () => {
              const r = await api.createProject(name.trim());
              setKey(r.ingestKey);
            })
          }
          style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
        >
          <label className="field">
            Project name
            <input
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Orders API"
              required
              maxLength={100}
              autoFocus
            />
          </label>
          {error && (
            <span className="err-text" role="alert">
              {error}
            </span>
          )}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            Create project
          </button>
        </form>
      )}
      {footer}
    </AuthCard>
  );
}
