// The admin console (decision 071): every organization on this server, their
// plans, usage and invoices. Only instance admins reach it; it has its own
// sidebar because it works across organizations, not inside one.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, Outlet, useNavigate, useRouterState } from '@tanstack/react-router';
import { useMemo, useState, type FormEvent, type ReactNode } from 'react';

import { IconBookmark, IconGrid, IconSettings, Logo } from '../components/icons';
import {
  Empty,
  SecretOnce,
  Segmented,
  Skeleton,
  Sparkline,
  Toast,
  useToast,
} from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtAgo, fmtCompact, fmtInt } from '../lib/format';
import { useSession } from '../lib/session';
import type {
  AdminOrg,
  AdminOrgDetail,
  InviteCreated,
  Invoice,
  PlanInput,
  PlanWithCount,
  Role,
} from '../lib/types';

type Toaster = (msg: string) => void;

const errText = (e: unknown) =>
  e instanceof ApiError || e instanceof Error ? e.message : 'Something went wrong';
const iqd = (n: number) => `${fmtInt(n)} IQD`;
const ago = (iso: string | null) => (iso ? fmtAgo(Date.parse(iso)) : 'never');
const limitText = (n: number | null) => (n === null ? 'unlimited' : fmtCompact(n));
/** A future time, e.g. "in 6d". */
const until = (iso: string) => {
  const ms = Date.parse(iso) - Date.now();
  if (ms <= 0) return 'expired';
  const days = Math.floor(ms / 86_400_000);
  return days >= 1 ? `in ${days}d` : `in ${Math.max(1, Math.round(ms / 3_600_000))}h`;
};

/** First and last day of the current month, as `YYYY-MM-DD`. */
function thisMonth(): [string, string] {
  const now = new Date();
  const y = now.getUTCFullYear();
  const m = now.getUTCMonth();
  const pad = (n: number) => String(n).padStart(2, '0');
  const last = new Date(Date.UTC(y, m + 1, 0)).getUTCDate();
  return [`${y}-${pad(m + 1)}-01`, `${y}-${pad(m + 1)}-${pad(last)}`];
}

// ---- layout ----------------------------------------------------------------------

const NAV = [
  { to: '/admin', label: 'Organizations', icon: IconGrid },
  { to: '/admin/plans', label: 'Plans', icon: IconSettings },
  { to: '/admin/invoices', label: 'Invoices', icon: IconBookmark },
] as const;

export function AdminShell() {
  const { me } = useSession();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const active = (to: string) =>
    to === '/admin'
      ? pathname === '/admin' || pathname.startsWith('/admin/orgs')
      : pathname.startsWith(to);
  return (
    <div className="app">
      <aside className="sidebar">
        <Link to="/admin" className="logo">
          <Logo />
          <span className="logo-word">trailscope</span>
          <span className="badge new" style={{ marginLeft: 4 }}>
            admin
          </span>
        </Link>
        <nav aria-label="Admin console" className="nav">
          {NAV.map((n) => (
            <Link key={n.to} to={n.to} className={active(n.to) ? 'active' : undefined}>
              <n.icon />
              {n.label}
            </Link>
          ))}
        </nav>
        {!me?.cloud && (
          <p className="dim" style={{ fontSize: 12, margin: '0 10px', lineHeight: 1.5 }}>
            Plan limits aren&apos;t enforced on this server. Set{' '}
            <code className="mono">TRAILSCOPE_CLOUD_ENABLED=true</code> to stop Free organizations
            at their limit and apply project and member caps. Usage is counted either way.
          </p>
        )}
        <div style={{ marginTop: 'auto', display: 'flex', flexDirection: 'column', gap: 6 }}>
          <a href="/live" className="btn" style={{ justifyContent: 'center' }}>
            ← Back to Trailscope
          </a>
          <span className="dim ellipsis" style={{ fontSize: 11.5, padding: '0 4px' }}>
            {me?.user.email}
          </span>
        </div>
      </aside>
      <Outlet />
    </div>
  );
}

function Panel({
  title,
  sub,
  actions,
  children,
}: {
  title: string;
  sub?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-label={title} className="panel" style={{ overflow: 'hidden' }}>
      <div
        className="row wrap"
        style={{ gap: 12, padding: '14px 18px', borderBottom: '1px solid var(--line)' }}
      >
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
      {hint && (
        <span className="dim" style={{ fontSize: 11.5 }}>
          {hint}
        </span>
      )}
    </label>
  );
}

const formStyle = {
  padding: 18,
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
  gap: 16,
  alignItems: 'end',
} as const;

function Tile({ label, value, note }: { label: string; value: ReactNode; note?: ReactNode }) {
  return (
    <div className="panel tile">
      <span className="muted" style={{ fontSize: 12 }}>
        {label}
      </span>
      <span className="tile-value" style={{ fontSize: 26 }}>
        {value}
      </span>
      {note && (
        <span className="dim" style={{ fontSize: 12 }}>
          {note}
        </span>
      )}
    </div>
  );
}

