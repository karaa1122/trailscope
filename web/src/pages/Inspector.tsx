import * as Dialog from '@radix-ui/react-dialog';
import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useMemo, useState } from 'react';

import { JsonView, Waterfall, countRedacted, type SpanBar } from '../components/charts';
import { IconAlert, IconCopy, IconJourney, IconLink, IconReplay } from '../components/icons';
import { Empty, Skeleton, StatusPill, Toast, copyText, useToast } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtAgo, fmtBytes, fmtClock, fmtDateTime, fmtInt, fmtMs, num, statusClass, str } from '../lib/format';
import { useHotkeys } from '../lib/hotkeys';
import { fromSearch, toInput, type QuerySearch } from '../lib/query';
import { useProject } from '../lib/session';
import type { ReplayResult, Row } from '../lib/types';

export interface InspectorSearch extends QuerySearch {
  from?: number;
  to?: number;
}

type Tab = 'req' | 'res' | 'headers' | 'curl';

const FRAMEWORK = ['node_modules', 'node:internal', 'internal/', 'site-packages', 'dist-packages', '/lib/python', '<frozen ', 'asyncio/'];

export function Inspector({ requestId, search }: { requestId: string; search: InspectorSearch }) {
  const project = useProject();
  const navigate = useNavigate();
  const [tab, setTab] = useState<Tab>('req');
  const [toast, setToast] = useToast();
  const [showFrames, setShowFrames] = useState(false);
  const [replay, setReplay] = useState<ReplayResult | null>(null);
  const detail = useQuery({ queryKey: ['request', project.id, requestId], queryFn: () => api.request(project.id, requestId), retry: false });
  const curl = useQuery({ queryKey: ['curl', project.id, requestId], queryFn: () => api.curl(project.id, requestId), enabled: !!detail.data });
  const targets = useQuery({ queryKey: ['targets', project.id], queryFn: () => api.replayTargets(project.id) });

  const req = detail.data?.request;
  const ts = num(req?.timestamp);
  const filter = useMemo(() => fromSearch(search, 'sql'), [search]);
  const nearby = useQuery({
    queryKey: ['nearby', project.id, JSON.stringify(toInput(filter)), ts],
    enabled: ts > 0,
    queryFn: async () => {
      const q = toInput(filter);
      const [before, after] = await Promise.all([
        api.rows({ project: project.id, query: q, from: ts - 24 * 3_600_000, to: ts, limit: 12, columns: ['request_id', 'timestamp', 'method', 'status', 'route', 'duration_ms', 'user_id'] }),
        api.rows({ project: project.id, query: q, from: ts + 1, to: ts + 24 * 3_600_000, limit: 6, order: 'asc', columns: ['request_id', 'timestamp', 'method', 'status', 'route', 'duration_ms', 'user_id'] }),
      ]);
      return [...after.rows.reverse(), ...before.rows];
    },
  });
  const list = nearby.data ?? [];
  const pos = list.findIndex((r) => str(r.request_id) === requestId);
  const go = (r: Row | undefined) =>
    r && void navigate({ to: '/requests/$requestId', params: { requestId: str(r.request_id) }, search: search as InspectorSearch });

  const copyCurl = async () => {
    const text = curl.data?.command ?? (await api.curl(project.id, requestId)).command;
    setToast((await copyText(text)) ? 'cURL copied' : 'Copy failed');
  };
  const spanBars: SpanBar[] = useSpanBars(detail.data?.spans ?? [], ts, num(req?.duration_ms), str(req?.method), str(req?.route));
  useHotkeys({
    j: () => go(list[pos + 1]),
    k: () => go(list[pos - 1]),
    c: () => void copyCurl(),
  });

  if (detail.isLoading) {
    return (
      <main className="main">
        <Skeleton h={40} />
        <Skeleton h={200} />
      </main>
    );
  }
  if (detail.error || !detail.data || !req) {
    return (
      <main className="main">
        <Empty title="Request not found">
          {detail.error instanceof ApiError && detail.error.status !== 404
            ? detail.error.message
            : `No request ${requestId} in this project. It may be older than the retention period.`}
          <Link to="/explore">Back to Explore</Link>
        </Empty>
      </main>
    );
  }

  const { spans, errorGroup, bodiesHidden } = detail.data;
  const status = num(req.status);
  const duration = num(req.duration_ms);
  const correlation = (req.correlation ?? {}) as Record<string, string>;
  const journeyKey = Object.entries(correlation)[0];
  const hasError = !!str(req.error_type) || status >= 500;
  const stack = str(req.error_stack);
  const frames = stack.split('\n').map((l) => l.trim()).filter((l) => l.startsWith('at ') || l.startsWith('File "'));
  const appFrames = frames.filter((f) => !FRAMEWORK.some((m) => f.includes(m)));
  const hiddenFrames = frames.length - appFrames.length;
  const shownFrames = showFrames ? frames : appFrames.length ? appFrames : frames;
  const headers = Object.entries((req.req_headers ?? {}) as Record<string, string>);
  const resHeaders = Object.entries((req.res_headers ?? {}) as Record<string, string>);
  const reqBody = str(req.req_body);
  const resBody = str(req.res_body);

  const insight = spanInsight(spanBars, duration);

  const doReplay = async (targetId: string) => {
    try {
      setReplay(await api.replay(project.id, requestId, targetId));
    } catch (e) {
      setToast(e instanceof Error ? e.message : 'Replay failed');
    }
  };

  const tabMeta: Record<Tab, string> = {
    req: bodiesHidden ? 'hidden: needs “can view bodies”' : `${fmtBytes(num(req.req_size))}${countRedacted(reqBody) ? ` · ${countRedacted(reqBody)} redacted` : ''}${num(req.body_truncated) ? ' · truncated' : ''}`,
    res: bodiesHidden ? 'hidden' : `${fmtBytes(num(req.res_size))} · ${(resHeaders.find(([k]) => k === 'content-type')?.[1] ?? '').split(';')[0]}`,
    headers: bodiesHidden ? 'hidden' : 'auth and cookies redacted',
    curl: curl.data?.placeholders.length ? `${curl.data.placeholders.length} placeholder${curl.data.placeholders.length > 1 ? 's' : ''}` : 'ready to paste',
  };

  return (
    <>
      <aside aria-label="Nearby requests" style={{ flex: '1 1 290px', borderRight: '1px solid var(--line)', background: 'var(--sunken)', display: 'flex', flexDirection: 'column' }}>
        <div className="row" style={{ padding: '18px 16px 12px' }}>
          <Link to="/explore" search={{ ...search, from: undefined, to: undefined }} style={{ fontSize: 13 }}>
            ← Explore
          </Link>
          <span className="mono muted ellipsis" style={{ marginLeft: 'auto', fontSize: 11, maxWidth: 160 }} title={filter.text}>
            {filter.mode === 'filters' ? `${filter.chips.length} filters` : filter.text || 'all requests'}
          </span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', padding: '0 8px', overflowY: 'auto' }}>
          {nearby.isLoading && <Skeleton h={300} />}
          {list.map((n) => {
            const s = num(n.status);
            const sel = str(n.request_id) === requestId;
            return (
              <Link
                key={str(n.request_id)}
                to="/requests/$requestId"
                params={{ requestId: str(n.request_id) }}
                search={search as InspectorSearch}
                style={{ display: 'flex', flexDirection: 'column', gap: 5, padding: 10, borderRadius: 8, color: 'var(--text)', background: sel ? 'var(--raised)' : 'transparent' }}
              >
                <span className="row">
                  <span className="dot" style={{ width: 7, height: 7, background: s >= 500 ? 'var(--err)' : s >= 400 ? 'var(--warn)' : 'var(--ok)' }} />
                  <span className={`pill ${statusClass(s)}`} style={{ background: 'none', padding: 0 }}>
                    {s}
                  </span>
                  <span className="mono muted" style={{ fontSize: 11 }}>
                    {str(n.method)}
                  </span>
                  <span className="mono muted" style={{ marginLeft: 'auto', fontSize: 11 }}>
                    {fmtMs(num(n.duration_ms))}
                  </span>
                </span>
                <span className="mono ellipsis" style={{ fontSize: 12.5 }}>
                  {str(n.route)}
                </span>
                <span className="mono dim" style={{ fontSize: 11 }}>
                  {fmtClock(num(n.timestamp), false)} · {str(n.user_id) || '—'}
                </span>
              </Link>
            );
          })}
        </div>
        <div className="muted" style={{ marginTop: 'auto', padding: 16, fontSize: 12 }}>
          <kbd>J</kbd> <kbd>K</kbd> next / previous · <kbd>C</kbd> copy cURL
        </div>
      </aside>

      <main className="main" style={{ gap: 16 }}>
        <header style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div className="row wrap" style={{ gap: 12 }}>
            <span className="method-pill">{str(req.method)}</span>
            <h1 className="mono" style={{ margin: 0, fontSize: 22, fontWeight: 500, letterSpacing: '-0.02em', overflowWrap: 'anywhere' }}>
              {str(req.path)}
              {str(req.query) && <span className="muted">?{str(req.query)}</span>}
            </h1>
            <span className={`pill ${statusClass(status)}`} style={{ fontSize: 14, padding: '3px 9px' }}>
              {status}
            </span>
            <span className="mono" style={{ fontSize: 14, color: duration >= project.settings.slowMs ? 'var(--warn)' : 'var(--text-2)' }}>
              {fmtMs(duration)}
            </span>
            <span className="mono muted" style={{ fontSize: 13 }} title={fmtDateTime(ts)}>
              {fmtClock(ts)}
            </span>
          </div>
          <div className="row wrap" style={{ gap: 6 }}>
            <span className="chip">{str(req.route)}</span>
            <span className="chip">{str(req.service)}</span>
            {str(req.user_id) && <span className="chip">user {str(req.user_id)}</span>}
            {str(req.client) && <span className="chip">{[str(req.client), str(req.client_version)].filter(Boolean).join(' ')}</span>}
            {str(req.release) && <span className="chip accent">release {str(req.release)}</span>}
            <span className="chip">{str(req.environment)}</span>
            <span className="chip">{requestId}</span>
          </div>
          <div className="row wrap">
            <button type="button" className="btn btn-primary" onClick={() => void copyCurl()}>
              <IconCopy /> Copy as cURL
            </button>
            {targets.data && targets.data.length > 0 && !bodiesHidden && (
              <Dropdown.Root>
                <Dropdown.Trigger asChild>
                  <button type="button" className="btn">
                    <IconReplay /> Replay{targets.data.length === 1 ? ` on ${targets.data[0]!.name}` : '…'}
                  </button>
                </Dropdown.Trigger>
                <Dropdown.Portal>
                  <Dropdown.Content className="popover" sideOffset={6}>
                    {targets.data.map((t) => (
                      <Dropdown.Item key={t.id} className="menu-item" onSelect={() => void doReplay(t.id)}>
                        {t.name} <span className="nav-hint">{t.baseUrl}</span>
                      </Dropdown.Item>
                    ))}
                  </Dropdown.Content>
                </Dropdown.Portal>
              </Dropdown.Root>
            )}
            <Link
              to="/journeys"
              search={{ q: journeyKey ? `${journeyKey[0]}:${journeyKey[1]}` : `request_id:${requestId}`, came: requestId }}
              className="btn"
            >
              <IconJourney /> Open {journeyKey ? journeyKey[0].replace(/_id$/, '') : 'request'} journey
            </Link>
            <button
              type="button"
              className="btn"
              onClick={() => void copyText(window.location.href).then((ok) => setToast(ok ? 'Link copied' : 'Copy failed'))}
            >
              <IconLink /> Share link
            </button>
          </div>
        </header>

        {hasError && (
          <section aria-label="Error" style={{ border: '1px solid var(--err-line)', background: 'var(--err-faint)', borderRadius: 12, padding: '16px 18px', display: 'flex', flexDirection: 'column', gap: 10 }}>
            <div className="row wrap" style={{ gap: 10 }}>
              <IconAlert style={{ color: 'var(--err-text)' }} />
              <span className="mono" style={{ fontSize: 14, fontWeight: 600, color: 'var(--err-text)' }}>
                {str(req.error_type) || `HTTP ${status}`}
              </span>
              <span style={{ fontSize: 13, color: 'var(--text-3)' }}>{str(req.error_message)}</span>
              {str(req.error_fingerprint) && (
                <Link to="/errors/$fingerprint" params={{ fingerprint: str(req.error_fingerprint) }} style={{ marginLeft: 'auto', fontSize: 12 }}>
                  Error group →
                </Link>
              )}
            </div>
            {frames.length > 0 && (
              <pre className="code" style={{ padding: '12px 14px', background: 'var(--sidebar)', borderRadius: 8, fontSize: 12, lineHeight: 1.7, color: 'var(--muted)' }}>
                {shownFrames.map((f) => (
                  <div key={f}>{f}</div>
                ))}
                {hiddenFrames > 0 && (
                  <button type="button" onClick={() => setShowFrames((s) => !s)} style={{ border: 0, background: 'none', padding: 0, color: 'var(--dim)', cursor: 'pointer', fontFamily: 'var(--mono)', fontSize: 12 }}>
                    {showFrames ? 'hide framework frames' : `… ${hiddenFrames} framework frames hidden`}
                  </button>
                )}
              </pre>
            )}
            {errorGroup && (
              <div className="row wrap" style={{ gap: '6px 18px', fontSize: 12, color: 'var(--text-2)' }}>
                <span>
                  <span className="mono" style={{ color: 'var(--text)' }}>{fmtInt(num(errorGroup.count))}×</span> in this group
                </span>
                <span>
                  <span className="mono" style={{ color: 'var(--text)' }}>{fmtInt(num(errorGroup.users))}</span> users affected
                </span>
                <span>
                  first seen <span className="mono" style={{ color: 'var(--text)' }}>{fmtAgo(num(errorGroup.first_seen))}</span>
                  {str(errorGroup.first_release) && <> in {str(errorGroup.first_release)}</>}
                </span>
              </div>
            )}
          </section>
        )}

        <div className="row wrap" style={{ gap: 16, alignItems: 'flex-start' }}>
          <section aria-label="Payload" className="panel" style={{ flex: '999 1 460px', minWidth: 0, overflow: 'hidden' }}>
            <div role="tablist" aria-label="Payload view" className="tabs">
              {(
                [
                  ['req', 'Request body'],
                  ['res', 'Response'],
                  ['headers', `Headers ${headers.length + resHeaders.length}`],
                  ['curl', 'cURL'],
                ] as [Tab, string][]
              ).map(([id, label]) => (
                <button key={id} type="button" role="tab" aria-selected={tab === id} onClick={() => setTab(id)}>
                  {label}
                </button>
              ))}
              <span className="mono dim" style={{ marginLeft: 'auto', alignSelf: 'center', paddingRight: 8, fontSize: 11 }}>
                {tabMeta[tab]}
              </span>
            </div>
            {bodiesHidden && tab !== 'curl' ? (
              <Empty title="Hidden">Bodies and headers need the “can view bodies” permission. Ask an admin.</Empty>
            ) : tab === 'req' ? (
              <JsonView text={reqBody} />
            ) : tab === 'res' ? (
              <JsonView text={resBody} />
            ) : tab === 'headers' ? (
              <div style={{ padding: '8px 0' }}>
                {[...headers.map(([k, v]) => ['→', k, v]), ...resHeaders.map(([k, v]) => ['←', k, v])].map(([dir, k, v]) => (
                  <div key={`${dir}${k}`} style={{ display: 'grid', gridTemplateColumns: '18px 200px minmax(0, 1fr)', gap: 12, padding: '8px 20px', fontFamily: 'var(--mono)', fontSize: 12.5, borderBottom: '1px solid var(--line-soft)' }}>
                    <span className="dim" title={dir === '→' ? 'request' : 'response'}>
                      {dir}
                    </span>
                    <span style={{ color: 'var(--ok)' }}>{k}</span>
                    {v === '[REDACTED]' ? <span><span className="redacted">REDACTED</span></span> : <span style={{ color: 'var(--text-3)', overflowWrap: 'anywhere' }}>{v}</span>}
                  </div>
                ))}
                {headers.length + resHeaders.length === 0 && <Empty title="No headers captured" />}
              </div>
            ) : (
              <pre className="code">{curl.data?.command ?? '…'}</pre>
            )}
          </section>

          <section aria-label="Request details" className="panel" style={{ flex: '1 1 300px', minWidth: 0, padding: '6px 0' }}>
            {(
              [
                ['Request ID', requestId],
                ['Trace ID', str(req.trace_id) || '—'],
                ['User', str(req.user_id) || '—'],
                ['Client', [str(req.client), str(req.client_version)].filter(Boolean).join(' ') || '—'],
                ['IP', str(req.ip) || '—'],
                ['Service', str(req.service)],
                ['Environment', str(req.environment)],
                ['Release', str(req.release) || '—'],
                ['Request size', fmtBytes(num(req.req_size))],
                ['Response size', fmtBytes(num(req.res_size))],
                ...Object.entries(correlation).map(([k, v]) => [k, v]),
                ...Object.entries((req.attributes ?? {}) as Record<string, string>).map(([k, v]) => [`attr.${k}`, v]),
              ] as [string, string][]
            ).map(([k, v]) => (
              <div key={k} className="kv">
                <span>{k}</span>
                <span>{v}</span>
              </div>
            ))}
          </section>
        </div>

        <section aria-label="Timing" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <div className="panel-head">
            <h2 className="panel-title">Where the time went</h2>
            <span className="panel-sub">
              {str(req.trace_id) ? `trace ${str(req.trace_id).slice(0, 8)}…${str(req.trace_id).slice(-4)} · ` : ''}
              {spans.length} span{spans.length === 1 ? '' : 's'}
            </span>
            {insight && (
              <span className="mono" style={{ marginLeft: 'auto', fontSize: 12, color: insight.hot ? 'var(--err-text)' : 'var(--text-2)' }}>
                {insight.text}
              </span>
            )}
          </div>
          {spanBars.length > 1 ? <Waterfall spans={spanBars} total={duration} /> : <Empty title="No spans">Instrument database and outbound calls to see a breakdown here.</Empty>}
        </section>
      </main>
      <ReplayDialog result={replay} onClose={() => setReplay(null)} />
      <Toast msg={toast} />
    </>
  );
}

