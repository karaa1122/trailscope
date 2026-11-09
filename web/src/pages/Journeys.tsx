import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';

import { IconSearch } from '../components/icons';
import { Empty, Segmented, Skeleton, Toast, copyText, useToast } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtClock, fmtDuration, fmtHm, fmtInt, fmtMs, rangeMs } from '../lib/format';
import { useSession } from '../lib/session';
import type { Journey, JourneyItem, Release } from '../lib/types';

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
          Search for a correlation key such as <code className="mono">account_id:acc_117</code>, a user ID, a request ID or a trace ID. Trailscope
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

/** The time span the timeline draws, with a little padding. */
function timeWindow(s: Journey['summary'], now: number) {
  const end = s.inProgress ? Math.max(now, s.last) : s.last;
  const pad = Math.round(Math.max(1000, (end - s.first) * 0.04));
  const from = s.first - pad;
  const to = end + (s.inProgress ? pad * 2 : pad);
  const span = Math.max(1, to - from);
  return { from, to, span, pos: (t: number) => `${(((t - from) / span) * 100).toFixed(2)}%` };
}

interface Problem {
  start: number;
  end: number;
  count: number;
  total: number;
  /** Most common failing step, e.g. "GET /projects/:id/tasks → 500". */
  title: string;
  errorType: string;
  /** The service where it failed (its lane). */
  lane: string;
  /** The first failing request, to open in the inspector. */
  firstRequestId?: string;
  before?: Release;
  after?: Release;
}

/**
 * The biggest burst of one kind of failure (same step, gaps under 15 minutes),
 * and the releases around it. Grouping by step keeps unrelated errors in other
 * services from blurring the story.
 */
function findProblem(items: JourneyItem[], releases: Release[]): Problem | null {
  const errs = items.filter((i) => i.status === 'error');
  if (!errs.length) return null;
  const weight = (r: JourneyItem[]) => r.reduce((n, i) => n + Math.max(1, i.similar), 0);
  const byStep = new Map<string, JourneyItem[]>();
  for (const e of errs) byStep.set(`${e.lane}|${e.title}`, [...(byStep.get(`${e.lane}|${e.title}`) ?? []), e]);
  const runs: JourneyItem[][] = [];
  for (const list of byStep.values()) {
    let run: JourneyItem[] = [];
    for (const e of list) {
      if (run.length && e.t - run[run.length - 1]!.until > 15 * 60_000) {
        runs.push(run);
        run = [];
      }
      run.push(e);
    }
    runs.push(run);
  }
  const best = runs.reduce((a, b) => (weight(b) > weight(a) ? b : a));
  const top = (key: (i: JourneyItem) => string) => {
    const counts = new Map<string, number>();
    for (const i of best) counts.set(key(i), (counts.get(key(i)) ?? 0) + Math.max(1, i.similar));
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? '';
  };
  const start = best[0]!.t;
  const end = Math.max(...best.map((i) => i.until));
  const sorted = [...releases].sort((a, b) => a.at - b.at);
  const before = sorted.filter((r) => r.at <= start + 60_000 && r.at >= start - 30 * 60_000).pop();
  // The fix is a later, different version deployed around the time the errors stopped.
  const after = sorted.find(
    (r) => r.at > (before?.at ?? start) && r.version !== before?.version && r.at >= end - 5 * 60_000 && r.at <= end + 15 * 60_000,
  );
  return {
    start,
    end,
    count: weight(best),
    total: weight(errs),
    title: top((i) => i.title),
    errorType: top((i) => i.detail.split(':')[0] ?? ''),
    lane: top((i) => i.lane),
    firstRequestId: best.find((i) => i.kind === 'request' && i.requestId)?.requestId,
    before,
    after,
  };
}