/** Events against the plan's limit, as a bar. */
function UsageBar({ used, limit, hard }: { used: number; limit: number | null; hard: boolean }) {
  const ratio = limit ? used / limit : 0;
  const color =
    ratio >= 1
      ? hard
        ? 'var(--err)'
        : 'var(--warn)'
      : ratio >= 0.8
        ? 'var(--warn)'
        : 'var(--accent)';
  return (
    <span style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
      <span className="mono" style={{ fontSize: 12 }}>
        {fmtCompact(used)} <span className="dim">/ {limitText(limit)}</span>
      </span>
      {limit !== null && (
        <span className="bar" aria-hidden="true">
          <span style={{ width: `${Math.min(100, ratio * 100)}%`, background: color }} />
        </span>
      )}
    </span>
  );
}

function StatusBadges({ org }: { org: AdminOrg }) {
  const over = org.eventsLimit !== null && org.eventsThisMonth >= org.eventsLimit;
  return (
    <span className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
      {org.suspended ? (
        <span className="pill s5">suspended</span>
      ) : over ? (
        <span className={`pill ${org.hardLimit ? 's5' : 's4'}`}>
          {org.hardLimit ? 'at limit' : 'overage'}
        </span>
      ) : (
        <span className="pill s2">active</span>
      )}
      {org.openInvoices > 0 && (
        <span className="pill s3">
          {org.openInvoices} open invoice{org.openInvoices === 1 ? '' : 's'}
        </span>
      )}
    </span>
  );
}

function usePlans() {
  return useQuery({ queryKey: ['admin', 'plans'], queryFn: api.admin.plans });
}

function PlanSelect({
  value,
  onChange,
  plans,
  label = 'Plan',
  current,
}: {
  value: string;
  onChange: (v: string) => void;
  plans: PlanWithCount[];
  label?: string;
  /** Shown even when archived. */
  current?: string;
}) {
  return (
    <select
      className="input"
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {plans
        .filter((p) => !p.archived || p.id === current || p.id === value)
        .map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
            {p.priceIqd ? ` · ${iqd(p.priceIqd)}/mo` : ''}
          </option>
        ))}
    </select>
  );
}

// ---- organizations ---------------------------------------------------------------

export function AdminOrgs() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [toast, setToast] = useToast();
  const orgs = useQuery({ queryKey: ['admin', 'orgs'], queryFn: api.admin.orgs });
  const plans = usePlans();
  const [filter, setFilter] = useState('');
  const [created, setCreated] = useState<{ org: AdminOrg; invite: InviteCreated | null } | null>(
    null,
  );

  const list = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const all = orgs.data ?? [];
    if (!q) return all;
    return all.filter(
      (o) =>
        o.name.toLowerCase().includes(q) ||
        o.slug.includes(q) ||
        o.planName.toLowerCase().includes(q) ||
        o.owners.some((e) => e.toLowerCase().includes(q)),
    );
  }, [orgs.data, filter]);

  const totals = useMemo(() => {
    const all = orgs.data ?? [];
    return {
      count: all.length,
      events: all.reduce((s, o) => s + o.eventsThisMonth, 0),
      paying: all.filter((o) => o.planId !== 'free').length,
      suspended: all.filter((o) => o.suspended).length,
      open: all.reduce((s, o) => s + o.openInvoices, 0),
    };
  }, [orgs.data]);

  const cols =
    'minmax(180px, 2fr) minmax(90px, 1fr) minmax(140px, 1.4fr) 70px 70px minmax(150px, 1.4fr)';

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Organizations</h1>
        <span className="muted">every company on this server</span>
      </header>

      <div className="tiles">
        <Tile
          label="Organizations"
          value={fmtInt(totals.count)}
          note={`${totals.paying} on a paid plan`}
        />
        <Tile
          label="Events this month"
          value={fmtCompact(totals.events)}
          note="all organizations"
        />
        <Tile label="Open invoices" value={fmtInt(totals.open)} />
        <Tile label="Suspended" value={fmtInt(totals.suspended)} />
      </div>

      <NewOrg
        plans={plans.data ?? []}
        toast={setToast}
        onCreated={(r) => {
          setCreated(r);
          void qc.invalidateQueries({ queryKey: ['admin'] });
        }}
      />
      {created && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          {created.invite ? (
            <SecretOnce
              label={`${created.org.name} is created. Send this owner invite link to them (valid 7 days).`}
              value={created.invite.url}
              onDone={() => setCreated(null)}
            />
          ) : (
            <div className="panel panel-pad" role="status">
              {created.org.name} is created. Invite its owner from its page.
            </div>
          )}
          <span>
            <Link to="/admin/orgs/$orgId" params={{ orgId: created.org.id }} className="btn btn-sm">
              Open {created.org.name}
            </Link>
          </span>
        </div>
      )}

      <Panel
        title="All organizations"
        sub={orgs.data ? `${list.length} of ${orgs.data.length}` : undefined}
        actions={
          <input
            className="input"
            placeholder="Filter by name, plan or owner"
            aria-label="Filter organizations"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: 240 }}
          />
        }
      >
        <div className="scroll-x">
          <div style={{ minWidth: 760 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>Organization</span>
              <span>Plan</span>
              <span>Events this month</span>
              <span>Projects</span>
              <span>Members</span>
              <span>Status</span>
            </div>
            {orgs.isLoading && (
              <div style={{ padding: 18 }}>
                <Skeleton h={80} />
              </div>
            )}
            {orgs.data?.length === 0 && (
              <Empty title="No organizations yet">Create the first one above.</Empty>
            )}
            {list.map((o) => (
              <div
                key={o.id}
                role="link"
                tabIndex={0}
                className="table-row"
                style={{ gridTemplateColumns: cols, padding: '10px 18px' }}
                onClick={() => void navigate({ to: '/admin/orgs/$orgId', params: { orgId: o.id } })}
                onKeyDown={(e) => {
                  if (e.key === 'Enter')
                    void navigate({ to: '/admin/orgs/$orgId', params: { orgId: o.id } });
                }}
              >
                <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                  <span className="ellipsis" style={{ fontWeight: 500 }}>
                    {o.name}
                  </span>
                  <span className="dim ellipsis" style={{ fontSize: 11.5 }}>
                    {o.owners.length ? o.owners.join(', ') : 'no owner yet'}
                  </span>
                </span>
                <span>{o.planName}</span>
                <UsageBar used={o.eventsThisMonth} limit={o.eventsLimit} hard={o.hardLimit} />
                <span className="mono">{o.projects}</span>
                <span className="mono">{o.members}</span>
                <StatusBadges org={o} />
              </div>
            ))}
          </div>
        </div>
      </Panel>
      <Toast msg={toast} />
    </main>
  );
}

