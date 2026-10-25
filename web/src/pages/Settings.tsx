import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';

import { Empty, SecretOnce, Segmented, Skeleton, Toast, useToast } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtAgo, fmtDateTime } from '../lib/format';
import { useProject, useSession } from '../lib/session';
import type { Project, ProjectSettings, RedactionRule, Role } from '../lib/types';
import { NotificationsSection } from './SettingsNotifications';

export type SettingsTab = 'project' | 'keys' | 'redaction' | 'replay' | 'members' | 'tokens' | 'notifications' | 'retention' | 'account';

const RANK: Record<Role, number> = { viewer: 0, member: 1, admin: 2, owner: 3 };
const ROLES: Role[] = ['viewer', 'member', 'admin', 'owner'];

const TABS: { id: SettingsTab; label: string; min: Role }[] = [
  { id: 'project', label: 'Project', min: 'viewer' },
  { id: 'keys', label: 'Ingest keys', min: 'admin' },
  { id: 'redaction', label: 'Redaction', min: 'admin' },
  { id: 'replay', label: 'Replay targets', min: 'admin' },
  { id: 'members', label: 'Members', min: 'admin' },
  { id: 'tokens', label: 'API tokens', min: 'viewer' },
  { id: 'notifications', label: 'Notifications', min: 'admin' },
  { id: 'retention', label: 'Retention', min: 'viewer' },
  { id: 'account', label: 'Account', min: 'viewer' },
];

type Toaster = (msg: string) => void;

const errText = (e: unknown) => (e instanceof ApiError || e instanceof Error ? e.message : 'Something went wrong');
const when = (iso: string | null) => (iso ? fmtAgo(Date.parse(iso)) : 'never');

