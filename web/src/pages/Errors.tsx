import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';

import { TimeChart } from '../components/charts';
import { IconJourney, IconSearch } from '../components/icons';
import { Empty, Segmented, Skeleton, Sparkline, StatusPill } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtAgo, fmtClock, fmtCompact, fmtDateTime, fmtInt, fmtMs, num, rangeMs, str } from '../lib/format';
import { useSession } from '../lib/session';
import type { ErrorGroup } from '../lib/types';

export interface ErrorsSearch {
  status?: 'new' | 'spiking';
  sort?: 'count' | 'new';
  range?: string;
  q?: string;
}

const RANGE_OPTIONS = [
  { id: '1h', label: '1h' },
  { id: '24h', label: '24h' },
  { id: '7d', label: '7d' },
  { id: '30d', label: '30d' },
];

function Badge({ g }: { g: ErrorGroup }) {
  if (g.spiking) return <span className="badge spike">SPIKE</span>;
  if (g.isNew) return <span className="badge new">NEW</span>;
  return <span className="badge steady">steady</span>;
}

export function Errors({ search }: { search: ErrorsSearch }) {
  const { project, environment } = useSession();
  const navigate = useNavigate();
  const [text, setText] = useState(search.q ?? '');
  useEffect(() => setText(search.q ?? ''), [search.q]);
  const range = search.range ?? '24h';
  const [now] = useState(() => Date.now());
  const set = (patch: Partial<ErrorsSearch>) => void navigate({ to: '/errors', search: { ...search, ...patch } });

  const errors = useQuery({
    queryKey: ['errors', 'list', project?.id, environment, search.status, search.sort, range, search.q],
    enabled: !!project,
    queryFn: () =>
      api.errors(project?.id ?? '', environment, search.status, 200, {
        from: now - rangeMs(range, '24h'),
        sort: search.sort,
        q: search.q,
      }),
    refetchInterval: 30_000,
  });
  const groups = errors.data?.groups ?? [];
  const cols = 'minmax(0, 2.4fr) minmax(0, 1.3fr) 96px 80px 70px 92px 84px';

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Errors</h1>
        <span className="panel-sub">grouped by fingerprint: error type, top app frame and route</span>
      </header>

      <div className="row wrap" style={{ gap: 10 }}>
        <form
          className="search-box"
          style={{ flex: '1 1 320px' }}
          onSubmit={(e) => {
            e.preventDefault();
            set({ q: text.trim() || undefined });
          }}
        >
          <IconSearch style={{ color: 'var(--muted)' }} />
          <label htmlFor="errors-q" className="sr-only">
            Filter by error type or message
          </label>
          <input id="errors-q" value={text} onChange={(e) => setText(e.target.value)} placeholder="Filter by type or message" />
        </form>
        <Segmented
          small
          label="Status"
          value={search.status ?? 'all'}
          onChange={(v) => set({ status: v === 'all' ? undefined : v })}
          options={[
            { id: 'all', label: 'All' },
            { id: 'new', label: 'New' },
            { id: 'spiking', label: 'Spiking' },
          ]}
        />
        <Segmented
          small
          label="Sort"
          value={search.sort ?? 'recent'}
          onChange={(v) => set({ sort: v === 'recent' ? undefined : v })}
          options={[
            { id: 'recent', label: 'Last seen' },
            { id: 'count', label: 'Most frequent' },
            { id: 'new', label: 'Newest' },
          ]}
        />
        <Segmented small label="Window" value={range} onChange={(v) => set({ range: v })} options={RANGE_OPTIONS} />
      </div>

      <section aria-label="Error groups" className="panel" style={{ overflow: 'hidden' }}>
        <div className="scroll-x">
          <div style={{ minWidth: 900 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>Error</span>
              <span>Where</span>
              <span>Last 24h</span>
              <span style={{ textAlign: 'right' }}>Events</span>
              <span style={{ textAlign: 'right' }}>Users</span>
              <span>First seen</span>
              <span>Last seen</span>
            </div>
            {errors.isLoading && <Skeleton h={240} />}
            {!errors.isLoading && groups.length === 0 && (
              <Empty title={search.q || search.status ? 'No matching errors' : 'No errors'}>
                {search.q || search.status ? 'Try another filter or a longer window.' : 'Nothing failed in this window.'}
              </Empty>
            )}
            {groups.map((g) => (
              <Link
                key={`${g.fingerprint}-${g.environment}`}
                to="/errors/$fingerprint"
                params={{ fingerprint: g.fingerprint }}
                className={`table-row${g.spiking ? ' err' : ''}`}
                style={{ gridTemplateColumns: cols, padding: '10px 18px' }}
              >
                <span style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                  <span className="row" style={{ minWidth: 0 }}>
                    <span className="mono ellipsis" style={{ fontSize: 13, fontWeight: 600 }}>
                      {g.errorType || 'Error'}
                    </span>
                    <Badge g={g} />
                  </span>
                  <span className="muted ellipsis" style={{ fontSize: 12 }} title={g.message}>
                    {g.message || '—'}
                  </span>
                </span>
                <span style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0 }}>
                  <span className="mono ellipsis" style={{ fontSize: 12 }}>
                    {g.route || g.source}
                  </span>
                  <span className="mono dim ellipsis" style={{ fontSize: 11 }}>
                    {g.service}
                    {environment ? '' : ` · ${g.environment}`}
                  </span>
                </span>
                <Sparkline values={g.sparkline} width={90} height={24} color={g.spiking ? 'var(--err)' : g.isNew ? 'var(--warn)' : 'var(--muted)'} label={`${g.last24h} in the last 24 hours`} />
                <span className="cell-mono" style={{ textAlign: 'right' }}>
                  {fmtCompact(g.total)}
                </span>
                <span className="cell-mono" style={{ textAlign: 'right' }}>
                  {fmtCompact(g.users)}
                </span>
                <span className="muted" style={{ fontSize: 12 }} title={fmtDateTime(g.firstSeen)}>
                  {fmtAgo(g.firstSeen)}
                  {g.firstRelease && <span className="mono dim" style={{ display: 'block', fontSize: 11 }}>{g.firstRelease}</span>}
                </span>
                <span className="muted" style={{ fontSize: 12 }} title={fmtDateTime(g.lastSeen)}>
                  {fmtAgo(g.lastSeen)}
                </span>
              </Link>
            ))}
          </div>
        </div>
      </section>
    </main>
  );
}