function NewOrg({
  plans,
  toast,
  onCreated,
}: {
  plans: PlanWithCount[];
  toast: Toaster;
  onCreated: (r: { org: AdminOrg; invite: InviteCreated | null }) => void;
}) {
  const [name, setName] = useState('');
  const [plan, setPlan] = useState('free');
  const [owner, setOwner] = useState('');
  const create = useMutation({
    mutationFn: () =>
      api.admin.createOrg({
        name: name.trim(),
        planId: plan,
        ownerEmail: owner.trim() || undefined,
      }),
    onSuccess: (r) => {
      setName('');
      setOwner('');
      onCreated(r);
    },
    onError: (e) => toast(errText(e)),
  });
  return (
    <Panel
      title="New organization"
      sub="For a customer: you don't become a member; their owner gets an invite link"
    >
      <form
        style={formStyle}
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim()) create.mutate();
        }}
      >
        <Field label="Company name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Acme"
            required
            maxLength={100}
          />
        </Field>
        <Field label="Plan">
          <PlanSelect value={plan} onChange={setPlan} plans={plans} />
        </Field>
        <Field label="Owner's email" hint="Optional: creates their owner invite link">
          <input
            className="input"
            type="email"
            value={owner}
            onChange={(e) => setOwner(e.target.value)}
            placeholder="cto@acme.com"
          />
        </Field>
        <div>
          <button type="submit" className="btn btn-primary" disabled={create.isPending}>
            Create organization
          </button>
        </div>
      </form>
    </Panel>
  );
}

// ---- one organization ------------------------------------------------------------

