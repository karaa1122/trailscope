import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useState, type ReactNode } from 'react';

import { QueryBar } from '../components/QueryBar';
import { Empty, SecretOnce, Segmented, Skeleton, Toast, useToast } from '../components/ui';
import { ApiError, api, type RuleInput } from '../lib/api';
import { fmtAgo, fmtDateTime, fmtInt, fmtMs } from '../lib/format';
import { EMPTY_QUERY, toInput, type QueryValue } from '../lib/query';
import { useProject, useSession } from '../lib/session';
import type { AlertChannel, AlertRule, ChannelKind, EmailRule, Role, RuleKind, SendResult } from '../lib/types';

export type AlertsTab = 'rules' | 'email' | 'channels' | 'history';

const RANK: Record<Role, number> = { viewer: 0, member: 1, admin: 2, owner: 3 };
const errText = (e: unknown) => (e instanceof ApiError || e instanceof Error ? e.message : 'Something went wrong');

const KINDS: { id: RuleKind; label: string; unit: string; help: string }[] = [
  { id: 'error_rate', label: '5xx rate', unit: '%', help: 'Share of matching requests that returned 5xx.' },
  { id: 'p95', label: 'p95 latency', unit: 'ms', help: '95th percentile of duration_ms of matching requests.' },
  { id: 'count', label: 'Count', unit: '', help: 'Number of matching requests (or events).' },
  { id: 'new_error', label: 'New error group', unit: 'groups', help: 'Error groups first seen in the window.' },
  { id: 'spiking_error', label: 'Spiking error group', unit: 'groups', help: 'Groups with ≥ 10 errors and 5× their usual rate.' },
];
const kindInfo = (k: RuleKind) => KINDS.find((x) => x.id === k) ?? KINDS[0]!;

function fmtValue(kind: RuleKind, v: number): string {
  if (kind === 'p95') return fmtMs(v);
  if (kind === 'error_rate') return `${v.toFixed(v < 10 ? 2 : 1)}%`;
  return fmtInt(v);
}

export function Alerts({ tab }: { tab?: AlertsTab }) {
  const navigate = useNavigate();
  const [toast, setToast] = useToast();
  const current = tab ?? 'rules';
  const tabs: { id: AlertsTab; label: string }[] = [
    { id: 'rules', label: 'Rules' },
    { id: 'email', label: 'Email on 5xx' },
    { id: 'channels', label: 'Channels' },
    { id: 'history', label: 'History' },
  ];
  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Alerts</h1>
        <span className="panel-sub">rules are checked every minute; Email on 5xx runs in real time</span>
      </header>
      <div className="tabs" role="tablist" style={{ padding: 0, border: 0 }}>
        {tabs.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={t.id === current} onClick={() => void navigate({ to: '/alerts', search: { tab: t.id } })}>
            {t.label}
          </button>
        ))}
      </div>
      {current === 'rules' && <RulesTab toast={setToast} />}
      {current === 'email' && <EmailTab toast={setToast} />}
      {current === 'channels' && <ChannelsTab toast={setToast} />}
      {current === 'history' && <HistoryTab />}
      <Toast msg={toast} />
    </main>
  );
}

type Toaster = (m: string) => void;