function Story({ data, problem, came }: { data: Journey; problem: Problem | null; came?: JourneyItem }) {
  const s = data.summary;
  const who = `${title(data.key)} ${data.value}`;
  const activity = `${fmtInt(s.requests)}${data.truncated ? '+' : ''} requests and ${fmtInt(s.jobs)} background jobs across ${s.services.length} service${s.services.length === 1 ? '' : 's'}`;
  return (
    <section aria-label="What happened" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 8, borderColor: problem ? 'var(--err-line)' : undefined }}>
      <h2 className="section-title" style={{ padding: 0, margin: 0 }}>
        What happened
      </h2>
      <p style={{ margin: 0, fontSize: 15, lineHeight: 1.6 }}>
        {who} made {activity} between {fmtHm(s.first)} and {fmtHm(s.last)}.{' '}
        {!problem ? (
          <span className="ok-text">Nothing failed for them in this window.</span>
        ) : (
          <>
            <span style={{ color: 'var(--err-text)', fontWeight: 600 }}>
              {problem.count === problem.total ? `${fmtInt(problem.total)} errors` : `${fmtInt(problem.count)} of the ${fmtInt(problem.total)} errors`}
            </span>{' '}
            happened between {fmtHm(problem.start)} and {fmtHm(problem.end)}
            {problem.before && (
              <>
                , right after release <b className="mono">{problem.before.version}</b> was deployed
              </>
            )}
            .
            {problem.after && (
              <>
                {' '}
                They stopped after <b className="mono">{problem.after.version}</b> at {fmtHm(problem.after.at)}.
              </>
            )}
          </>
        )}
      </p>
      {problem && (
        <div className="row wrap" style={{ gap: 8, padding: '10px 12px', borderRadius: 8, background: 'var(--err-faint)', fontSize: 13.5 }}>
          <span style={{ fontWeight: 600 }}>Where it failed:</span>
          <span className="chip">service {problem.lane}</span>
          <span className="chip" style={{ color: 'var(--err-text)' }}>
            {problem.title}
          </span>
          {problem.errorType && (
            <span className="chip" style={{ color: 'var(--err-text)' }}>
              {problem.errorType}
            </span>
          )}
          {problem.firstRequestId && (
            <Link to="/requests/$requestId" params={{ requestId: problem.firstRequestId }} className="btn btn-sm" style={{ marginLeft: 'auto' }}>
              Open the first failure
            </Link>
          )}
        </div>
      )}
      {came && (
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          The request you came from ({fmtClock(came.t, false)}, <span className="mono">{came.title}</span>) is ringed in lime below.
        </p>
      )}
    </section>
  );
}