export function AdminOrgPage({ orgId }: { orgId: string }) {
  const qc = useQueryClient();
  const [toast, setToast] = useToast();
  const detail = useQuery({
    queryKey: ['admin', 'org', orgId],
    queryFn: () => api.admin.org(orgId),
  });
  const plans = usePlans();
  const refresh = () => void qc.invalidateQueries({ queryKey: ['admin'] });
  const update = useMutation({
    mutationFn: (body: { name?: string; planId?: string; suspended?: boolean; note?: string }) =>
      api.admin.updateOrg(orgId, body),
    onSuccess: () => {
      setToast('Saved');
      refresh();
    },
    onError: (e) => setToast(errText(e)),
  });
  const join = useMutation({
    mutationFn: () => api.admin.join(orgId),
    onSuccess: () => window.location.assign('/live'),
    onError: (e) => setToast(errText(e)),
  });

  if (detail.isLoading) {
    return (
      <main className="main">
        <Skeleton h={28} w={240} />
        <Skeleton h={200} />
      </main>
    );
  }
  if (!detail.data) {
    return (
      <main className="main">
        <Empty title="Organization not found">
          <Link to="/admin">Back to organizations</Link>
        </Empty>
      </main>
    );
  }
  const d = detail.data;
  const o = d.org;
  const plan = plans.data?.find((p) => p.id === o.planId);
  const series = lastDays(d.usage, 31);

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <Link to="/admin" className="muted">
          Organizations
        </Link>
        <span className="dim">/</span>
        <h1 className="page-title">{o.name}</h1>
        <StatusBadges org={o} />
        <span className="row" style={{ marginLeft: 'auto', gap: 8 }}>
          <button
            type="button"
            className="btn"
            onClick={() => update.mutate({ suspended: !o.suspended })}
            disabled={update.isPending}
            title={
              o.suspended
                ? 'Accept its ingest keys again'
                : 'Refuse its ingest keys; members can still sign in'
            }
          >
            {o.suspended ? 'Resume' : 'Suspend'}
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => join.mutate()}
            disabled={join.isPending}
            title="Become an owner of this organization and open it"
          >
            Open as owner
          </button>
        </span>
      </header>

      <div className="tiles">
        <div className="panel tile">
          <span className="muted" style={{ fontSize: 12 }}>
            Events this month
          </span>
          <UsageBar used={o.eventsThisMonth} limit={o.eventsLimit} hard={o.hardLimit} />
          <Sparkline
            values={series}
            width={200}
            color="var(--accent)"
            label="Events per day, last 31 days"
          />
        </div>
        <Tile
          label="Projects"
          value={fmtInt(o.projects)}
          note={`plan allows ${plan?.maxProjects ?? 'unlimited'}`}
        />
        <Tile
          label="Members"
          value={fmtInt(o.members)}
          note={`plan allows ${plan?.maxMembers ?? 'unlimited'}`}
        />
        <Tile
          label="Plan"
          value={<span style={{ fontSize: 22 }}>{o.planName}</span>}
          note={plan ? `${iqd(plan.priceIqd)} a month` : undefined}
        />
      </div>

      <OrgSettings
        detail={d}
        plans={plans.data ?? []}
        onSave={(b) => update.mutate(b)}
        busy={update.isPending}
      />
      <OrgProjects detail={d} />
      <OrgMembers detail={d} toast={setToast} onChange={refresh} />
      <InvoicesPanel
        org={o}
        invoices={d.invoices}
        plans={plans.data ?? []}
        toast={setToast}
        onChange={refresh}
      />
      <Toast msg={toast} />
    </main>
  );
}

/** Daily events for the last `n` days (UTC), zero-filled. */
function lastDays(usage: AdminOrgDetail['usage'], n: number): number[] {
  const by = new Map(usage.map((u) => [u.day, u.events]));
  const out: number[] = [];
  const today = Date.UTC(
    new Date().getUTCFullYear(),
    new Date().getUTCMonth(),
    new Date().getUTCDate(),
  );
  for (let i = n - 1; i >= 0; i--) {
    const day = new Date(today - i * 86_400_000).toISOString().slice(0, 10);
    out.push(by.get(day) ?? 0);
  }
  return out;
}

function OrgSettings({
  detail,
  plans,
  onSave,
  busy,
}: {
  detail: AdminOrgDetail;
  plans: PlanWithCount[];
  onSave: (b: { name?: string; planId?: string; note?: string }) => void;
  busy: boolean;
}) {
  const o = detail.org;
  const [name, setName] = useState(o.name);
  const [plan, setPlan] = useState(o.planId);
  const [note, setNote] = useState(o.note);
  const dirty = name.trim() !== o.name || plan !== o.planId || note !== o.note;
  return (
    <Panel title="Plan and details" sub={`slug ${o.slug} · created ${ago(o.createdAt)}`}>
      <form
        style={formStyle}
        onSubmit={(e) => {
          e.preventDefault();
          onSave({
            name: name.trim() !== o.name ? name.trim() : undefined,
            planId: plan !== o.planId ? plan : undefined,
            note: note !== o.note ? note : undefined,
          });
        }}
      >
        <Field label="Name">
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
            maxLength={100}
          />
        </Field>
        <Field label="Plan" hint="Takes effect within seconds, at ingest too">
          <PlanSelect value={plan} onChange={setPlan} plans={plans} current={o.planId} />
        </Field>
        <Field label="Private note" hint="Only instance admins see it">
          <input
            className="input"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="e.g. pays yearly, contact on WhatsApp"
            maxLength={4000}
          />
        </Field>
        <div>
          <button type="submit" className="btn btn-primary" disabled={!dirty || busy}>
            Save
          </button>
        </div>
      </form>
    </Panel>
  );
}