export function Settings({ tab }: { tab?: SettingsTab }) {
  const { me } = useSession();
  const navigate = useNavigate();
  const [toast, setToast] = useToast();
  const role = me?.user.role ?? 'viewer';
  const tabs = TABS.filter((t) => RANK[role] >= RANK[t.min]);
  const current = tabs.find((t) => t.id === tab)?.id ?? 'project';
  const isAdmin = RANK[role] >= RANK.admin;

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Settings</h1>
      </header>
      <div className="row wrap" style={{ gap: 18, alignItems: 'flex-start' }}>
        <nav aria-label="Settings sections" style={{ flex: '0 1 190px', display: 'flex', flexDirection: 'column', gap: 2 }} className="nav">
          {tabs.map((t) => (
            <a
              key={t.id}
              href={`/settings?tab=${t.id}`}
              className={t.id === current ? 'active' : undefined}
              aria-current={t.id === current ? 'page' : undefined}
              onClick={(e) => {
                e.preventDefault();
                void navigate({ to: '/settings', search: { tab: t.id } });
              }}
            >
              {t.label}
            </a>
          ))}
        </nav>
        <div style={{ flex: '999 1 560px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 16 }}>
          {current === 'project' && <ProjectSection toast={setToast} canEdit={isAdmin} isOwner={role === 'owner'} />}
          {current === 'keys' && <KeysSection toast={setToast} />}
          {current === 'redaction' && <RedactionSection toast={setToast} />}
          {current === 'replay' && <ReplaySection toast={setToast} />}
          {current === 'members' && <MembersSection toast={setToast} isOwner={role === 'owner'} />}
          {current === 'tokens' && <TokensSection toast={setToast} role={role} />}
          {current === 'notifications' && <NotificationsSection toast={setToast} />}
          {current === 'retention' && <RetentionSection />}
          {current === 'account' && <AccountSection toast={setToast} />}
        </div>
      </div>
      <Toast msg={toast} />
    </main>
  );
}

function Panel({ title, sub, children, actions }: { title: string; sub?: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <section aria-label={title} className="panel" style={{ overflow: 'hidden' }}>
      <div className="row wrap" style={{ gap: 12, padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
        <h2 className="panel-title">{title}</h2>
        {sub && <span className="panel-sub">{sub}</span>}
        {actions && <span style={{ marginLeft: 'auto' }}>{actions}</span>}
      </div>
      {children}
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="field">
      {label}
      {children}
      {hint && <span className="dim" style={{ fontSize: 11.5 }}>{hint}</span>}
    </label>
  );
}

const formStyle = { padding: 18, display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 16 } as const;

// ---- project ---------------------------------------------------------------------

function ProjectSection({ toast, canEdit, isOwner }: { toast: Toaster; canEdit: boolean; isOwner: boolean }) {
  const project = useProject();
  const { refresh, projects, setProject } = useSession();
  const [name, setName] = useState(project.name);
  const [s, setS] = useState<ProjectSettings>(project.settings);
  const [rates, setRates] = useState(() => formatRates(project.settings.routeSampleRates));
  const [ignore, setIgnore] = useState(project.settings.ignoreRoutes.join('\n'));
  const [created, setCreated] = useState<{ name: string; key: string } | null>(null);
  useEffect(() => {
    setName(project.name);
    setS(project.settings);
    setRates(formatRates(project.settings.routeSampleRates));
    setIgnore(project.settings.ignoreRoutes.join('\n'));
  }, [project]);

  const save = useMutation({
    mutationFn: () => {
      const routeSampleRates = parseRates(rates);
      return api.updateProject(project.id, {
        name: name.trim(),
        settings: { ...s, routeSampleRates, ignoreRoutes: ignore.split('\n').map((l) => l.trim()).filter(Boolean) },
      });
    },
    onSuccess: async () => {
      await refresh();
      toast('Project saved');
    },
    onError: (e) => toast(errText(e)),
  });
  const create = useMutation({
    mutationFn: (n: string) => api.createProject(n),
    onSuccess: async (r) => {
      await refresh();
      setCreated({ name: r.project.name, key: r.ingestKey });
    },
    onError: (e) => toast(errText(e)),
  });
  const remove = useMutation({
    mutationFn: (p: Project) => api.deleteProject(p.id),
    onSuccess: async () => {
      const next = projects.find((p) => p.id !== project.id);
      if (next) setProject(next.id);
      await refresh();
      toast('Project deleted');
    },
    onError: (e) => toast(errText(e)),
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    try {
      parseRates(rates);
    } catch (err) {
      toast(errText(err));
      return;
    }
    save.mutate();
  };

  return (
    <>
      <Panel title="Project" sub={<span className="mono">{project.slug}</span>}>
        <form onSubmit={submit}>
          <fieldset disabled={!canEdit} style={{ border: 0, margin: 0, padding: 0 }}>
            <div style={formStyle}>
              <Field label="Name">
                <input className="input" value={name} onChange={(e) => setName(e.target.value)} required maxLength={100} />
              </Field>
              <Field label="Body capture" hint="Bodies are stored for errors only by default.">
                <Segmented
                  small
                  label="Body capture"
                  value={s.bodyCapture}
                  onChange={(v) => setS({ ...s, bodyCapture: v })}
                  options={[
                    { id: 'errors', label: 'errors' },
                    { id: 'always', label: 'always' },
                    { id: 'never', label: 'never' },
                  ]}
                />
              </Field>
              <Field label="Slow threshold (ms)" hint="Requests at or above this count as slow and as over SLO.">
                <input className="input mono" type="number" min={1} max={600000} value={s.slowMs} onChange={(e) => setS({ ...s, slowMs: Number(e.target.value) })} />
              </Field>
              <Field label="Sample rate" hint="Share of fast, successful requests SDKs keep; 4xx, 5xx and slow requests are always kept.">
                <input className="input mono" type="number" min={0} max={1} step={0.01} value={s.sampleRate} onChange={(e) => setS({ ...s, sampleRate: Number(e.target.value) })} />
              </Field>
              <Field label="Max body size (bytes)">
                <input className="input mono" type="number" min={0} max={1048576} value={s.maxBodyBytes} onChange={(e) => setS({ ...s, maxBodyBytes: Number(e.target.value) })} />
              </Field>
              <Field label="Mask client IPs" hint="Keeps the first three IPv4 octets or the first 48 bits of an IPv6 address.">
                <label className="row" style={{ minHeight: 44, color: 'var(--text)' }}>
                  <input type="checkbox" checked={s.maskIp} onChange={(e) => setS({ ...s, maskIp: e.target.checked })} /> mask IPs at ingest
                </label>
              </Field>
              <Field label="Per-route sample rates" hint="One per line: GET /healthz = 0.01">
                <textarea className="textarea mono" rows={4} value={rates} onChange={(e) => setRates(e.target.value)} style={{ padding: 10 }} />
              </Field>
              <Field label="Ignored routes" hint="One route pattern per line; SDKs skip these.">
                <textarea className="textarea mono" rows={4} value={ignore} onChange={(e) => setIgnore(e.target.value)} style={{ padding: 10 }} />
              </Field>
            </div>
            {canEdit && (
              <div className="row" style={{ padding: '0 18px 18px' }}>
                <button type="submit" className="btn btn-primary" disabled={save.isPending}>
                  Save project
                </button>
              </div>
            )}
          </fieldset>
        </form>
      </Panel>

      {isOwner && (
        <Panel title="All projects" sub={`${projects.length} in this install`}>
          {created && (
            <div style={{ padding: 18 }}>
              <SecretOnce label={`Project “${created.name}” created. Its first ingest key:`} value={created.key} onDone={() => setCreated(null)} />
            </div>
          )}
          {projects.map((p) => (
            <div key={p.id} className="kv" style={{ alignItems: 'center' }}>
              <span style={{ fontSize: 13, color: 'var(--text)' }}>
                {p.name} <span className="mono dim">{p.slug}</span>
              </span>
              <span className="row" style={{ gap: 6 }}>
                {p.id !== project.id && (
                  <button type="button" className="btn btn-sm" onClick={() => setProject(p.id)}>
                    Switch
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={projects.length < 2}
                  title={projects.length < 2 ? 'The last project cannot be deleted' : undefined}
                  onClick={() => window.prompt(`This deletes all data for “${p.name}”. Type its slug to confirm.`) === p.slug && remove.mutate(p)}
                >
                  Delete
                </button>
              </span>
            </div>
          ))}
          <form
            className="row"
            style={{ padding: 18 }}
            onSubmit={(e) => {
              e.preventDefault();
              const input = e.currentTarget.elements.namedItem('name') as HTMLInputElement;
              if (input.value.trim()) create.mutate(input.value.trim());
              input.value = '';
            }}
          >
            <input name="name" className="input" placeholder="New project name" aria-label="New project name" style={{ maxWidth: 320 }} />
            <button type="submit" className="btn">
              Create project
            </button>
          </form>
        </Panel>
      )}
    </>
  );
}

function formatRates(r: Record<string, number>): string {
  return Object.entries(r)
    .map(([k, v]) => `${k} = ${v}`)
    .join('\n');
}

function parseRates(text: string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const line of text.split('\n').map((l) => l.trim()).filter(Boolean)) {
    const i = line.lastIndexOf('=');
    const rate = Number(line.slice(i + 1));
    if (i < 1 || !Number.isFinite(rate) || rate < 0 || rate > 1) throw new Error(`Per-route sample rate “${line}” must look like “GET /path = 0.5”`);
    out[line.slice(0, i).trim()] = rate;
  }
  return out;
}

// ---- ingest keys -------------------------------------------------------------------

function KeysSection({ toast }: { toast: Toaster }) {
  const project = useProject();
  const qc = useQueryClient();
  const [secret, setSecret] = useState<string | null>(null);
  const keys = useQuery({ queryKey: ['keys', project.id], queryFn: () => api.keys(project.id) });
  const refresh = () => void qc.invalidateQueries({ queryKey: ['keys', project.id] });
  const create = useMutation({
    mutationFn: (name: string) => api.createKey(project.id, name),
    onSuccess: (r) => {
      setSecret(r.token);
      refresh();
    },
    onError: (e) => toast(errText(e)),
  });
  const revoke = useMutation({ mutationFn: (id: string) => api.revokeKey(project.id, id), onSuccess: refresh, onError: (e) => toast(errText(e)) });
  return (
    <Panel title="Ingest keys" sub="SDKs send events with these; revoking takes effect within a minute">
      {secret && (
        <div style={{ padding: 18 }}>
          <SecretOnce label="New ingest key." value={secret} onDone={() => setSecret(null)} />
        </div>
      )}
      {keys.isLoading && <Skeleton h={80} />}
      {keys.data?.length === 0 && <Empty title="No keys" />}
      {keys.data?.map((k) => (
        <div key={k.id} className="kv" style={{ alignItems: 'center', opacity: k.revokedAt ? 0.55 : 1 }}>
          <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span style={{ color: 'var(--text)', fontSize: 13 }}>{k.name}</span>
            <span className="mono dim" style={{ fontSize: 11.5 }}>
              {k.prefix} · created {when(k.createdAt)} · last used {when(k.lastUsedAt)}
            </span>
          </span>
          {k.revokedAt ? (
            <span className="muted">revoked {when(k.revokedAt)}</span>
          ) : (
            <button type="button" className="btn btn-sm" onClick={() => window.confirm(`Revoke “${k.name}”? SDKs using it stop sending.`) && revoke.mutate(k.id)}>
              Revoke
            </button>
          )}
        </div>
      ))}
      <NameForm placeholder="Key name, e.g. production api" button="Create key" onSubmit={(n) => create.mutate(n)} />
    </Panel>
  );
}

function NameForm({ placeholder, button, onSubmit }: { placeholder: string; button: string; onSubmit: (name: string) => void }) {
  const [name, setName] = useState('');
  return (
    <form
      className="row"
      style={{ padding: 18 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (name.trim()) onSubmit(name.trim());
        setName('');
      }}
    >
      <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={placeholder} aria-label={placeholder} style={{ maxWidth: 360 }} />
      <button type="submit" className="btn">
        {button}
      </button>
    </form>
  );
}

// ---- redaction ---------------------------------------------------------------------

const RULE_HINTS: Record<RedactionRule['kind'], string> = {
  header: 'Header name, e.g. x-session-token',
  body_key: 'JSON key anywhere in a body, e.g. card_number',
  regex: 'RE2 regex applied to bodies, e.g. \\b\\d{3}-\\d{2}-\\d{4}\\b',
};

function RedactionSection({ toast }: { toast: Toaster }) {
  const project = useProject();
  const qc = useQueryClient();
  const [kind, setKind] = useState<RedactionRule['kind']>('body_key');
  const [pattern, setPattern] = useState('');
  const rules = useQuery({ queryKey: ['rules', project.id], queryFn: () => api.rules(project.id) });
  const refresh = () => void qc.invalidateQueries({ queryKey: ['rules', project.id] });
  const create = useMutation({
    mutationFn: () => api.createRule(project.id, kind, pattern.trim()),
    onSuccess: () => {
      setPattern('');
      refresh();
    },
    onError: (e) => toast(errText(e)),
  });
  const remove = useMutation({ mutationFn: (id: string) => api.deleteRule(project.id, id), onSuccess: refresh, onError: (e) => toast(errText(e)) });
  return (
    <Panel title="Redaction rules" sub="applied at ingest on top of the built-in rules (authorization, cookies, passwords, tokens, card numbers)">
      {rules.isLoading && <Skeleton h={80} />}
      {rules.data?.length === 0 && <Empty title="No custom rules">The built-in rules still apply.</Empty>}
      {rules.data?.map((r) => (
        <div key={r.id} className="kv" style={{ alignItems: 'center' }}>
          <span className="row" style={{ gap: 10 }}>
            <span className="badge steady">{r.kind}</span>
            <span className="mono" style={{ color: 'var(--text)', fontSize: 12.5 }}>
              {r.pattern}
            </span>
          </span>
          <button type="button" className="btn btn-sm" onClick={() => remove.mutate(r.id)}>
            Remove
          </button>
        </div>
      ))}
      <form
        className="row wrap"
        style={{ padding: 18 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (pattern.trim()) create.mutate();
        }}
      >
        <Segmented
          small
          label="Rule kind"
          value={kind}
          onChange={setKind}
          options={[
            { id: 'body_key', label: 'body key' },
            { id: 'header', label: 'header' },
            { id: 'regex', label: 'regex' },
          ]}
        />
        <input className="input mono" value={pattern} onChange={(e) => setPattern(e.target.value)} placeholder={RULE_HINTS[kind]} aria-label="Pattern" style={{ flex: '1 1 260px' }} />
        <button type="submit" className="btn">
          Add rule
        </button>
      </form>
    </Panel>
  );
}

// ---- replay targets ----------------------------------------------------------------

function ReplaySection({ toast }: { toast: Toaster }) {
  const project = useProject();
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const targets = useQuery({ queryKey: ['targets', project.id], queryFn: () => api.replayTargets(project.id) });
  const refresh = () => void qc.invalidateQueries({ queryKey: ['targets', project.id] });
  const create = useMutation({
    mutationFn: () => api.createTarget(project.id, name.trim(), url.trim()),
    onSuccess: () => {
      setName('');
      setUrl('');
      refresh();
    },
    onError: (e) => toast(errText(e)),
  });
  const remove = useMutation({ mutationFn: (id: string) => api.deleteTarget(project.id, id), onSuccess: refresh, onError: (e) => toast(errText(e)) });
  return (
    <Panel title="Replay targets" sub="requests can only be replayed against these base URLs; auth headers are always stripped">
      {targets.data?.length === 0 && <Empty title="No replay targets">Add a staging or local URL to enable “Replay” in the inspector.</Empty>}
      {targets.data?.map((t) => (
        <div key={t.id} className="kv" style={{ alignItems: 'center' }}>
          <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span style={{ color: 'var(--text)', fontSize: 13 }}>{t.name}</span>
            <span className="mono dim" style={{ fontSize: 11.5 }}>
              {t.baseUrl}
            </span>
          </span>
          <button type="button" className="btn btn-sm" onClick={() => remove.mutate(t.id)}>
            Remove
          </button>
        </div>
      ))}
      <form
        className="row wrap"
        style={{ padding: 18 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim() && url.trim()) create.mutate();
        }}
      >
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Name, e.g. staging" aria-label="Target name" style={{ flex: '1 1 160px' }} />
        <input className="input mono" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://staging.example.com" aria-label="Base URL" style={{ flex: '2 1 260px' }} />
        <button type="submit" className="btn">
          Add target
        </button>
      </form>
    </Panel>
  );
}

// ---- members -----------------------------------------------------------------------

function MembersSection({ toast, isOwner }: { toast: Toaster; isOwner: boolean }) {
  const { me } = useSession();
  const qc = useQueryClient();
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<Role>('member');
  const [bodies, setBodies] = useState(false);
  const [link, setLink] = useState<string | null>(null);
  const members = useQuery({ queryKey: ['members'], queryFn: api.members });
  const invites = useQuery({ queryKey: ['invites'], queryFn: api.invites });
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ['members'] });
    void qc.invalidateQueries({ queryKey: ['invites'] });
  };
  const onError = (e: unknown) => toast(errText(e));
  const update = useMutation({
    mutationFn: (a: { id: string; body: Parameters<typeof api.updateMember>[1] }) => api.updateMember(a.id, a.body),
    onSuccess: refresh,
    onError,
  });
  const remove = useMutation({ mutationFn: api.deleteMember, onSuccess: refresh, onError });
  const invite = useMutation({
    mutationFn: () => api.createInvite(email.trim(), role, bodies),
    onSuccess: (r) => {
      setLink(new URL(r.url, window.location.origin).toString());
      setEmail('');
      refresh();
    },
    onError,
  });
  const revokeInvite = useMutation({ mutationFn: api.deleteInvite, onSuccess: refresh, onError });
  const assignable = ROLES.filter((r) => isOwner || r !== 'owner');

  return (
    <>
      <Panel title="Members" sub="roles apply to every project; “bodies” lets a member see request and response bodies and headers">
        {members.isLoading && <Skeleton h={100} />}
        {members.data?.map((m) => {
          const self = m.id === me?.user.id;
          const locked = self || (m.role === 'owner' && !isOwner);
          return (
            <div key={m.id} className="kv" style={{ alignItems: 'center', opacity: m.disabled ? 0.55 : 1 }}>
              <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span style={{ color: 'var(--text)', fontSize: 13 }}>
                  {m.name || m.email} {self && <span className="dim">(you)</span>}
                </span>
                <span className="mono dim ellipsis" style={{ fontSize: 11.5 }}>
                  {m.email} · last sign-in {when(m.lastLoginAt)}
                </span>
              </span>
              <span className="row wrap" style={{ gap: 8, justifyContent: 'flex-end' }}>
                <select
                  className="input"
                  aria-label={`Role for ${m.email}`}
                  value={m.role}
                  disabled={locked}
                  onChange={(e) => update.mutate({ id: m.id, body: { role: e.target.value as Role } })}
                  style={{ width: 'auto', minHeight: 32 }}
                >
                  {(locked ? ROLES : assignable).map((r) => (
                    <option key={r} value={r}>
                      {r}
                    </option>
                  ))}
                </select>
                <label className="row" style={{ fontSize: 12, color: 'var(--text-2)', fontFamily: 'var(--font)' }}>
                  <input type="checkbox" checked={m.canViewBodies} disabled={locked} onChange={(e) => update.mutate({ id: m.id, body: { canViewBodies: e.target.checked } })} />
                  bodies
                </label>
                {!locked && (
                  <>
                    <button type="button" className="btn btn-sm" onClick={() => update.mutate({ id: m.id, body: { disabled: !m.disabled } })}>
                      {m.disabled ? 'Enable' : 'Disable'}
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => window.confirm(`Remove ${m.email}?`) && remove.mutate(m.id)}>
                      Remove
                    </button>
                  </>
                )}
              </span>
            </div>
          );
        })}
      </Panel>

      <Panel title="Invites" sub="invite links expire after 7 days; send the link yourself">
        {link && (
          <div style={{ padding: 18 }}>
            <SecretOnce label="Invite link created." value={link} onDone={() => setLink(null)} />
          </div>
        )}
        {invites.data?.map((i) => (
          <div key={i.id} className="kv" style={{ alignItems: 'center' }}>
            <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              <span style={{ color: 'var(--text)', fontSize: 13 }}>{i.email}</span>
              <span className="mono dim" style={{ fontSize: 11.5 }}>
                {i.role}
                {i.canViewBodies ? ' · bodies' : ''} · expires {fmtDateTime(Date.parse(i.expiresAt))}
              </span>
            </span>
            <button type="button" className="btn btn-sm" onClick={() => revokeInvite.mutate(i.id)}>
              Revoke
            </button>
          </div>
        ))}
        <form
          className="row wrap"
          style={{ padding: 18 }}
          onSubmit={(e) => {
            e.preventDefault();
            if (email.trim()) invite.mutate();
          }}
        >
          <input className="input" type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="teammate@example.com" aria-label="Email" style={{ flex: '2 1 240px' }} />
          <select className="input" aria-label="Role" value={role} onChange={(e) => setRole(e.target.value as Role)} style={{ width: 'auto' }}>
            {assignable.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <label className="row" style={{ fontSize: 12.5, color: 'var(--text-2)' }}>
            <input type="checkbox" checked={bodies} onChange={(e) => setBodies(e.target.checked)} /> can view bodies
          </label>
          <button type="submit" className="btn">
            Create invite link
          </button>
        </form>
      </Panel>
    </>
  );
}

// ---- API tokens --------------------------------------------------------------------

function TokensSection({ toast, role }: { toast: Toaster; role: Role }) {
  const { projects } = useSession();
  const qc = useQueryClient();
  const [name, setName] = useState('');
  const [scope, setScope] = useState<'read' | 'write' | 'admin'>('read');
  const [projectId, setProjectId] = useState('');
  const [days, setDays] = useState('90');
  const [secret, setSecret] = useState<string | null>(null);
  const tokens = useQuery({ queryKey: ['tokens'], queryFn: api.tokens });
  const refresh = () => void qc.invalidateQueries({ queryKey: ['tokens'] });
  const create = useMutation({
    mutationFn: () =>
      api.createToken({
        name: name.trim(),
        scopes: scope === 'admin' ? ['read', 'write', 'admin'] : scope === 'write' ? ['read', 'write'] : ['read'],
        projectId: projectId || null,
        expiresInDays: days ? Number(days) : null,
      }),
    onSuccess: (r) => {
      setSecret(r.token);
      setName('');
      refresh();
    },
    onError: (e) => toast(errText(e)),
  });
  const revoke = useMutation({ mutationFn: api.revokeToken, onSuccess: refresh, onError: (e) => toast(errText(e)) });
  const scopes = [
    { id: 'read' as const, label: 'read' },
    ...(RANK[role] >= RANK.member ? [{ id: 'write' as const, label: 'write' }] : []),
    ...(RANK[role] >= RANK.admin ? [{ id: 'admin' as const, label: 'admin' }] : []),
  ];
  return (
    <Panel title="API tokens" sub="personal tokens for scripts and CI: send as “Authorization: Bearer lbx_api_…”">
      {secret && (
        <div style={{ padding: 18 }}>
          <SecretOnce label="New API token." value={secret} onDone={() => setSecret(null)} />
        </div>
      )}
      {tokens.data?.length === 0 && <Empty title="No tokens" />}
      {tokens.data?.map((t) => (
        <div key={t.id} className="kv" style={{ alignItems: 'center' }}>
          <span style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <span style={{ color: 'var(--text)', fontSize: 13 }}>{t.name}</span>
            <span className="mono dim" style={{ fontSize: 11.5 }}>
              {t.prefix} · {t.scopes.join(', ')}
              {t.projectId ? ` · ${projects.find((p) => p.id === t.projectId)?.name ?? 'one project'}` : ' · all projects'} · last used {when(t.lastUsedAt)}
              {t.expiresAt ? ` · expires ${fmtDateTime(Date.parse(t.expiresAt))}` : ''}
            </span>
          </span>
          <button type="button" className="btn btn-sm" onClick={() => window.confirm(`Revoke “${t.name}”?`) && revoke.mutate(t.id)}>
            Revoke
          </button>
        </div>
      ))}
      <form
        className="row wrap"
        style={{ padding: 18 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) create.mutate();
        }}
      >
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Token name, e.g. ci" aria-label="Token name" style={{ flex: '2 1 200px' }} />
        <Segmented small label="Scope" value={scope} onChange={setScope} options={scopes} />
        <select className="input" aria-label="Project" value={projectId} onChange={(e) => setProjectId(e.target.value)} style={{ width: 'auto' }}>
          <option value="">all projects</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <select className="input" aria-label="Expiry" value={days} onChange={(e) => setDays(e.target.value)} style={{ width: 'auto' }}>
          <option value="30">30 days</option>
          <option value="90">90 days</option>
          <option value="365">1 year</option>
          <option value="">never expires</option>
        </select>
        <button type="submit" className="btn">
          Create token
        </button>
      </form>
    </Panel>
  );
}