function Panel({ title, sub, actions, children }: { title: string; sub?: ReactNode; actions?: ReactNode; children: ReactNode }) {
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

function Result({ r }: { r: SendResult | null }) {
  if (!r) return null;
  return (
    <div
      role="status"
      className="mono"
      style={{
        margin: '0 18px 18px',
        padding: '10px 12px',
        borderRadius: 8,
        fontSize: 12.5,
        overflowWrap: 'anywhere',
        background: r.ok ? 'var(--ok-soft)' : 'var(--err-soft)',
        color: r.ok ? 'var(--text)' : 'var(--err-text)',
      }}
    >
      {r.ok ? `Sent to ${r.recipients.join(', ')}. Server said: ${r.response ?? 'ok'}` : `Failed: ${r.error ?? 'unknown error'}`}
    </div>
  );
}

// ---- rules ----------------------------------------------------------------------------

function RulesTab({ toast }: { toast: Toaster }) {
  const project = useProject();
  const { me } = useSession();
  const qc = useQueryClient();
  const canEdit = RANK[me?.user.role ?? 'viewer'] >= RANK.member;
  const [editing, setEditing] = useState<AlertRule | 'new' | null>(null);
  const rules = useQuery({ queryKey: ['rules', project.id], queryFn: () => api.rules(project.id), refetchInterval: 30_000 });
  const channels = useQuery({ queryKey: ['channels', project.id], queryFn: () => api.channels(project.id) });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteRule(project.id, id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['rules', project.id] }),
    onError: (e) => toast(errText(e)),
  });
  const channelName = (id: string) => channels.data?.find((c) => c.id === id)?.name ?? 'deleted channel';

  if (editing) {
    return (
      <RuleEditor
        rule={editing === 'new' ? null : editing}
        channels={channels.data ?? []}
        onDone={() => {
          setEditing(null);
          void qc.invalidateQueries({ queryKey: ['rules', project.id] });
        }}
        toast={toast}
      />
    );
  }
  const cols = '90px minmax(0, 1.4fr) minmax(0, 1.6fr) 110px minmax(0, 1fr) 150px';
  return (
    <Panel
      title="Rules"
      sub="a query, a condition and a window; one notification per state change"
      actions={
        canEdit && (
          <button type="button" className="btn btn-sm btn-primary" onClick={() => setEditing('new')}>
            New rule
          </button>
        )
      }
    >
      <div className="scroll-x">
        <div style={{ minWidth: 900 }}>
          <div className="table-head" style={{ gridTemplateColumns: cols }}>
            <span>State</span>
            <span>Name</span>
            <span>Condition</span>
            <span>Now</span>
            <span>Channels</span>
            <span />
          </div>
          {rules.isLoading && <Skeleton h={120} />}
          {rules.data?.length === 0 && (
            <Empty title="No rules yet">
              For example: 5xx rate above 5% over 5 minutes on <code className="mono">route = '/checkout'</code>, or any new error group in production.
            </Empty>
          )}
          {rules.data?.map((r) => {
            const k = kindInfo(r.kind);
            const q = r.query.input && 'text' in r.query.input ? r.query.input.text : r.query.input ? `${r.query.input.chips.length} filters` : '';
            return (
              <div key={r.id} className={`table-row${r.state === 'firing' ? ' err' : ''}`} style={{ gridTemplateColumns: cols, cursor: 'default', padding: '10px 18px' }}>
                <span>
                  {!r.enabled ? (
                    <span className="badge steady">off</span>
                  ) : r.state === 'firing' ? (
                    <span className="badge spike">FIRING</span>
                  ) : (
                    <span className="badge steady">ok</span>
                  )}
                </span>
                <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                  <span className="ellipsis" style={{ fontWeight: 500 }}>
                    {r.name}
                  </span>
                  {r.lastError && (
                    <span className="err-text ellipsis" style={{ fontSize: 11.5 }} title={r.lastError}>
                      {r.lastError}
                    </span>
                  )}
                </span>
                <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                  <span className="cell-mono ellipsis">
                    {k.label} &gt; {fmtValue(r.kind, r.threshold)} over {r.windowMinutes} min
                  </span>
                  <span className="mono dim ellipsis" style={{ fontSize: 11 }} title={q}>
                    {[r.environment, q].filter(Boolean).join(' · ') || 'all requests'}
                  </span>
                </span>
                <span className="cell-mono" title={r.lastEvaluatedAt ? `checked ${fmtAgo(Date.parse(r.lastEvaluatedAt))}` : 'not checked yet'}>
                  {r.lastValue === null ? '—' : fmtValue(r.kind, r.lastValue)}
                </span>
                <span className="muted ellipsis" style={{ fontSize: 12 }}>
                  {r.channelIds.length ? r.channelIds.map(channelName).join(', ') : 'no channels'}
                </span>
                <span className="row" style={{ justifyContent: 'flex-end', gap: 6 }}>
                  {canEdit && (
                    <>
                      <button type="button" className="btn btn-sm" onClick={() => setEditing(r)}>
                        Edit
                      </button>
                      <button type="button" className="btn btn-sm" onClick={() => window.confirm(`Delete “${r.name}”?`) && remove.mutate(r.id)}>
                        Delete
                      </button>
                    </>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </Panel>
  );
}

function toValue(input: AlertRule['query']['input']): QueryValue {
  if (!input) return EMPTY_QUERY;
  if (input.mode === 'filters') return { mode: 'filters', text: '', chips: input.chips };
  return { mode: input.mode, text: input.text, chips: [] };
}

function RuleEditor({ rule, channels, onDone, toast }: { rule: AlertRule | null; channels: AlertChannel[]; onDone: () => void; toast: Toaster }) {
  const project = useProject();
  const [name, setName] = useState(rule?.name ?? '');
  const [kind, setKind] = useState<RuleKind>(rule?.kind ?? 'error_rate');
  const [query, setQuery] = useState<QueryValue>(toValue(rule?.query.input ?? null));
  const [environment, setEnvironment] = useState(rule?.environment ?? 'production');
  const [threshold, setThreshold] = useState(String(rule?.threshold ?? 5));
  const [windowMinutes, setWindow] = useState(String(rule?.windowMinutes ?? 5));
  const [cooldown, setCooldown] = useState(String(rule?.cooldownMinutes ?? 10));
  const [channelIds, setChannelIds] = useState<string[]>(rule?.channelIds ?? []);
  const [enabled, setEnabled] = useState(rule?.enabled ?? true);
  const [preview, setPreview] = useState<{ value: number; firing: boolean } | string | null>(null);
  const envs = useQuery({ queryKey: ['environments', project.id], queryFn: () => api.environments(project.id) });
  const info = kindInfo(kind);
  const usesQuery = kind === 'count' || kind === 'error_rate' || kind === 'p95';

  const input = (): RuleInput => ({
    project: project.id,
    name: name.trim(),
    kind,
    query: { table: 'requests', input: usesQuery && (query.text.trim() || query.chips.length) ? toInput(query) : null },
    environment,
    threshold: Number(threshold),
    windowMinutes: Number(windowMinutes),
    cooldownMinutes: Number(cooldown),
    channelIds,
    enabled,
  });
  const save = useMutation({
    mutationFn: () => (rule ? api.updateRule(rule.id, input()) : api.createRule(input())),
    onSuccess: () => {
      toast(rule ? 'Rule saved' : 'Rule created');
      onDone();
    },
    onError: (e) => toast(errText(e)),
  });
  const runPreview = async () => {
    try {
      setPreview(await api.previewRule({ ...input(), name: name.trim() || 'preview' }));
    } catch (e) {
      setPreview(errText(e));
    }
  };

  return (
    <Panel title={rule ? `Edit “${rule.name}”` : 'New rule'}>
      <form
        style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 16 }}
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))', gap: 16 }}>
          <label className="field">
            Name
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} required maxLength={120} placeholder="Checkout failing" />
          </label>
          <label className="field">
            Environment
            <select className="input" value={environment} onChange={(e) => setEnvironment(e.target.value)}>
              <option value="">all environments</option>
              {[...new Set([...(envs.data ?? []), environment].filter(Boolean))].map((e) => (
                <option key={e} value={e}>
                  {e}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className="field">
          Condition
          <Segmented small label="Condition" value={kind} onChange={setKind} options={KINDS.map((k) => ({ id: k.id, label: k.label }))} />
          <span className="dim" style={{ fontSize: 11.5 }}>
            {info.help}
          </span>
        </label>
        {usesQuery && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <span style={{ fontSize: 12.5, color: 'var(--muted)' }}>Filter (run it with ⌘↵ or Run to apply)</span>
            <QueryBar title="Rule filter" value={query} onRun={setQuery} />
          </div>
        )}
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 16 }}>
          <label className="field">
            Fires when above {info.unit && `(${info.unit})`}
            <input className="input mono" type="number" min={0} step="any" value={threshold} onChange={(e) => setThreshold(e.target.value)} required />
          </label>
          <label className="field">
            Window (minutes)
            <input className="input mono" type="number" min={1} max={1440} value={windowMinutes} onChange={(e) => setWindow(e.target.value)} required />
          </label>
          <label className="field">
            Cooldown (minutes)
            <input className="input mono" type="number" min={0} max={10080} value={cooldown} onChange={(e) => setCooldown(e.target.value)} required />
          </label>
        </div>
        <fieldset style={{ border: 0, padding: 0, margin: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
          <legend style={{ fontSize: 12.5, color: 'var(--muted)', marginBottom: 6 }}>Notify</legend>
          {channels.length === 0 && (
            <span className="muted" style={{ fontSize: 13 }}>
              No channels yet. Add one under <Link to="/alerts" search={{ tab: 'channels' }}>Channels</Link>; the rule still records its state without one.
            </span>
          )}
          <span className="row wrap" style={{ gap: 14 }}>
            {channels.map((c) => (
              <label key={c.id} className="row" style={{ fontSize: 13 }}>
                <input
                  type="checkbox"
                  checked={channelIds.includes(c.id)}
                  onChange={(e) => setChannelIds((ids) => (e.target.checked ? [...ids, c.id] : ids.filter((x) => x !== c.id)))}
                />
                {c.name} <span className="dim mono" style={{ fontSize: 11 }}>{c.kind}</span>
              </label>
            ))}
          </span>
        </fieldset>
        <label className="row" style={{ fontSize: 13 }}>
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Enabled
        </label>
        <div className="row wrap">
          <button type="submit" className="btn btn-primary" disabled={save.isPending}>
            {rule ? 'Save rule' : 'Create rule'}
          </button>
          <button type="button" className="btn" onClick={() => void runPreview()}>
            Check now
          </button>
          <button type="button" className="btn btn-ghost" onClick={onDone}>
            Cancel
          </button>
          {preview !== null && (
            <span className="mono" role="status" style={{ fontSize: 12.5, color: typeof preview === 'string' ? 'var(--err-text)' : preview.firing ? 'var(--err-text)' : 'var(--text-2)' }}>
              {typeof preview === 'string' ? preview : `now ${fmtValue(kind, preview.value)} → ${preview.firing ? 'would fire' : 'ok'}`}
            </span>
          )}
        </div>
      </form>
    </Panel>
  );
}

// ---- Email on 5xx ---------------------------------------------------------------------

function EmailTab({ toast }: { toast: Toaster }) {
  const project = useProject();
  const { me } = useSession();
  const qc = useQueryClient();
  const isAdmin = RANK[me?.user.role ?? 'viewer'] >= RANK.admin;
  const rule = useQuery({ queryKey: ['email-rule', project.id], queryFn: () => api.emailRule(project.id) });
  const members = useQuery({ queryKey: ['members'], queryFn: api.members, enabled: isAdmin });
  const mutes = useQuery({ queryKey: ['mutes', project.id], queryFn: () => api.mutes(project.id) });
  const [draft, setDraft] = useState<{ enabled: boolean; filter: string; memberIds: string[]; emails: string[] } | null>(null);
  const [newEmail, setNewEmail] = useState('');
  const [result, setResult] = useState<SendResult | null>(null);
  useEffect(() => {
    if (rule.data) setDraft({ enabled: rule.data.enabled, filter: rule.data.filter, memberIds: rule.data.memberIds, emails: rule.data.emails });
  }, [rule.data]);

  const save = useMutation({
    mutationFn: () => api.saveEmailRule({ project: project.id, ...(draft ?? { enabled: true, filter: '', memberIds: [], emails: [] }) }),
    onSuccess: (r: EmailRule) => {
      qc.setQueryData(['email-rule', project.id], r);
      toast('Email on 5xx saved');
    },
    onError: (e) => toast(errText(e)),
  });
  const test = useMutation({
    mutationFn: () => api.testEmailRule(project.id),
    onSuccess: setResult,
    onError: (e) => setResult({ ok: false, error: errText(e), response: null, recipients: [] }),
  });
  const unmute = useMutation({
    mutationFn: (fp: string) => api.unmute(project.id, fp),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['mutes', project.id] }),
  });

  // Who will get it, computed from the draft so the list updates while editing.
  const recipients = useMemo(() => {
    if (!draft) return [];
    const out: { email: string; name: string }[] = [];
    for (const id of draft.memberIds) {
      const m = members.data?.find((x) => x.id === id) ?? rule.data?.recipients.find((r) => r.memberId === id);
      if (m && !('disabled' in m && m.disabled)) out.push({ email: m.email, name: m.name });
    }
    for (const e of draft.emails) if (!out.some((r) => r.email.toLowerCase() === e.toLowerCase())) out.push({ email: e, name: '' });
    return out;
  }, [draft, members.data, rule.data]);
  const dirty =
    !!draft &&
    !!rule.data &&
    (draft.enabled !== rule.data.enabled ||
      draft.filter !== rule.data.filter ||
      draft.memberIds.join() !== rule.data.memberIds.join() ||
      draft.emails.join() !== rule.data.emails.join());

  if (rule.isLoading || !draft || !rule.data) return <Skeleton h={300} />;
  const r = rule.data;
  const addEmail = () => {
    const e = newEmail.trim().toLowerCase();
    if (!e) return;
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(e)) {
      toast(`“${e}” is not an email address`);
      return;
    }
    if (!draft.emails.includes(e)) setDraft({ ...draft, emails: [...draft.emails, e] });
    setNewEmail('');
  };

  return (
    <>
      {!r.smtpConfigured && (
        <div className="panel panel-pad" role="alert" style={{ borderColor: 'var(--warn-line)', background: 'var(--warn-soft)', fontSize: 13 }}>
          SMTP isn't configured, so no email can be sent yet.{' '}
          {isAdmin ? (
            <Link to="/settings" search={{ tab: 'notifications' }}>
              Set it up in Settings → Notifications
            </Link>
          ) : (
            'Ask an admin to set it up in Settings → Notifications.'
          )}
        </div>
      )}

      <Panel
        title="Recipients"
        sub="only the people listed here get the 5xx emails"
        actions={
          isAdmin && (
            <button type="button" className="btn btn-sm" disabled={test.isPending || dirty || r.recipients.length === 0 || !r.smtpConfigured} onClick={() => test.mutate()} title={dirty ? 'Save first' : undefined}>
              Send a test email to them
            </button>
          )
        }
      >
        <div style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div
            role="status"
            style={{
              padding: '12px 14px',
              borderRadius: 10,
              border: `1px solid ${recipients.length ? 'var(--line-strong)' : 'var(--warn-line)'}`,
              background: recipients.length ? 'var(--sidebar)' : 'var(--warn-soft)',
              fontSize: 13,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}
          >
            {recipients.length === 0 ? (
              <span>Nobody is listed, so no 5xx emails are sent. Choose members or add addresses below.</span>
            ) : (
              <>
                <span>
                  {recipients.length === 1 ? 'This person gets' : `These ${recipients.length} people get`} the email:
                </span>
                <span className="row wrap" style={{ gap: 6 }}>
                  {recipients.map((x) => (
                    <span key={x.email} className="chip" title={x.name || undefined}>
                      {x.name ? `${x.name} <${x.email}>` : x.email}
                    </span>
                  ))}
                </span>
              </>
            )}
          </div>

          {isAdmin && (
            <>
              <fieldset style={{ border: 0, padding: 0, margin: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
                <legend className="section-title" style={{ padding: 0, marginBottom: 8 }}>
                  Members
                </legend>
                {members.isLoading && <Skeleton h={40} />}
                <span style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 8 }}>
                  {members.data
                    ?.filter((m) => !m.disabled)
                    .map((m) => (
                      <label key={m.id} className="row" style={{ fontSize: 13, padding: '8px 10px', border: '1px solid var(--line)', borderRadius: 8, minWidth: 0 }}>
                        <input
                          type="checkbox"
                          checked={draft.memberIds.includes(m.id)}
                          onChange={(e) =>
                            setDraft({
                              ...draft,
                              memberIds: e.target.checked ? [...draft.memberIds, m.id] : draft.memberIds.filter((x) => x !== m.id),
                            })
                          }
                        />
                        <span style={{ display: 'flex', flexDirection: 'column', minWidth: 0 }}>
                          <span className="ellipsis">{m.name || m.email}</span>
                          <span className="mono dim ellipsis" style={{ fontSize: 11 }}>
                            {m.email}
                          </span>
                        </span>
                      </label>
                    ))}
                </span>
              </fieldset>

              <fieldset style={{ border: 0, padding: 0, margin: 0, display: 'flex', flexDirection: 'column', gap: 8 }}>
                <legend className="section-title" style={{ padding: 0, marginBottom: 8 }}>
                  Other addresses
                </legend>
                <span className="row wrap" style={{ gap: 6 }}>
                  {draft.emails.map((e) => (
                    <span key={e} className="chip">
                      {e}
                      <button
                        type="button"
                        aria-label={`Remove ${e}`}
                        onClick={() => setDraft({ ...draft, emails: draft.emails.filter((x) => x !== e) })}
                        style={{ border: 0, background: 'none', color: 'var(--muted)', cursor: 'pointer', padding: 0 }}
                      >
                        ×
                      </button>
                    </span>
                  ))}
                  {draft.emails.length === 0 && <span className="dim" style={{ fontSize: 12.5 }}>none</span>}
                </span>
                <span className="row">
                  <input
                    className="input"
                    type="email"
                    value={newEmail}
                    onChange={(e) => setNewEmail(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        addEmail();
                      }
                    }}
                    placeholder="oncall@example.com"
                    aria-label="Add an email address"
                    style={{ maxWidth: 320 }}
                  />
                  <button type="button" className="btn" onClick={addEmail}>
                    Add
                  </button>
                </span>
              </fieldset>
            </>
          )}
        </div>
        <Result r={result} />
      </Panel>

      <Panel
        title="Rule"
        sub={
          r.active ? (
            <span className="ok-text">active: emails go out within seconds of a matching 5xx</span>
          ) : (
            <span>inactive: {!draft.enabled ? 'turned off' : !r.smtpConfigured ? 'SMTP not configured' : 'no recipients'}</span>
          )
        }
      >
        <form
          style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 14 }}
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <fieldset disabled={!isAdmin} style={{ border: 0, padding: 0, margin: 0, display: 'flex', flexDirection: 'column', gap: 14 }}>
            <label className="row" style={{ fontSize: 13 }}>
              <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} /> Send an email when a matching
              5xx happens
            </label>
            <label className="field">
              Filter
              <input className="input mono" value={draft.filter} onChange={(e) => setDraft({ ...draft, filter: e.target.value })} spellCheck={false} />
              <span className="dim" style={{ fontSize: 11.5 }}>
                Query language; usable columns: status, environment, service, release, method, route, duration_ms, user_id, error_type, error_message,
                attributes[…], correlation[…].
              </span>
            </label>
          </fieldset>
          <ul className="muted" style={{ margin: 0, paddingLeft: 18, fontSize: 12.5, lineHeight: 1.7 }}>
            <li>The first occurrence of an error group is emailed immediately.</li>
            <li>After that, at most one email per group every {r.throttleMinutes} minutes, with a digest only if it kept happening.</li>
            <li>A group that was quiet for {r.regressionHours} hours and comes back is emailed again.</li>
            <li>At most {r.hourlyCap} emails per hour for this project; then a single “paused” email.</li>
            <li>Emails never include request or response bodies, headers or IP addresses.</li>
          </ul>
          {isAdmin && (
            <div className="row">
              <button type="submit" className="btn btn-primary" disabled={!dirty || save.isPending}>
                Save
              </button>
              {dirty && <span className="muted" style={{ fontSize: 12 }}>unsaved changes (recipients included)</span>}
            </div>
          )}
        </form>
      </Panel>

      <Panel title="Muted errors" sub="muted from the links in the emails">
        {mutes.data?.length === 0 && <Empty title="Nothing muted" />}
        {mutes.data?.map((m) => (
          <div key={m.fingerprint} className="kv" style={{ alignItems: 'center' }}>
            <span className="row" style={{ gap: 10 }}>
              <Link to="/errors/$fingerprint" params={{ fingerprint: m.fingerprint }} className="mono" style={{ fontSize: 12.5 }}>
                {m.fingerprint}
              </Link>
              <span className="muted">{m.forever ? 'forever' : `until ${fmtDateTime(Date.parse(m.mutedUntil ?? ''))}`}</span>
            </span>
            {RANK[me?.user.role ?? 'viewer'] >= RANK.member && (
              <button type="button" className="btn btn-sm" onClick={() => unmute.mutate(m.fingerprint)}>
                Unmute
              </button>
            )}
          </div>
        ))}
      </Panel>
    </>
  );
}