function useSpanBars(spans: Row[], ts: number, duration: number, method: string, route: string): SpanBar[] {
  return useMemo(() => {
    const root: SpanBar = { id: 'root', kind: 'http', name: `${method} ${route}`, start: 0, duration, error: false, root: true };
    const children = spans
      .map((s, i) => ({
        id: `${str(s.span_id)}-${i}`,
        kind: str(s.kind),
        name: str(s.name),
        start: Math.max(0, num(s.start) - ts),
        duration: num(s.duration_ms),
        error: str(s.status) === 'error',
        root: false,
      }))
      .sort((a, b) => a.start - b.start);
    return [root, ...children];
  }, [spans, ts, duration, method, route]);
}

const KIND_LABEL: Record<string, string> = {
  external: 'an external call',
  db: 'the database',
  cache: 'the cache',
  queue: 'a queue',
  http: 'a downstream service',
  fn: 'application code',
};

function spanInsight(spans: SpanBar[], total: number): { text: string; hot: boolean } | null {
  const kids = spans.filter((s) => !s.root);
  if (!kids.length || total <= 0) return null;
  const top = kids.reduce((a, b) => (b.duration > a.duration ? b : a));
  const share = top.duration / total;
  if (share < 0.3) return { text: `time spread across ${kids.length} spans`, hot: false };
  return { text: `${Math.round(share * 100)}% waiting on ${KIND_LABEL[top.kind] ?? top.kind}`, hot: share > 0.6 };
}

function ReplayDialog({ result, onClose }: { result: ReplayResult | null; onClose: () => void }) {
  return (
    <Dialog.Root open={!!result} onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog" aria-describedby={undefined}>
          <div className="panel-bar">
            <Dialog.Title className="panel-title">Replay result</Dialog.Title>
            {result?.status ? <StatusPill status={result.status} /> : <span className="err-text">failed</span>}
            <span className="mono muted" style={{ fontSize: 12 }}>
              {result && fmtMs(result.durationMs)}
            </span>
          </div>
          <div style={{ padding: '10px 18px' }} className="mono muted">
            {result?.url}
          </div>
          {result?.error ? <pre className="code err-text">{result.error}</pre> : <JsonView text={result?.body ?? ''} />}
          <div className="row" style={{ justifyContent: 'flex-end', padding: 14 }}>
            <Dialog.Close asChild>
              <button type="button" className="btn">
                Close
              </button>
            </Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