function JourneyView({ data, came, now, shown, onMore }: { data: Journey; came?: string; now: number; shown: number; onMore: () => void }) {
  const s = data.summary;
  const { project, environment } = useSession();
  const win = timeWindow(s, now);
  const releases = useQuery({
    queryKey: ['releases', project?.id, environment, 'journey', win.from, win.to],
    enabled: !!project,
    queryFn: () => api.releases(project?.id ?? '', win.from, win.to, environment),
    staleTime: 60_000,
  });
  const markers = useMemo(() => (releases.data ?? []).filter((r) => r.at >= win.from && r.at <= win.to), [releases.data, win.from, win.to]);
  const problem = useMemo(() => findProblem(data.items, markers), [data.items, markers]);
  const cameItem = came ? data.items.find((i) => i.kind === 'request' && i.requestId === came) : undefined;
  const hasProblems = s.errors + s.warnings > 0;
  const [onlyProblems, setOnlyProblems] = useState(hasProblems);
  const logItems = onlyProblems ? data.items.filter((i) => i.status !== 'ok' || (!!came && i.requestId === came)) : data.items;
  const last = data.items[data.items.length - 1];
  const recovered = s.errors > 0 && last?.status === 'ok';
  const elapsed = Math.max(0, s.last - s.first);
  const tiles = [
    { value: fmtDuration(elapsed), label: s.inProgress ? 'since first activity' : 'first to last activity' },
    { value: `${fmtInt(s.requests)}${data.truncated ? '+' : ''}`, label: 'API requests' },
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
            <span className="dot" style={{ background: 'var(--accent)' }} /> Active now
          </span>
        ) : (
          <span className="chip" style={{ fontFamily: 'var(--font)', borderRadius: 999, padding: '4px 12px' }}>
            Quiet since {fmtHm(s.last)}
          </span>
        )}
        <span className="muted" style={{ fontSize: 13 }}>
          {s.errors === 0 ? 'no errors' : recovered ? 'working again now' : 'the last step failed'}
          {data.truncated && ' · very active: only the first 2,000 items are shown, pick a shorter window'}
        </span>
      </header>

      <Story data={data} problem={problem} came={cameItem} />

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

      <Swimlanes data={data} came={came} now={now} win={win} markers={markers} problem={problem} />

      <div className="row wrap" style={{ gap: 16, alignItems: 'flex-start' }}>
        <section aria-label="Event log" className="panel" style={{ flex: '999 1 520px', minWidth: 0, overflow: 'hidden' }}>
          <div className="row wrap" style={{ gap: 12, padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
            <h2 className="panel-title">Event log</h2>
            <span className="panel-sub">
              {onlyProblems ? 'errors and retries only, oldest first' : `everything, oldest first${s.collapsed > 0 ? ` · ${fmtInt(s.collapsed)} repetitive items collapsed` : ''}`}
            </span>
            {hasProblems && (
              <span style={{ marginLeft: 'auto' }}>
                <Segmented
                  small
                  label="Show"
                  value={onlyProblems ? 'problems' : 'all'}
                  onChange={(v) => setOnlyProblems(v === 'problems')}
                  options={[
                    { id: 'problems', label: `Problems (${fmtInt(s.errors + s.warnings)})` },
                    { id: 'all', label: `Everything (${fmtInt(data.items.length)})` },
                  ]}
                />
              </span>
            )}
          </div>
          <div className="scroll-x">
            <div style={{ minWidth: 640 }}>
              {logItems.slice(0, shown).map((e, i) => (
                <EventRow key={`${e.t}-${i}`} e={e} came={came} />
              ))}
              {logItems.length > shown && (
                <button type="button" className="btn btn-ghost" style={{ width: '100%', borderRadius: 0 }} onClick={onMore}>
                  Show {fmtInt(Math.min(PAGE, logItems.length - shown))} more
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

function Swimlanes({
  data,
  came,
  now,
  win,
  markers,
  problem,
}: {
  data: Journey;
  came?: string;
  now: number;
  win: ReturnType<typeof timeWindow>;
  markers: Release[];
  problem: Problem | null;
}) {
  const s = data.summary;
  const { from, span, pos } = win;
  // Release labels go on two rows so close ones don't print on top of each other;
  // one too close to both rows is left unlabelled (its line stays).
  const labelled: { r: Release; row: number }[] = [];
  const lastAt = [-Infinity, -Infinity];
  for (const r of [...markers].sort((a, b) => a.at - b.at)) {
    const row = lastAt.findIndex((t) => (r.at - t) / span > 0.07);
    if (row < 0) continue;
    lastAt[row] = r.at;
    labelled.push({ r, row });
  }
  const shade = problem && { left: pos(problem.start - 30_000), width: `max(6px, calc(${pos(problem.end + 30_000)} - ${pos(problem.start - 30_000)}))` };
  const byLane = useMemo(() => {
    const m = new Map<string, JourneyItem[]>();
    for (const lane of data.lanes) m.set(lane, []);
    for (const i of data.items) m.get(i.lane)?.push(i);
    // Problems draw last so they sit on top of neighbouring dots.
    const rank = (i: JourneyItem) => (i.requestId && i.requestId === came ? 3 : i.status === 'error' ? 2 : i.status === 'warn' ? 1 : 0);
    for (const list of m.values()) list.sort((a, b) => rank(a) - rank(b));
    return m;
  }, [data, came]);
  return (
    <section aria-label="Swimlane timeline" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <div className="row wrap" style={{ gap: 12 }}>
        <h2 className="panel-title">Timeline</h2>
        <span className="panel-sub">each row is a service · each dot is one request, job or log line · time runs left to right · hover a dot for details</span>
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
          {problem && shade && (
            <div style={{ display: 'grid', gridTemplateColumns: '150px minmax(0, 1fr)', gap: 14 }}>
              <span />
              <span style={{ position: 'relative', height: 22, display: 'block' }}>
                <span
                  className="mono"
                  style={{ position: 'absolute', left: pos(problem.start), bottom: 2, fontSize: 11.5, fontWeight: 600, color: 'var(--err-text)', whiteSpace: 'nowrap' }}
                >
                  ▼ {fmtInt(problem.count)} errors here
                </span>
              </span>
            </div>
          )}
          {data.lanes.map((lane) => (
            <div key={lane} style={{ display: 'grid', gridTemplateColumns: '150px minmax(0, 1fr)', gap: 14, alignItems: 'center', minHeight: 40, borderBottom: '1px solid var(--line-soft)' }}>
              <span className="mono ellipsis" style={{ fontSize: 12, color: 'var(--text-2)' }} title={lane}>
                {lane}
              </span>
              <span style={{ position: 'relative', height: 40, display: 'block' }}>
                <span style={{ position: 'absolute', left: 0, right: 0, top: 19, height: 1, background: 'var(--selected)' }} />
                {shade && <span style={{ position: 'absolute', top: 0, bottom: 0, left: shade.left, width: shade.width, background: 'var(--err-soft)' }} />}
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
                  // Problems are big and bright; normal traffic is small and faded so they stand out.
                  const isCame = !!i.requestId && i.requestId === came;
                  const size = isCame ? 14 : i.status === 'error' ? 12 : i.status === 'warn' ? 9 : i.similar > 1 ? 4 : 6;
                  const color = i.similar > 1 && i.status === 'ok' ? 'var(--span)' : (COLORS[i.status] ?? 'var(--ok)');
                  const ring = isCame ? '0 0 0 2px var(--panel), 0 0 0 4px var(--accent)' : i.status === 'error' ? '0 0 0 2px var(--panel)' : undefined;
                  const opacity = isCame || i.status !== 'ok' ? 1 : 0.4;
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
                        opacity,
                      }}
                    />
                  );
                })}
              </span>
            </div>
          ))}
          <div style={{ display: 'grid', gridTemplateColumns: '150px minmax(0, 1fr)', gap: 14, paddingTop: 8 }}>
            <span />
            <span className="mono dim" style={{ position: 'relative', height: labelled.some((l) => l.row === 1) ? 47 : markers.length ? 32 : 16, display: 'block', fontSize: 11 }}>
              {[0, 1 / 3, 2 / 3].map((f) => (
                <span key={f} style={{ position: 'absolute', left: `${f * 100}%`, transform: f ? 'translateX(-50%)' : undefined }}>
                  {span > 36 * 3_600_000 ? `${new Date(from + span * f).getMonth() + 1}/${new Date(from + span * f).getDate()} ` : ''}
                  {span < 10 * 60_000 ? fmtClock(from + span * f, false) : fmtHm(from + span * f)}
                </span>
              ))}
              {labelled.map(({ r, row }) => (
                <span key={r.id} title={`${r.service} ${r.version} · ${fmtClock(r.at, false)}`} style={{ position: 'absolute', left: pos(r.at), transform: 'translateX(-50%)', color: 'var(--accent)', whiteSpace: 'nowrap', top: 16 + row * 15 }}>
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
          {e.requestId && e.kind !== 'request' && <span className="badge steady">opens the request that started it</span>}
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
  // Requests open in the inspector; jobs and log events open the request that started them.
  if (e.requestId) {
    return (
      <Link
        to="/requests/$requestId"
        params={{ requestId: e.requestId }}
        className="table-row"
        title={e.kind === 'request' ? 'Open this request' : 'Open the request that started this'}
        style={{ ...style, minHeight: 0 }}
      >
        {body}
      </Link>
    );
  }
  return <div style={style}>{body}</div>;
}