// ---- retention and account ---------------------------------------------------------

function RetentionSection() {
  const r = useQuery({ queryKey: ['retention'], queryFn: api.retention });
  return (
    <Panel title="Retention" sub="set in logbox.toml or with LOGBOX_RETENTION_* environment variables; applied at startup">
      {r.data ? (
        <>
          <div className="kv">
            <span>Requests, events and spans</span>
            <span>{r.data.days} days</span>
          </div>
          <div className="kv">
            <span>Bodies and headers</span>
            <span>{r.data.bodyDays} days</span>
          </div>
        </>
      ) : (
        <Skeleton h={60} />
      )}
    </Panel>
  );
}

function AccountSection({ toast }: { toast: Toaster }) {
  const { me, theme, toggleTheme, signOut } = useSession();
  const [current, setCurrent] = useState('');
  const [next, setNext] = useState('');
  const change = useMutation({
    mutationFn: () => api.changePassword(current, next),
    onSuccess: () => {
      setCurrent('');
      setNext('');
      toast('Password changed');
    },
    onError: (e) => toast(errText(e)),
  });
  return (
    <>
      <Panel title="Account">
        <div className="kv">
          <span>Name</span>
          <span>{me?.user.name}</span>
        </div>
        <div className="kv">
          <span>Email</span>
          <span>{me?.user.email}</span>
        </div>
        <div className="kv">
          <span>Role</span>
          <span>
            {me?.user.role}
            {me?.user.can_view_bodies ? ' · can view bodies' : ''}
          </span>
        </div>
        <div className="kv" style={{ alignItems: 'center' }}>
          <span>Theme</span>
          <Segmented small label="Theme" value={theme} onChange={(t) => t !== theme && toggleTheme()} options={[{ id: 'dark', label: 'dark' }, { id: 'light', label: 'light' }]} />
        </div>
        <div className="row" style={{ padding: 18 }}>
          <button type="button" className="btn" onClick={() => void signOut()}>
            Sign out
          </button>
        </div>
      </Panel>
      <Panel title="Change password" sub="at least 10 characters; other sessions are signed out">
        <form
          style={formStyle}
          onSubmit={(e) => {
            e.preventDefault();
            change.mutate();
          }}
        >
          <Field label="Current password">
            <input className="input" type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} required />
          </Field>
          <Field label="New password">
            <input className="input" type="password" autoComplete="new-password" minLength={10} value={next} onChange={(e) => setNext(e.target.value)} required />
          </Field>
          <div className="row" style={{ alignSelf: 'end' }}>
            <button type="submit" className="btn btn-primary" disabled={change.isPending}>
              Change password
            </button>
          </div>
        </form>
      </Panel>
    </>
  );
}
