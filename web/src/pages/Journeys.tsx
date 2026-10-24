import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';

import { IconSearch } from '../components/icons';
import { Empty, Segmented, Skeleton, Toast, copyText, useToast } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtClock, fmtDuration, fmtHm, fmtInt, fmtMs, rangeMs } from '../lib/format';
import { useSession } from '../lib/session';
import type { Journey, JourneyItem } from '../lib/types';

export interface JourneySearch {
  q?: string;
  /** The request the user came from (highlighted as "you came from here"). */
  came?: string;
  range?: string;
}

const COLORS: Record<string, string> = { ok: 'var(--ok)', warn: 'var(--warn)', error: 'var(--err)' };
const RANGE_OPTIONS = [
  { id: '1h', label: '1h' },
  { id: '6h', label: '6h' },
  { id: '24h', label: '24h' },
  { id: '7d', label: '7d' },
];
const PAGE = 300;

/** `key:value`, or a bare ID whose kind is guessed from its shape. */
export function parseJourneyQuery(q: string): { key: string; value: string } | null {
  const text = q.trim();
  if (!text) return null;
  const m = /^([A-Za-z_][\w.-]{0,127})\s*[:=]\s*(.+)$/.exec(text);
  if (m) return { key: m[1]!, value: m[2]!.trim().replace(/^['"]|['"]$/g, '') };
  if (/^[0-9a-f]{32}$/i.test(text)) return { key: 'trace_id', value: text };
  if (/^req_/i.test(text)) return { key: 'request_id', value: text };
  return { key: 'user_id', value: text };
}

function title(key: string): string {
  const base = key.replace(/_id$/, '').replace(/[_.-]+/g, ' ');
  return base.charAt(0).toUpperCase() + base.slice(1);
}

export function Journeys({ search }: { search: JourneySearch }) {
  const { project, environment } = useSession();
  const navigate = useNavigate();
  const [text, setText] = useState(search.q ?? '');
  const [toast, setToast] = useToast();
  const [shown, setShown] = useState(PAGE);
  useEffect(() => setText(search.q ?? ''), [search.q]);
  const parsed = parseJourneyQuery(search.q ?? '');
  const range = search.range ?? '24h';
  const [now] = useState(() => Date.now());

  const journey = useQuery({
    queryKey: ['journey', project?.id, environment, parsed?.key, parsed?.value, range],
    enabled: !!project && !!parsed,
    queryFn: () => api.journey(project?.id ?? '', parsed?.key ?? '', parsed?.value ?? '', environment, now - rangeMs(range, '24h')),
    refetchInterval: (q) => (q.state.data?.summary.inProgress ? 10_000 : false),
    retry: false,
  });

  const submit = () => void navigate({ to: '/journeys', search: { q: text.trim() || undefined, range: search.range } });
  const data = journey.data;

  const exportJson = () => {
    if (!data) return;
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `journey-${data.key}-${data.value}.json`.replace(/[^\w.-]+/g, '_');
    a.click();
    URL.revokeObjectURL(a.href);
  };

  return (
    <main className="main" style={{ gap: 18 }}>
      <div className="row wrap" style={{ gap: 10 }}>
        <form
          className="search-box"
          style={{ flex: '1 1 380px' }}
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <IconSearch style={{ color: 'var(--muted)' }} />
          <label htmlFor="journey-q" className="sr-only">
            Find a journey by correlation key, user, request or trace ID
          </label>
          <input
            id="journey-q"
            className="mono"
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="account_id:acc_117"
            autoComplete="off"
            spellCheck={false}
            style={{ flex: 1, minWidth: 0, height: 34, background: 'transparent', border: 0, outline: 'none', color: 'var(--text)', fontSize: 13 }}
          />
          <span className="muted" style={{ fontSize: 12 }}>
            key:value · user · request · trace ID
          </span>
        </form>
        <Segmented
          small
          label="Window"
          options={RANGE_OPTIONS}
          value={range}
          onChange={(r) => void navigate({ to: '/journeys', search: { ...search, range: r } })}
        />
        <button type="button" className="btn" disabled={!data} onClick={() => void copyText(window.location.href).then((ok) => setToast(ok ? 'Link copied' : 'Copy failed'))}>
          Share journey
        </button>
        <button type="button" className="btn" disabled={!data} onClick={exportJson}>
          Export JSON
        </button>
      </div>

      {!parsed && (
        <Empty title="Follow one thing across every service">
          Search for a correlation key such as <code className="mono">account_id:acc_117</code>, a user ID, a request ID or a trace ID. Logbox
          lines up every request, job and event that carries it.
        </Empty>
      )}
      {parsed && journey.isLoading && (
        <>
          <Skeleton h={36} w={360} />
          <Skeleton h={80} />
          <Skeleton h={240} />
        </>
      )}
      {journey.error && (
        <Empty title="Could not load this journey">{journey.error instanceof ApiError ? journey.error.message : 'Try again.'}</Empty>
      )}
      {data && data.items.length === 0 && (
        <Empty title="Nothing found">
          No requests or events carry <code className="mono">{data.key} = {data.value}</code> in this window. Try a longer window or another environment.
        </Empty>
      )}
      {data && data.items.length > 0 && <JourneyView data={data} came={search.came} now={now} shown={shown} onMore={() => setShown((s) => s + PAGE)} />}
      <Toast msg={toast} />
    </main>
  );
}

function JourneyView({ data, came, now, shown, onMore }: { data: Journey; came?: string; now: number; shown: number; onMore: () => void }) {
  const s = data.summary;
  const last = data.items[data.items.length - 1];
  const recovered = s.errors > 0 && last?.status === 'ok';
  const elapsed = Math.max(0, s.last - s.first);
  const tiles = [
    { value: fmtDuration(elapsed), label: s.inProgress ? 'since first activity' : 'first to last activity' },
    { value: fmtInt(s.requests), label: 'API requests' },
    { value: fmtInt(s.jobs), label: 'background jobs' },
    {
      value: fmtInt(s.errors),
      label: s.errors === 0 ? 'errors' : recovered ? 'errors, then recovered' : 'errors',
      color: s.errors ? (recovered ? 'var(--warn)' : 'var(--err-text)') : undefined,
    },
    { value: fmtInt(s.services.length), label: s.services.length === 1 ? 'service touched' : 'services touched' },
  ];
  const sections = Object.entries(data.context);
  return (
    <>
      <header className="row wrap" style={{ gap: '12px 14px' }}>
        <h1 style={{ margin: 0, fontSize: 26, fontWeight: 600, letterSpacing: '-0.02em' }}>
          {title(data.key)} <span className="mono" style={{ fontWeight: 500 }}>{data.value}</span>
        </h1>
        {s.inProgress ? (
          <span className="chip accent" style={{ fontFamily: 'var(--font)', fontWeight: 600, borderRadius: 999, padding: '4px 12px' }}>
            <span className="dot" style={{ background: 'var(--accent)' }} /> In progress
          </span>
        ) : (
          <span className="chip" style={{ fontFamily: 'var(--font)', borderRadius: 999, padding: '4px 12px' }}>
            Quiet since {fmtHm(s.last)}
          </span>
        )}
        <span className="muted" style={{ fontSize: 13 }}>
          {s.errors === 0 ? 'no errors' : recovered ? `recovered after ${s.errors} error${s.errors > 1 ? 's' : ''}` : `${s.errors} error${s.errors > 1 ? 's' : ''}, last step failed`}
          {data.truncated && ' · showing the first 2,000 items'}
        </span>
      </header>

      <section aria-label="Summary" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(170px, 1fr))', gap: 10 }}>
        {tiles.map((t) => (
          <div key={t.label} className="panel" style={{ padding: '14px 16px', display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span className="mono" style={{ fontSize: 24, fontWeight: 500, letterSpacing: '-0.02em', color: t.color }}>
              {t.value}
            </span>
            <span className="muted" style={{ fontSize: 12 }}>
              {t.label}
            </span>
          </div>
        ))}
      </section>

      <Swimlanes data={data} came={came} now={now} />

      <div className="row wrap" style={{ gap: 16, alignItems: 'flex-start' }}>
        <section aria-label="Event log" className="panel" style={{ flex: '999 1 520px', minWidth: 0, overflow: 'hidden' }}>
          <div className="row" style={{ gap: 12, padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
            <h2 className="panel-title">Event log</h2>
            <span className="panel-sub">
              {s.collapsed > 0 ? `${fmtInt(s.collapsed)} repetitive items collapsed` : `${fmtInt(data.items.length)} items`}
            </span>
          </div>
          <div className="scroll-x">
            <div style={{ minWidth: 640 }}>
              {data.items.slice(0, shown).map((e, i) => (
                <EventRow key={`${e.t}-${i}`} e={e} came={came} />
              ))}
              {data.items.length > shown && (
                <button type="button" className="btn btn-ghost" style={{ width: '100%', borderRadius: 0 }} onClick={onMore}>
                  Show {fmtInt(Math.min(PAGE, data.items.length - shown))} more
                </button>
              )}
            </div>
          </div>
        </section>

        <aside aria-label="Context" style={{ flex: '1 1 300px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12 }}>
          {sections.map(([name, values]) => (
            <section key={name} aria-label={name} className="panel" style={{ padding: '6px 0' }}>
              <h2 className="section-title" style={{ padding: '10px 16px 6px', margin: 0 }}>
                {name === 'who' ? 'User and client' : name}
              </h2>
              {Object.entries(values).map(([k, v]) => (
                <div key={k} className="kv">
                  <span>{k}</span>
                  <span>{v}</span>
                </div>
              ))}
            </section>
          ))}
          {(data.errors.length > 0 || data.traces.length > 0) && (
            <section aria-label="Related" className="panel" style={{ padding: '14px 16px', display: 'flex', flexDirection: 'column', gap: 10 }}>
              <h2 className="section-title" style={{ padding: 0, margin: 0 }}>
                Related
              </h2>
              {data.errors.slice(0, 6).map((e) => (
                <Link key={e.fingerprint} to="/errors/$fingerprint" params={{ fingerprint: e.fingerprint }} className="row" style={{ fontSize: 13, color: 'var(--text)' }}>
                  <span className="dot" style={{ width: 8, height: 8, background: COLORS[e.status] ?? 'var(--err)' }} />
                  <span className="mono ellipsis">{e.errorType || 'Error'}</span>
                  <span className="mono muted" style={{ marginLeft: 'auto', fontSize: 12 }}>
                    {fmtInt(e.count)}×
                  </span>
                </Link>
              ))}
              {data.traces.slice(0, 5).map((t) => (
                <Link key={t.traceId} to="/journeys" search={{ q: `trace_id:${t.traceId}` }} className="row" style={{ fontSize: 13, color: 'var(--text)' }}>
                  <span className="dot" style={{ width: 8, height: 8, background: 'var(--ok)' }} />
                  <span className="mono ellipsis">
                    trace {t.traceId.slice(0, 8)}…{t.traceId.slice(-4)}
                  </span>
                  <span className="muted" style={{ marginLeft: 'auto', fontSize: 12, whiteSpace: 'nowrap' }}>
                    {t.spans} spans
                  </span>
                </Link>
              ))}
            </section>
          )}
        </aside>
      </div>
    </>
  );
}

function Swimlanes({ data, came, now }: { data: Journey; came?: string; now: number }) {
  const s = data.summary;
  const end = s.inProgress ? Math.max(now, s.last) : s.last;
  const pad = Math.round(Math.max(1000, (end - s.first) * 0.04));
  const from = s.first - pad;
  const to = end + (s.inProgress ? pad * 2 : pad);
  const span = Math.max(1, to - from);
  const pos = (t: number) => `${(((t - from) / span) * 100).toFixed(2)}%`;
  const byLane = useMemo(() => {
    const m = new Map<string, JourneyItem[]>();
    for (const lane of data.lanes) m.set(lane, []);
    for (const i of data.items) m.get(i.lane)?.push(i);
    // Problems draw last so they sit on top of neighbouring dots.
    const rank = (i: JourneyItem) => (i.requestId && i.requestId === came ? 3 : i.status === 'error' ? 2 : i.status === 'warn' ? 1 : 0);
    for (const list of m.values()) list.sort((a, b) => rank(a) - rank(b));
    return m;
  }, [data, came]);
  const { project, environment } = useSession();
  const releases = useQuery({
    queryKey: ['releases', project?.id, environment, 'journey', from, to],
    enabled: !!project,
    queryFn: () => api.releases(project?.id ?? '', from, to, environment),
    staleTime: 60_000,
  });
  const markers = (releases.data ?? []).filter((r) => r.at >= from && r.at <= to);
  return (
    <section aria-label="Swimlane timeline" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div className="row wrap" style={{ gap: 12 }}>
        <h2 className="panel-title">Across services</h2>
        <span className="panel-sub">every request, job and event carrying this value, one lane per service</span>
        <span className="row muted" style={{ marginLeft: 'auto', gap: 14, fontSize: 12 }}>
          {[
            ['ok', 'ok'],
            ['warn', 'retry / warn'],
            ['error', 'error'],
          ].map(([k, l]) => (
            <span key={k} className="row" style={{ gap: 6 }}>
              <span className="dot" style={{ width: 9, height: 9, background: COLORS[k!] }} />
              {l}
            </span>
          ))}
        </span>
      </div>
      <div className="scroll-x">
        <div style={{ minWidth: 760, display: 'flex', flexDirection: 'column' }}>
          {data.lanes.map((lane) => (
            <div key={lane} style={{ display: 'grid', gridTemplateColumns: '150px minmax(0, 1fr)', gap: 14, alignItems: 'center', minHeight: 40, borderBottom: '1px solid var(--line-soft)' }}>
              <span className="mono ellipsis" style={{ fontSize: 12, color: 'var(--text-2)' }} title={lane}>
                {lane}
              </span>
              <span style={{ position: 'relative', height: 40, display: 'block' }}>
                <span style={{ position: 'absolute', left: 0, right: 0, top: 19, height: 1, background: 'var(--selected)' }} />
                {markers.map((r) => (
                  <span key={r.id} style={{ position: 'absolute', top: 0, bottom: 0, left: pos(r.at), borderLeft: '1px dashed var(--accent-line)' }} />
                ))}
                {s.inProgress && (
                  <>
                    <span style={{ position: 'absolute', top: 0, bottom: 0, left: pos(now), right: 0, background: 'repeating-linear-gradient(135deg, rgba(127,127,127,0.06) 0 6px, transparent 6px 12px)' }} />
                    <span style={{ position: 'absolute', top: 0, bottom: 0, left: pos(now), width: 1, background: 'var(--accent)' }} />
                  </>
                )}
                {(byLane.get(lane) ?? []).map((i, n) => {
                  const big = i.status !== 'ok' || i.requestId === came;
                  const size = i.similar > 1 ? 6 : big ? 12 : 9;
                  const color = i.similar > 1 ? 'var(--span)' : (COLORS[i.status] ?? 'var(--ok)');
                  const ring = i.requestId && i.requestId === came ? '0 0 0 2px var(--panel), 0 0 0 4px var(--accent)' : big ? '0 0 0 2px var(--panel)' : undefined;
                  const tip = `${fmtClock(i.t, false)} ${i.title}${i.similar > 1 ? ` (${i.similar}×)` : ''}${i.detail ? `\n${i.detail}` : ''}`;
                  return (
                    <span
                      key={`${i.t}-${n}`}
                      title={tip}
                      style={{
                        position: 'absolute',
                        top: '50%',
                        left: pos(i.t),
                        width: i.similar > 1 ? `max(6px, calc(${pos(i.until)} - ${pos(i.t)}))` : size,
                        height: size,
                        transform: i.similar > 1 ? 'translateY(-50%)' : 'translate(-50%, -50%)',
                        borderRadius: 999,
                        background: color,
                        boxShadow: ring,
                      }}
                    />
                  );
                })}
              </span>
            </div>
          ))}
          <div style={{ display: 'grid', gridTemplateColumns: '150px minmax(0, 1fr)', gap: 14, paddingTop: 8 }}>
            <span />
            <span className="mono dim" style={{ position: 'relative', height: markers.length ? 32 : 16, display: 'block', fontSize: 11 }}>
              {[0, 1 / 3, 2 / 3].map((f) => (
                <span key={f} style={{ position: 'absolute', left: `${f * 100}%`, transform: f ? 'translateX(-50%)' : undefined }}>
                  {span > 36 * 3_600_000 ? `${new Date(from + span * f).getMonth() + 1}/${new Date(from + span * f).getDate()} ` : ''}
                  {span < 10 * 60_000 ? fmtClock(from + span * f, false) : fmtHm(from + span * f)}
                </span>
              ))}
              {markers.map((r) => (
                <span key={r.id} title={`${r.service} ${r.version} · ${fmtClock(r.at, false)}`} style={{ position: 'absolute', left: pos(r.at), transform: 'translateX(-50%)', color: 'var(--accent)', whiteSpace: 'nowrap', top: 16 }}>
                  {r.version}
                </span>
              ))}
              {s.inProgress && (
                <span style={{ position: 'absolute', left: pos(now), transform: 'translateX(-50%)', color: 'var(--accent)', whiteSpace: 'nowrap' }}>
                  now {fmtClock(now, false)}
                </span>
              )}
            </span>
          </div>
        </div>
      </div>
    </section>
  );
}

function EventRow({ e, came }: { e: JourneyItem; came?: string }) {
  const inspected = !!came && e.requestId === came && e.kind === 'request';
  const color = COLORS[e.status] ?? 'var(--ok)';
  const titleColor = e.status === 'error' ? 'var(--err-text)' : e.status === 'warn' ? 'var(--warn)' : 'var(--text)';
  const body = (
    <>
      <span className="mono muted" style={{ fontSize: 12, paddingTop: 2 }}>
        {fmtClock(e.t, false)}
      </span>
      <span style={{ display: 'flex', justifyContent: 'center', paddingTop: 5 }}>
        <span className="dot" style={{ width: 10, height: 10, background: color }} />
      </span>
      <span className="mono ellipsis" style={{ justifySelf: 'start', maxWidth: '100%', fontSize: 11, color: 'var(--text-2)', border: '1px solid var(--line-strong)', borderRadius: 5, padding: '2px 6px' }}>
        {e.lane}
      </span>
      <span style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 }}>
        <span className="row wrap">
          <span className="mono" style={{ fontSize: 13, color: titleColor, overflowWrap: 'anywhere' }}>
            {e.title}
          </span>
          {e.similar > 1 && <span className="badge steady">{fmtInt(e.similar)} similar until {fmtClock(e.until, false)}</span>}
          {inspected && (
            <span className="badge new" style={{ color: 'var(--accent-ink)' }}>
              YOU CAME FROM HERE
            </span>
          )}
        </span>
        {e.detail && (
          <span className="muted" style={{ fontSize: 12.5, overflowWrap: 'anywhere' }}>
            {e.detail}
          </span>
        )}
      </span>
      <span className="mono" style={{ fontSize: 12, textAlign: 'right', color: 'var(--text-2)', paddingTop: 2 }}>
        {e.durationMs > 0 ? fmtMs(e.durationMs) : '—'}
      </span>
    </>
  );
  const style = {
    display: 'grid',
    gridTemplateColumns: '74px 22px 140px minmax(0, 1fr) 80px',
    gap: '0 12px',
    alignItems: 'start',
    padding: '12px 18px',
    borderBottom: '1px solid var(--line-soft)',
    color: 'var(--text)',
    background: inspected ? 'var(--accent-soft)' : undefined,
  } as const;
  if (e.requestId && e.kind === 'request') {
    return (
      <Link to="/requests/$requestId" params={{ requestId: e.requestId }} className="table-row" style={{ ...style, minHeight: 0 }}>
        {body}
      </Link>
    );
  }
  return <div style={style}>{body}</div>;
}