// ---- channels -------------------------------------------------------------------------

function ChannelsTab({ toast }: { toast: Toaster }) {
  const project = useProject();
  const { me } = useSession();
  const qc = useQueryClient();
  const isAdmin = RANK[me?.user.role ?? 'viewer'] >= RANK.admin;
  const channels = useQuery({ queryKey: ['channels', project.id], queryFn: () => api.channels(project.id) });
  const members = useQuery({ queryKey: ['members'], queryFn: api.members, enabled: isAdmin });
  const [kind, setKind] = useState<ChannelKind>('slack');
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [botToken, setBotToken] = useState('');
  const [chatId, setChatId] = useState('');
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [emails, setEmails] = useState('');
  const [secret, setSecret] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, SendResult>>({});
  const refresh = () => void qc.invalidateQueries({ queryKey: ['channels', project.id] });
  const create = useMutation({
    mutationFn: () =>
      api.createChannel({
        project: project.id,
        name: name.trim(),
        kind,
        url,
        botToken,
        chatId,
        memberIds,
        emails: emails.split(/[\s,;]+/).filter(Boolean),
      }),
    onSuccess: (r) => {
      setSecret(r.signingSecret);
      setName('');
      setUrl('');
      setBotToken('');
      setChatId('');
      setMemberIds([]);
      setEmails('');
      refresh();
      toast('Channel added');
    },
    onError: (e) => toast(errText(e)),
  });
  const remove = useMutation({ mutationFn: (id: string) => api.deleteChannel(project.id, id), onSuccess: refresh, onError: (e) => toast(errText(e)) });
  const test = useMutation({
    mutationFn: (id: string) => api.testChannel(project.id, id),
    onSuccess: (r, id) => setResults((m) => ({ ...m, [id]: r })),
    onError: (e, id) => setResults((m) => ({ ...m, [id]: { ok: false, error: errText(e), response: null, recipients: [] } })),
  });
  const describe = (c: AlertChannel) => {
    if (c.kind === 'email') {
      const names = c.config.memberIds.map((id) => members.data?.find((m) => m.id === id)?.email ?? 'member');
      return [...names, ...c.config.emails].join(', ');
    }
    if (c.kind === 'telegram') return `chat ${c.config.chatId}`;
    return c.config.urlHost;
  };

  return (
    <>
      <Panel title="Channels" sub="where rule notifications go; webhooks are signed with HMAC-SHA256">
        {secret && (
          <div style={{ padding: 18 }}>
            <SecretOnce label="Webhook signing secret. Verify X-Logbox-Signature: sha256=HMAC(secret, timestamp + '.' + body)." value={secret} onDone={() => setSecret(null)} />
          </div>
        )}
        {channels.isLoading && <Skeleton h={80} />}
        {channels.data?.length === 0 && <Empty title="No channels yet" />}
        {channels.data?.map((c) => (
          <div key={c.id}>
            <div className="kv" style={{ alignItems: 'center' }}>
              <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span style={{ color: 'var(--text)', fontSize: 13 }}>
                  {c.name} <span className="badge steady">{c.kind}</span>
                </span>
                <span className="mono dim ellipsis" style={{ fontSize: 11.5 }}>
                  {describe(c)}
                </span>
              </span>
              {isAdmin && (
                <span className="row" style={{ gap: 6 }}>
                  <button type="button" className="btn btn-sm" disabled={test.isPending} onClick={() => test.mutate(c.id)}>
                    Send test
                  </button>
                  <button type="button" className="btn btn-sm" onClick={() => window.confirm(`Delete “${c.name}”? Rules stop using it.`) && remove.mutate(c.id)}>
                    Delete
                  </button>
                </span>
              )}
            </div>
            <Result r={results[c.id] ?? null} />
          </div>
        ))}
      </Panel>

      {isAdmin && (
        <Panel title="Add a channel">
          <form
            style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 14 }}
            onSubmit={(e) => {
              e.preventDefault();
              create.mutate();
            }}
          >
            <Segmented
              small
              label="Channel kind"
              value={kind}
              onChange={setKind}
              options={[
                { id: 'slack', label: 'Slack' },
                { id: 'webhook', label: 'Webhook' },
                { id: 'telegram', label: 'Telegram' },
                { id: 'email', label: 'Email' },
              ]}
            />
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 14 }}>
              <label className="field">
                Name
                <input className="input" value={name} onChange={(e) => setName(e.target.value)} required placeholder="#ops alerts" />
              </label>
              {(kind === 'slack' || kind === 'webhook') && (
                <label className="field">
                  {kind === 'slack' ? 'Incoming webhook URL' : 'URL'}
                  <input className="input mono" value={url} onChange={(e) => setUrl(e.target.value)} required placeholder="https://hooks.slack.com/services/…" />
                </label>
              )}
              {kind === 'telegram' && (
                <>
                  <label className="field">
                    Bot token
                    <input className="input mono" value={botToken} onChange={(e) => setBotToken(e.target.value)} required placeholder="123456:ABC…" />
                  </label>
                  <label className="field">
                    Chat ID
                    <input className="input mono" value={chatId} onChange={(e) => setChatId(e.target.value)} required placeholder="-1001234567890" />
                  </label>
                </>
              )}
              {kind === 'email' && (
                <label className="field">
                  Other addresses
                  <input className="input" value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="oncall@example.com, lead@example.com" />
                </label>
              )}
            </div>
            {kind === 'email' && (
              <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
                <legend style={{ fontSize: 12.5, color: 'var(--muted)', marginBottom: 6 }}>Members who receive it</legend>
                <span className="row wrap" style={{ gap: 14 }}>
                  {members.data
                    ?.filter((m) => !m.disabled)
                    .map((m) => (
                      <label key={m.id} className="row" style={{ fontSize: 13 }}>
                        <input
                          type="checkbox"
                          checked={memberIds.includes(m.id)}
                          onChange={(e) => setMemberIds((ids) => (e.target.checked ? [...ids, m.id] : ids.filter((x) => x !== m.id)))}
                        />
                        {m.name || m.email}
                      </label>
                    ))}
                </span>
              </fieldset>
            )}
            <div className="row">
              <button type="submit" className="btn btn-primary" disabled={create.isPending}>
                Add channel
              </button>
            </div>
          </form>
        </Panel>
      )}
    </>
  );
}