function OrgProjects({ detail }: { detail: AdminOrgDetail }) {
  const cols = 'minmax(160px, 2fr) minmax(120px, 1fr) 140px 120px';
  return (
    <Panel title="Projects" sub={`${detail.projects.length}`}>
      {detail.projects.length === 0 ? (
        <div className="panel-pad dim">
          No projects yet; the owner creates the first one after signing in.
        </div>
      ) : (
        <div className="scroll-x">
          <div style={{ minWidth: 560 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>Project</span>
              <span>Slug</span>
              <span>Stored this month</span>
              <span>Created</span>
            </div>
            {detail.projects.map((p) => (
              <div
                key={p.id}
                className="table-row"
                style={{ gridTemplateColumns: cols, cursor: 'default' }}
              >
                <span className="ellipsis">{p.name}</span>
                <span className="cell-mono dim ellipsis">{p.slug}</span>
                <span className="cell-mono">
                  {p.eventsThisMonth === null ? '—' : fmtCompact(p.eventsThisMonth)}
                </span>
                <span className="dim">{ago(p.createdAt)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </Panel>
  );
}

const INVITE_ROLES: Role[] = ['owner', 'admin', 'member', 'viewer'];

function OrgMembers({
  detail,
  toast,
  onChange,
}: {
  detail: AdminOrgDetail;
  toast: Toaster;
  onChange: () => void;
}) {
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<Role>(detail.members.length ? 'member' : 'owner');
  const [link, setLink] = useState<string | null>(null);
  const invite = useMutation({
    mutationFn: () =>
      api.admin.invite(detail.org.id, {
        email: email.trim(),
        role,
        canViewBodies: role === 'owner',
      }),
    onSuccess: (r) => {
      setLink(r.url);
      setEmail('');
      onChange();
    },
    onError: (e) => toast(errText(e)),
  });
  const cols = 'minmax(200px, 2fr) 90px 120px';
  return (
    <Panel title="Members" sub={`${detail.members.length} · ${detail.invites.length} open invites`}>
      {detail.members.length === 0 && detail.invites.length === 0 && (
        <div className="panel-pad dim">Nobody yet. Invite the owner below.</div>
      )}
      {detail.members.length > 0 && (
        <>
          <div className="table-head" style={{ gridTemplateColumns: cols }}>
            <span>Member</span>
            <span>Role</span>
            <span>Last sign-in</span>
          </div>
          {detail.members.map((m) => (
            <div
              key={m.id}
              className="table-row"
              style={{ gridTemplateColumns: cols, cursor: 'default', padding: '8px 18px' }}
            >
              <span style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
                <span className="ellipsis">{m.name || m.email}</span>
                {m.name && (
                  <span className="dim ellipsis" style={{ fontSize: 11.5 }}>
                    {m.email}
                  </span>
                )}
              </span>
              <span>
                {m.role}
                {m.disabled && <span className="dim"> · disabled</span>}
              </span>
              <span className="dim">{ago(m.lastLoginAt)}</span>
            </div>
          ))}
        </>
      )}
      {detail.invites.map((i) => (
        <div key={i.email} className="kv">
          <span>
            {i.email} <span className="dim">· invited as {i.role}</span>
          </span>
          <span className="dim">expires {until(i.expiresAt)}</span>
        </div>
      ))}
      {link && (
        <div style={{ padding: 18 }}>
          <SecretOnce
            label="Invite link created. Send it yourself (valid 7 days)."
            value={link}
            onDone={() => setLink(null)}
          />
        </div>
      )}
      <form
        className="row wrap"
        style={{ padding: 18, gap: 10, borderTop: '1px solid var(--line-soft)' }}
        onSubmit={(e) => {
          e.preventDefault();
          if (email.trim()) invite.mutate();
        }}
      >
        <input
          className="input"
          type="email"
          placeholder="Email to invite"
          aria-label="Email to invite"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          style={{ maxWidth: 300 }}
          required
        />
        <select
          className="input"
          aria-label="Role"
          value={role}
          onChange={(e) => setRole(e.target.value as Role)}
          style={{ width: 120 }}
        >
          {INVITE_ROLES.map((r) => (
            <option key={r} value={r}>
              {r}
            </option>
          ))}
        </select>
        <button type="submit" className="btn" disabled={invite.isPending}>
          Create invite link
        </button>
      </form>
    </Panel>
  );
}

// ---- invoices --------------------------------------------------------------------

function InvoiceRows({
  invoices,
  showOrg,
  toast,
  onChange,
}: {
  invoices: Invoice[];
  showOrg: boolean;
  toast: Toaster;
  onChange: () => void;
}) {
  const update = useMutation({
    mutationFn: ({
      id,
      body,
    }: {
      id: string;
      body: { status: Invoice['status']; applyPlan?: boolean };
    }) => api.admin.updateInvoice(id, body),
    onSuccess: () => {
      toast('Invoice updated');
      onChange();
    },
    onError: (e) => toast(errText(e)),
  });
  const cols = showOrg
    ? 'minmax(140px, 1.4fr) minmax(90px, 1fr) minmax(170px, 1.3fr) minmax(110px, 1fr) 80px minmax(190px, 1.5fr)'
    : 'minmax(90px, 1fr) minmax(170px, 1.3fr) minmax(110px, 1fr) 80px minmax(190px, 1.5fr)';
  if (invoices.length === 0) return <div className="panel-pad dim">No invoices.</div>;
  return (
    <div className="scroll-x">
      <div style={{ minWidth: showOrg ? 820 : 680 }}>
        <div className="table-head" style={{ gridTemplateColumns: cols }}>
          {showOrg && <span>Organization</span>}
          <span>Plan</span>
          <span>Period</span>
          <span>Amount</span>
          <span>Status</span>
          <span />
        </div>
        {invoices.map((i) => (
          <div
            key={i.id}
            className="table-row"
            style={{ gridTemplateColumns: cols, cursor: 'default', padding: '8px 18px' }}
          >
            {showOrg && (
              <Link to="/admin/orgs/$orgId" params={{ orgId: i.orgId }} className="ellipsis">
                {i.orgName}
              </Link>
            )}
            <span className="ellipsis" title={i.note || undefined}>
              {i.planName}
              {i.note && <span className="dim"> · {i.note}</span>}
            </span>
            <span className="cell-mono">
              {i.periodStart} → {i.periodEnd}
            </span>
            <span className="cell-mono">{iqd(i.amountIqd)}</span>
            <span>
              <span
                className={`pill ${i.status === 'paid' ? 's2' : i.status === 'open' ? 's3' : 's4'}`}
              >
                {i.status}
              </span>
            </span>
            <span className="row" style={{ gap: 6, justifyContent: 'flex-end' }}>
              {i.status === 'open' && (
                <>
                  <button
                    type="button"
                    className="btn btn-sm btn-primary"
                    title={`Also move the organization to ${i.planName}`}
                    onClick={() =>
                      update.mutate({ id: i.id, body: { status: 'paid', applyPlan: true } })
                    }
                  >
                    Mark paid
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm"
                    onClick={() => update.mutate({ id: i.id, body: { status: 'void' } })}
                  >
                    Void
                  </button>
                </>
              )}
              {i.status === 'paid' && (
                <span className="dim" style={{ fontSize: 12 }}>
                  paid {ago(i.paidAt)}
                </span>
              )}
              {i.status === 'void' && (
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => update.mutate({ id: i.id, body: { status: 'open' } })}
                >
                  Reopen
                </button>
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function NewInvoice({
  orgs,
  fixedOrg,
  plans,
  toast,
  onChange,
}: {
  orgs: AdminOrg[];
  fixedOrg?: AdminOrg;
  plans: PlanWithCount[];
  toast: Toaster;
  onChange: () => void;
}) {
  const [start0, end0] = thisMonth();
  const [orgId, setOrgId] = useState(fixedOrg?.id ?? '');
  const org = fixedOrg ?? orgs.find((o) => o.id === orgId);
  const [planId, setPlanId] = useState(
    fixedOrg && fixedOrg.planId !== 'free' ? fixedOrg.planId : 'team',
  );
  const [start, setStart] = useState(start0);
  const [end, setEnd] = useState(end0);
  const [amount, setAmount] = useState('');
  const [note, setNote] = useState('');
  const planPrice = plans.find((p) => p.id === planId)?.priceIqd ?? 0;
  const create = useMutation({
    mutationFn: () =>
      api.admin.createInvoice({
        orgId: org?.id ?? '',
        planId,
        periodStart: start,
        periodEnd: end,
        amountIqd: amount.trim() === '' ? planPrice : Number(amount),
        note: note.trim() || undefined,
      }),
    onSuccess: () => {
      toast('Invoice recorded');
      setAmount('');
      setNote('');
      onChange();
    },
    onError: (e) => toast(errText(e)),
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!org) {
      toast('Pick an organization');
      return;
    }
    create.mutate();
  };
  return (
    <form style={{ ...formStyle, borderTop: '1px solid var(--line-soft)' }} onSubmit={submit}>
      {!fixedOrg && (
        <Field label="Organization">
          <select
            className="input"
            value={orgId}
            onChange={(e) => setOrgId(e.target.value)}
            required
          >
            <option value="">Choose…</option>
            {orgs.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name}
              </option>
            ))}
          </select>
        </Field>
      )}
      <Field label="Plan">
        <PlanSelect value={planId} onChange={setPlanId} plans={plans} />
      </Field>
      <Field label="From">
        <input
          className="input"
          type="date"
          value={start}
          onChange={(e) => setStart(e.target.value)}
          required
        />
      </Field>
      <Field label="To (inclusive)">
        <input
          className="input"
          type="date"
          value={end}
          onChange={(e) => setEnd(e.target.value)}
          required
        />
      </Field>
      <Field label="Amount (IQD)" hint={`Empty = plan price, ${iqd(planPrice)}`}>
        <input
          className="input mono"
          inputMode="numeric"
          pattern="[0-9]*"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          placeholder={String(planPrice)}
        />
      </Field>
      <Field label="Note">
        <input
          className="input"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="e.g. FIB ref 1234"
          maxLength={1000}
        />
      </Field>
      <div>
        <button type="submit" className="btn" disabled={create.isPending}>
          Record invoice
        </button>
      </div>
    </form>
  );
}

function InvoicesPanel({
  org,
  invoices,
  plans,
  toast,
  onChange,
}: {
  org: AdminOrg;
  invoices: Invoice[];
  plans: PlanWithCount[];
  toast: Toaster;
  onChange: () => void;
}) {
  return (
    <Panel
      title="Invoices"
      sub="Recorded by hand until FIB payments are connected; “Mark paid” also moves them to the invoice's plan"
    >
      <InvoiceRows invoices={invoices} showOrg={false} toast={toast} onChange={onChange} />
      <NewInvoice orgs={[]} fixedOrg={org} plans={plans} toast={toast} onChange={onChange} />
    </Panel>
  );
}

type InvoiceFilter = 'all' | Invoice['status'];

export function AdminInvoices() {
  const qc = useQueryClient();
  const [toast, setToast] = useToast();
  const [status, setStatus] = useState<InvoiceFilter>('open');
  const invoices = useQuery({
    queryKey: ['admin', 'invoices', status],
    queryFn: () => api.admin.invoices(status === 'all' ? {} : { status }),
  });
  const orgs = useQuery({ queryKey: ['admin', 'orgs'], queryFn: api.admin.orgs });
  const plans = usePlans();
  const refresh = () => void qc.invalidateQueries({ queryKey: ['admin'] });
  const open = (invoices.data ?? []).filter((i) => i.status === 'open');
  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Invoices</h1>
        <span className="muted">what each organization owes, in IQD</span>
      </header>
      {status === 'open' && invoices.data && (
        <div className="tiles">
          <Tile label="Open invoices" value={fmtInt(open.length)} />
          <Tile label="Outstanding" value={iqd(open.reduce((s, i) => s + i.amountIqd, 0))} />
        </div>
      )}
      <Panel
        title="Invoices"
        actions={
          <Segmented<InvoiceFilter>
            label="Status"
            small
            value={status}
            onChange={setStatus}
            options={[
              { id: 'open', label: 'Open' },
              { id: 'paid', label: 'Paid' },
              { id: 'void', label: 'Void' },
              { id: 'all', label: 'All' },
            ]}
          />
        }
      >
        {invoices.isLoading ? (
          <div style={{ padding: 18 }}>
            <Skeleton h={60} />
          </div>
        ) : (
          <InvoiceRows invoices={invoices.data ?? []} showOrg toast={setToast} onChange={refresh} />
        )}
      </Panel>
      <Panel
        title="Record an invoice"
        sub="The customer pays by FIB; mark it paid when the money arrives"
      >
        <NewInvoice
          orgs={orgs.data ?? []}
          plans={plans.data ?? []}
          toast={setToast}
          onChange={refresh}
        />
      </Panel>
      <Toast msg={toast} />
    </main>
  );
}

// ---- plans -----------------------------------------------------------------------

const EMPTY_PLAN: PlanInput = {
  name: '',
  priceIqd: 0,
  eventsPerMonth: 1_000_000,
  hardLimit: false,
  maxProjects: null,
  maxMembers: null,
  archived: false,
};

export function AdminPlans() {
  const qc = useQueryClient();
  const [toast, setToast] = useToast();
  const plans = usePlans();
  const [editing, setEditing] = useState<string | null>(null);
  const refresh = () => void qc.invalidateQueries({ queryKey: ['admin'] });
  const cols = 'minmax(150px, 1.5fr) minmax(110px, 1fr) minmax(150px, 1.3fr) 80px 80px 60px 70px';
  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Plans</h1>
        <span className="muted">what each organization may send, and what it costs</span>
      </header>
      <Panel title="Plans" sub="Changes apply to every organization on the plan within seconds">
        <div className="scroll-x">
          <div style={{ minWidth: 760 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>Plan</span>
              <span>Price / month</span>
              <span>Events / month</span>
              <span>Projects</span>
              <span>Members</span>
              <span>Orgs</span>
              <span />
            </div>
            {plans.isLoading && (
              <div style={{ padding: 18 }}>
                <Skeleton h={60} />
              </div>
            )}
            {plans.data?.map((p) => (
              <div key={p.id}>
                <div
                  className="table-row"
                  style={{ gridTemplateColumns: cols, cursor: 'default', padding: '8px 18px' }}
                >
                  <span style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
                    <span className="ellipsis" style={{ fontWeight: 500 }}>
                      {p.name}
                      {p.archived && <span className="dim"> · archived</span>}
                    </span>
                    <span className="cell-mono dim">{p.id}</span>
                  </span>
                  <span className="cell-mono">{iqd(p.priceIqd)}</span>
                  <span>
                    <span className="mono">{limitText(p.eventsPerMonth)}</span>
                    {p.eventsPerMonth !== null && (
                      <span className="dim" style={{ fontSize: 12 }}>
                        {' '}
                        · {p.hardLimit ? 'stops' : 'overage'}
                      </span>
                    )}
                  </span>
                  <span className="mono">{p.maxProjects ?? '∞'}</span>
                  <span className="mono">{p.maxMembers ?? '∞'}</span>
                  <span className="mono">{p.orgs}</span>
                  <span style={{ textAlign: 'right' }}>
                    <button
                      type="button"
                      className="btn btn-sm"
                      onClick={() => setEditing(editing === p.id ? null : p.id)}
                    >
                      {editing === p.id ? 'Close' : 'Edit'}
                    </button>
                  </span>
                </div>
                {editing === p.id && (
                  <PlanForm
                    initial={p}
                    submitLabel="Save plan"
                    onSubmit={async (body) => {
                      await api.admin.updatePlan(p.id, body);
                      setToast('Plan saved');
                      setEditing(null);
                      refresh();
                    }}
                    toast={setToast}
                  />
                )}
              </div>
            ))}
          </div>
        </div>
      </Panel>
      <Panel title="New plan" sub="e.g. a yearly plan, or a custom deal for one customer">
        <PlanForm
          initial={EMPTY_PLAN}
          withId
          submitLabel="Create plan"
          onSubmit={async (body, id) => {
            await api.admin.createPlan({ ...body, id: id ?? '' });
            setToast('Plan created');
            refresh();
          }}
          toast={setToast}
        />
      </Panel>
      <Toast msg={toast} />
    </main>
  );
}

const optNum = (s: string): number | null => {
  const t = s.trim().replace(/[,_\s]/g, '');
  if (t === '') return null;
  const n = Number(t);
  return Number.isFinite(n) ? Math.floor(n) : null;
};

function PlanForm({
  initial,
  withId,
  submitLabel,
  onSubmit,
  toast,
}: {
  initial: PlanInput;
  withId?: boolean;
  submitLabel: string;
  onSubmit: (body: PlanInput, id?: string) => Promise<void>;
  toast: Toaster;
}) {
  const [id, setId] = useState('');
  const [name, setName] = useState(initial.name);
  const [price, setPrice] = useState(String(initial.priceIqd));
  const [events, setEvents] = useState(
    initial.eventsPerMonth === null ? '' : String(initial.eventsPerMonth),
  );
  const [hard, setHard] = useState(initial.hardLimit);
  const [projects, setProjects] = useState(
    initial.maxProjects === null ? '' : String(initial.maxProjects),
  );
  const [members, setMembers] = useState(
    initial.maxMembers === null ? '' : String(initial.maxMembers),
  );
  const [archived, setArchived] = useState(initial.archived);
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    try {
      await onSubmit(
        {
          name: name.trim(),
          priceIqd: optNum(price) ?? 0,
          eventsPerMonth: optNum(events),
          hardLimit: hard,
          maxProjects: optNum(projects),
          maxMembers: optNum(members),
          archived,
        },
        withId ? id.trim() : undefined,
      );
      if (withId) {
        setId('');
        setName('');
      }
    } catch (err) {
      toast(errText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      style={{ ...formStyle, background: withId ? undefined : 'var(--sidebar)' }}
      onSubmit={(e) => void submit(e)}
    >
      {withId && (
        <Field label="ID" hint="Lowercase, e.g. team-yearly; can't change later">
          <input
            className="input mono"
            value={id}
            onChange={(e) => setId(e.target.value.toLowerCase())}
            pattern="[a-z0-9][a-z0-9-]{0,39}"
            required
          />
        </Field>
      )}
      <Field label="Name">
        <input
          className="input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
          maxLength={60}
        />
      </Field>
      <Field label="Price per month (IQD)">
        <input
          className="input mono"
          inputMode="numeric"
          value={price}
          onChange={(e) => setPrice(e.target.value)}
        />
      </Field>
      <Field label="Events per month" hint="Empty = unlimited">
        <input
          className="input mono"
          inputMode="numeric"
          value={events}
          onChange={(e) => setEvents(e.target.value)}
        />
      </Field>
      <Field label="Max projects" hint="Empty = unlimited">
        <input
          className="input mono"
          inputMode="numeric"
          value={projects}
          onChange={(e) => setProjects(e.target.value)}
        />
      </Field>
      <Field label="Max members" hint="Empty = unlimited">
        <input
          className="input mono"
          inputMode="numeric"
          value={members}
          onChange={(e) => setMembers(e.target.value)}
        />
      </Field>
      <label className="row" style={{ minHeight: 40, color: 'var(--text)', gap: 8 }}>
        <input type="checkbox" checked={hard} onChange={(e) => setHard(e.target.checked)} />
        Stop at the limit (otherwise count overage)
      </label>
      {!withId && (
        <label className="row" style={{ minHeight: 40, color: 'var(--text)', gap: 8 }}>
          <input
            type="checkbox"
            checked={archived}
            onChange={(e) => setArchived(e.target.checked)}
          />
          Archived (no new organizations)
        </label>
      )}
      <div>
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {submitLabel}
        </button>
      </div>
    </form>
  );
}