export function ErrorGroupPage({ fingerprint }: { fingerprint: string }) {
  const { project, environment } = useSession();
  const [now] = useState(() => Date.now());
  const detail = useQuery({
    queryKey: ['error', project?.id, fingerprint, environment],
    enabled: !!project,
    queryFn: () => api.errorDetail(project?.id ?? '', fingerprint, environment),
    retry: false,
  });
  const releases = useQuery({
    queryKey: ['releases', project?.id, environment, '7d'],
    enabled: !!project,
    queryFn: () => api.releases(project?.id ?? '', now - 7 * 86_400_000, now, environment),
  });

  if (detail.isLoading) {
    return (
      <main className="main">
        <Skeleton h={40} w={400} />
        <Skeleton h={180} />
      </main>
    );
  }
  if (detail.error || !detail.data) {
    return (
      <main className="main">
        <Empty title="Error group not found">
          {detail.error instanceof ApiError && detail.error.status !== 404 ? detail.error.message : 'It may be older than the retention period.'}
          <Link to="/errors">Back to errors</Link>
        </Empty>
      </main>
    );
  }
  const { group: g, samples, hourly, stack } = detail.data;
  const frames = stack
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
  // A dense 7-day grid of hours, so gaps show as gaps.
  const hour = 3_600_000;
  const lastHour = Math.floor(now / hour) * hour;
  const times = Array.from({ length: 168 }, (_, i) => lastHour - (167 - i) * hour);
  const byHour = new Map(hourly.map((h) => [h[0], h[1]]));
  const counts = times.map((t) => byHour.get(t) ?? 0);
  const groupReleases = (releases.data ?? []).filter((r) => r.service === g.service);
  const users = new Set(samples.map((s) => str(s.user_id)).filter(Boolean));
  const firstUser = [...users][0];
  const filterSql = `error_fingerprint = ${g.fingerprint}`;
  const tiles = [
    { label: 'events (all time)', value: fmtInt(g.total) },
    { label: 'last 24 hours', value: fmtInt(g.last24h) },
    { label: 'last 15 minutes', value: fmtInt(g.last15m), color: g.spiking ? 'var(--err-text)' : undefined },
    { label: 'users affected', value: fmtInt(g.users) },
  ];

  return (
    <main className="main" style={{ gap: 16 }}>
      <Link to="/errors" style={{ fontSize: 13 }}>
        ← Errors
      </Link>
      <header style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div className="row wrap" style={{ gap: 12 }}>
          <h1 className="mono" style={{ margin: 0, fontSize: 22, fontWeight: 600, color: 'var(--err-text)', overflowWrap: 'anywhere' }}>
            {g.errorType || 'Error'}
          </h1>
          <Badge g={g} />
        </div>
        <p style={{ margin: 0, fontSize: 14, color: 'var(--text-3)', overflowWrap: 'anywhere' }}>{g.message}</p>
        <div className="row wrap" style={{ gap: 6 }}>
          {g.route && <span className="chip">{g.route}</span>}
          <span className="chip">{g.service}</span>
          <span className="chip">{g.environment}</span>
          {g.source !== 'request' && <span className="chip">{g.source}</span>}
          <span className="chip" title={fmtDateTime(g.firstSeen)}>
            first seen {fmtAgo(g.firstSeen)}
          </span>
          {g.firstRelease && <span className="chip accent">since release {g.firstRelease}</span>}
          <span className="chip" title={fmtDateTime(g.lastSeen)}>
            last seen {fmtAgo(g.lastSeen)}
          </span>
        </div>
        <div className="row wrap">
          {g.source === 'request' && (
            <Link to="/explore" search={{ q: filterSql, mode: 'sql', range: '7d' }} className="btn btn-primary">
              Open in Explore
            </Link>
          )}
          {g.sampleRequestId && (
            <Link to="/requests/$requestId" params={{ requestId: g.sampleRequestId }} className="btn">
              Latest occurrence
            </Link>
          )}
          {firstUser && (
            <Link to="/journeys" search={{ q: `user_id:${firstUser}`, came: g.sampleRequestId || undefined }} className="btn">
              <IconJourney /> Journey of an affected user
            </Link>
          )}
        </div>
      </header>

      <section aria-label="Counts" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(160px, 1fr))', gap: 10 }}>
        {tiles.map((t) => (
          <div key={t.label} className="panel" style={{ padding: '14px 16px', display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span className="mono" style={{ fontSize: 24, fontWeight: 500, color: t.color }}>
              {t.value}
            </span>
            <span className="muted" style={{ fontSize: 12 }}>
              {t.label}
            </span>
          </div>
        ))}
      </section>

      <section aria-label="Occurrences" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div className="row">
          <h2 className="panel-title">Occurrences per hour</h2>
          <span className="panel-sub">last 7 days · dashed lines are {g.service} releases</span>
        </div>
        {hourly.length ? (
          <TimeChart
            times={times}
            step={3_600_000}
            label="Occurrences per hour"
            series={[{ name: 'events', values: counts, color: 'var(--err)', kind: 'bar' }]}
            releases={groupReleases}
          />
        ) : (
          <Empty title="No occurrences in the last 7 days" />
        )}
      </section>

      {frames.length > 0 && (
        <section aria-label="Stack trace" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          <h2 className="panel-title">Stack trace</h2>
          <pre className="code" style={{ margin: 0, padding: '12px 14px', background: 'var(--sidebar)', borderRadius: 8, fontSize: 12, lineHeight: 1.7, color: 'var(--muted)', overflowX: 'auto' }}>
            {frames.join('\n')}
          </pre>
        </section>
      )}

      <section aria-label="Recent occurrences" className="panel" style={{ overflow: 'hidden' }}>
        <div className="row" style={{ padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
          <h2 className="panel-title">Recent occurrences</h2>
          <span className="panel-sub">{samples.length} newest</span>
        </div>
        <div className="scroll-x">
          <div style={{ minWidth: 760 }}>
            {samples.map((s, i) => {
              const id = str(s.request_id);
              const status = num(s.status);
              const cols = '90px 64px minmax(0, 1fr) 110px 90px 80px';
              const content = (
                <>
                  <span className="cell-mono muted">{fmtClock(num(s.timestamp), false)}</span>
                  <span>{s.method !== undefined ? <StatusPill status={status} /> : <span className="badge steady">{str(s.kind)}</span>}</span>
                  <span className="cell-mono ellipsis">{s.method !== undefined ? `${str(s.method)} ${str(s.path)}` : `${str(s.name)}${num(s.attempt) ? ` · attempt ${num(s.attempt)}` : ''}`}</span>
                  <span className="cell-mono ellipsis muted">{str(s.user_id) || '—'}</span>
                  <span className="cell-mono ellipsis muted">{str(s.release) || '—'}</span>
                  <span className="cell-mono" style={{ textAlign: 'right' }}>
                    {fmtMs(num(s.duration_ms))}
                  </span>
                </>
              );
              return id && s.method !== undefined ? (
                <Link key={`${id}-${i}`} to="/requests/$requestId" params={{ requestId: id }} className="table-row" style={{ gridTemplateColumns: cols }}>
                  {content}
                </Link>
              ) : (
                <div key={`${id}-${i}`} className="table-row" style={{ gridTemplateColumns: cols, cursor: 'default' }}>
                  {content}
                </div>
              );
            })}
          </div>
        </div>
      </section>
    </main>
  );
}