// ---- history --------------------------------------------------------------------------

function HistoryTab() {
  const project = useProject();
  const [status, setStatus] = useState<'all' | 'sent' | 'failed' | 'pending'>('all');
  const rows = useQuery({
    queryKey: ['history', project.id, status],
    queryFn: () => api.history(project.id, status === 'all' ? undefined : status, 200),
    refetchInterval: 10_000,
  });
  const cols = '150px 84px minmax(0, 1fr) minmax(0, 2fr) 120px';
  return (
    <Panel
      title="History"
      sub="every notification: time, recipient, rule, error group and the server's answer"
      actions={
        <Segmented
          small
          label="Status"
          value={status}
          onChange={setStatus}
          options={[
            { id: 'all', label: 'All' },
            { id: 'sent', label: 'Sent' },
            { id: 'failed', label: 'Failed' },
            { id: 'pending', label: 'Retrying' },
          ]}
        />
      }
    >
      <div className="scroll-x">
        <div style={{ minWidth: 900 }}>
          <div className="table-head" style={{ gridTemplateColumns: cols }}>
            <span>Time</span>
            <span>Status</span>
            <span>To</span>
            <span>Message</span>
            <span>Why</span>
          </div>
          {rows.isLoading && <Skeleton h={160} />}
          {rows.data?.length === 0 && <Empty title="Nothing sent yet" />}
          {rows.data?.map((n) => (
            <div key={n.id} className={`table-row${n.status === 'failed' ? ' err' : ''}`} style={{ gridTemplateColumns: cols, cursor: 'default', padding: '10px 18px', alignItems: 'start' }}>
              <span className="cell-mono muted" title={n.createdAt}>
                {fmtDateTime(Date.parse(n.createdAt))}
              </span>
              <span>
                <span className={`badge ${n.status === 'sent' ? 'steady' : n.status === 'failed' ? 'spike' : 'slow'}`}>{n.status === 'pending' ? `retry ${n.attempts}/5` : n.status}</span>
              </span>
              <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span className="cell-mono ellipsis">{n.target}</span>
                <span className="dim mono" style={{ fontSize: 11 }}>
                  {n.channelKind}
                </span>
              </span>
              <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span className="ellipsis" style={{ fontSize: 13 }} title={n.subject}>
                  {n.subject}
                </span>
                {(n.lastError || n.response) && (
                  <span className={`mono ellipsis ${n.lastError ? 'err-text' : 'dim'}`} style={{ fontSize: 11 }} title={n.lastError ?? n.response ?? ''}>
                    {n.lastError ?? n.response}
                  </span>
                )}
              </span>
              <span style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span className="cell-mono">{n.reason}</span>
                <span className="dim ellipsis" style={{ fontSize: 11 }}>
                  {n.ruleName || n.source}
                </span>
                {n.fingerprint && /^\d+$/.test(n.fingerprint) && (
                  <Link to="/errors/$fingerprint" params={{ fingerprint: n.fingerprint }} style={{ fontSize: 11 }}>
                    error group
                  </Link>
                )}
              </span>
            </div>
          ))}
        </div>
      </div>
    </Panel>
  );
}
