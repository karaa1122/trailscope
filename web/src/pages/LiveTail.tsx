import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useMemo, useRef, useState } from 'react';

import { Heatmap, type HeatMetric } from '../components/charts';
import { IconPlay } from '../components/icons';
import { QueryBar } from '../components/QueryBar';
import { TimeRange } from '../components/TimeRange';
import { Empty, Segmented, Skeleton, Sparkline, StatusPill } from '../components/ui';
import { api } from '../lib/api';
import { fmtAgo, fmtClock, fmtCompact, fmtInt, fmtMs, fmtPct, rangeMs } from '../lib/format';
import { useHotkeys } from '../lib/hotkeys';
import { useLiveTail } from '../lib/live';
import { EMPTY_QUERY, addCondition, fromSearch, toSearch, type QuerySearch, type QueryValue } from '../lib/query';
import { useProject, useSession } from '../lib/session';
import type { LiveRequest, Overview, Tile } from '../lib/types';

export interface LiveSearch extends QuerySearch {
  window?: string;
}

/** The filter text the live stream understands (SQL or search). */
function liveFilter(v: QueryValue): { text: string; mode: 'sql' | 'search' } {
  if (v.mode === 'search') return { text: v.text, mode: 'search' };
  if (v.mode === 'sql') return { text: v.text, mode: 'sql' };
  const sql = v.chips.reduce<QueryValue>((acc, c) => addCondition(acc, c), { ...EMPTY_QUERY, mode: 'sql' });
  return { text: sql.text, mode: 'sql' };
}

type StreamFilter = 'all' | 'errors' | '5xx' | 'slow';

export function LiveTail({ search }: { search: LiveSearch }) {
  const project = useProject();
  const { environment } = useSession();
  const navigate = useNavigate();
  const value = useMemo(() => fromSearch(search, 'sql'), [search]);
  const windowId = search.window ?? '30m';
  const minutes = Math.round(rangeMs(windowId, '30m') / 60_000);
  const [paused, setPaused] = useState(false);
  const [metric, setMetric] = useState<HeatMetric>('p95');
  const [filter, setFilter] = useState<StreamFilter>('all');
  const lf = liveFilter(value);
  const live = useLiveTail(project.id, lf.text, lf.mode, environment, paused);
  const slowMs = project.settings.slowMs;

  const setSearch = (patch: Partial<LiveSearch>) =>
    void navigate({ to: '/live', search: { ...search, ...patch } });

  const overview = useQuery({
    queryKey: ['overview', project.id, environment, minutes],
    queryFn: () => api.overview(project.id, environment, minutes),
    refetchInterval: 15_000,
  });
  const heat = useQuery({
    queryKey: ['heatmap', project.id, environment, minutes],
    queryFn: () => api.heatmap(project.id, environment, minutes, 8),
    refetchInterval: 30_000,
  });
  const now = Date.now();
  const releases = useQuery({
    queryKey: ['releases', project.id, environment, minutes],
    queryFn: () => api.releases(project.id, now - minutes * 60_000, now + 60_000, environment),
    refetchInterval: 60_000,
  });
  const errors = useQuery({
    queryKey: ['errors', 'live', project.id, environment],
    queryFn: () => api.errors(project.id, environment, undefined, 20),
    refetchInterval: 30_000,
  });

  const keep = (r: LiveRequest, f: StreamFilter) =>
    f === 'errors' ? r.status >= 400 : f === '5xx' ? r.status >= 500 : f === 'slow' ? r.durationMs >= slowMs : true;
  const rows = useMemo(() => live.rows.filter((r) => keep(r, filter)), [live.rows, filter, slowMs]); // eslint-disable-line react-hooks/exhaustive-deps
  const counts = useMemo(() => {
    const c = { all: live.rows.length, errors: 0, '5xx': 0, slow: 0 };
    for (const r of live.rows) {
      if (r.status >= 400) c.errors++;
      if (r.status >= 500) c['5xx']++;
      if (r.durationMs >= slowMs) c.slow++;
    }
    return c;
  }, [live.rows, slowMs]);

  const [selected, setSelected] = useState(0);
  const open = (r: LiveRequest | undefined) => {
    if (!r) return;
    void navigate({
      to: '/requests/$requestId',
      params: { requestId: r.requestId },
      search: { ...toSearch(value), from: r.timestamp - 15 * 60_000, to: r.timestamp + 60_000 },
    });
  };
  useHotkeys({
    space: () => setPaused((p) => !p),
    j: () => setSelected((i) => Math.min(rows.length - 1, i + 1)),
    k: () => setSelected((i) => Math.max(0, i - 1)),
    enter: () => open(rows[selected]),
  });

  const spikes = (errors.data?.groups ?? [])
    .filter((g) => g.isNew || g.spiking)
    .slice(0, 3);
  const panelErrors = spikes.length ? spikes : (errors.data?.groups ?? []).slice(0, 3);

  return (
    <main className="main">
      <QueryBar
        value={value}
        live
        onRun={(v) => setSearch(toSearch(v))}
        title="Live tail"
        actions={
          <>
            <TimeRange
              value={windowId}
              options={['15m', '30m', '1h', '6h', '24h']}
              onChange={(id) => setSearch({ window: id })}
            />
            {paused ? (
              <button type="button" className="btn btn-paused" onClick={() => setPaused(false)} aria-pressed="true">
                <IconPlay /> Paused · {fmtInt(live.pendingWhilePaused)} new
              </button>
            ) : (
              <button type="button" className="btn btn-live" onClick={() => setPaused(true)} aria-pressed="false">
                <span className="dot" aria-hidden="true" />
                Live
              </button>
            )}
          </>
        }
      />

      <section aria-label="Overview" className="tiles">
        <Tiles data={overview.data} loading={overview.isLoading} />
      </section>

      <div className="row wrap" style={{ gap: 12, alignItems: 'stretch' }}>
        <section aria-label="Route heatmap" className="panel panel-pad" style={{ flex: '999 1 620px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 14 }}>
          <div className="panel-head">
            <h2 className="panel-title">Route heatmap</h2>
            <span className="panel-sub">
              {metric === 'p95' ? 'p95 latency' : metric === 'errors' ? '5xx rate' : 'volume'} · 1 cell ={' '}
              {heat.data && heat.data.times.length > 1 ? `${Math.round((heat.data.times[1]! - heat.data.times[0]!) / 60_000)} min` : '1 min'} · click a cell
              to filter
            </span>
            <span style={{ marginLeft: 'auto' }}>
              <Segmented
                small
                label="Heatmap metric"
                value={metric}
                onChange={setMetric}
                options={[
                  { id: 'p95', label: 'p95' },
                  { id: 'errors', label: 'errors' },
                  { id: 'volume', label: 'volume' },
                ]}
              />
            </span>
          </div>
          {heat.isLoading ? (
            <Skeleton h={180} />
          ) : heat.data && heat.data.routes.length ? (
            <Heatmap
              times={heat.data.times}
              routes={heat.data.routes}
              sloMs={heat.data.sloMs}
              metric={metric}
              releases={releases.data ?? []}
              onCell={(route, t) => {
                const step = heat.data && heat.data.times.length > 1 ? heat.data.times[1]! - heat.data.times[0]! : 60_000;
                void navigate({
                  to: '/explore',
                  search: { q: `route = '${route.replace(/'/g, "''")}'`, mode: 'sql', from: t, to: t + step },
                });
              }}
            />
          ) : (
            <Empty title="No traffic yet">Requests appear here a few seconds after your services send them.</Empty>
          )}
        </section>

        <section aria-label="New and spiking errors" className="panel panel-pad" style={{ flex: '1 1 320px', minWidth: 0, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div className="row">
            <h2 className="panel-title">New &amp; spiking errors</h2>
            <Link to="/errors" style={{ marginLeft: 'auto', fontSize: 12 }}>
              All errors →
            </Link>
          </div>
          {errors.isLoading && <Skeleton h={80} />}
          {!errors.isLoading && panelErrors.length === 0 && <Empty title="No errors">Nothing failed in the last 24 hours.</Empty>}
          {panelErrors.map((g) => {
            const hot = g.spiking;
            return (
              <Link
                key={g.fingerprint}
                to="/errors/$fingerprint"
                params={{ fingerprint: g.fingerprint }}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 8,
                  padding: 12,
                  borderRadius: 10,
                  color: 'var(--text)',
                  background: hot ? 'var(--err-faint)' : 'var(--sidebar)',
                  border: `1px solid ${hot ? 'var(--err-line)' : 'var(--selected)'}`,
                }}
              >
                <span className="row">
                  <span className="mono ellipsis" style={{ fontSize: 13, fontWeight: 600 }}>
                    {g.errorType || 'Error'}
                  </span>
                  <span style={{ marginLeft: 'auto' }}>
                    {g.spiking ? <span className="badge spike">SPIKE</span> : g.isNew ? <span className="badge new">NEW</span> : <span className="badge steady">steady</span>}
                  </span>
                </span>
                <span className="row muted" style={{ fontSize: 12, gap: 10 }}>
                  <span className="mono">
                    {g.service}
                    {g.source !== 'request' ? ` · ${g.source}` : ''}
                  </span>
                  <span>
                    {fmtCompact(g.last24h)}× · {fmtCompact(g.users)} users
                  </span>
                  <span style={{ marginLeft: 'auto' }}>
                    <Sparkline values={g.sparkline} width={80} height={22} color={hot ? 'var(--err)' : g.isNew ? 'var(--warn)' : 'var(--muted)'} />
                  </span>
                </span>
                {g.firstRelease && g.isNew && (
                  <span style={{ fontSize: 12, color: 'var(--text-2)' }}>
                    First seen {fmtAgo(g.firstSeen)}, release {g.firstRelease}
                  </span>
                )}
              </Link>
            );
          })}
        </section>
      </div>

      <section aria-label="Live stream" className="panel" style={{ overflow: 'hidden' }}>
        <div className="panel-bar">
          <h2 className="panel-title">Stream</h2>
          <span className="mono" style={{ fontSize: 12, color: paused ? 'var(--warn)' : live.connected ? 'var(--accent)' : 'var(--muted)' }}>
            {paused
              ? `paused · ${fmtInt(live.pendingWhilePaused)} new`
              : live.connected
                ? `● streaming · ${live.rate.toFixed(1)} req/s`
                : live.error ?? 'connecting…'}
            {live.skipped > 0 && ` · ${fmtInt(live.skipped)} skipped (rate cap)`}
          </span>
          <div role="group" aria-label="Filter requests" className="row wrap" style={{ gap: 6, marginLeft: 8 }}>
            {(
              [
                ['all', 'All'],
                ['errors', 'Errors ≥400'],
                ['5xx', '5xx'],
                ['slow', `Slow >${fmtMs(slowMs).replace(' ', '')}`],
              ] as [StreamFilter, string][]
            ).map(([id, label]) => (
              <button
                key={id}
                type="button"
                aria-pressed={filter === id}
                onClick={() => setFilter(id)}
                className="btn-sm mono"
                style={{
                  borderRadius: 8,
                  cursor: 'pointer',
                  border: `1px solid ${filter === id ? 'var(--accent)' : 'var(--line-strong)'}`,
                  background: filter === id ? 'var(--accent)' : 'transparent',
                  color: filter === id ? 'var(--accent-ink)' : 'var(--text-2)',
                }}
              >
                {label} <span style={{ opacity: 0.75 }}>{fmtInt(counts[id])}</span>
              </button>
            ))}
          </div>
        </div>
        <StreamTable rows={rows} slowMs={slowMs} selected={selected} onOpen={open} onSelect={setSelected} />
        <div className="row wrap muted" style={{ gap: '8px 16px', padding: '12px 18px', fontSize: 12 }}>
          <span>
            Sampling:{' '}
            {project.settings.sampleRate >= 1
              ? '100% of requests'
              : `${Math.round(project.settings.sampleRate * 100)}% of healthy requests · 100% of errors and slow requests`}
          </span>
          <span className="mono" style={{ marginLeft: 'auto' }}>
            bodies redacted at the SDK and server
          </span>
        </div>
      </section>
    </main>
  );
}

function deltaText(t: Tile, kind: 'count' | 'rate' | 'ms'): { text: string; color: string } {
  if (t.previous === 0) return { text: t.value ? 'new traffic' : 'no data', color: 'var(--muted)' };
  if (kind === 'rate') {
    const up = t.value > t.previous * 1.5 && t.value - t.previous > 0.005;
    return { text: `${up ? '↑' : t.value < t.previous ? '↓' : ''} from ${fmtPct(t.previous)}`, color: up ? 'var(--err-text)' : 'var(--muted)' };
  }
  const change = (t.value - t.previous) / t.previous;
  const pct = `${change >= 0 ? '+' : ''}${Math.round(change * 100)}%`;
  if (kind === 'ms') return { text: `${change > 0.15 ? '↑' : ''} ${pct}`, color: change > 0.15 ? 'var(--warn)' : 'var(--muted)' };
  return { text: `${pct} vs previous`, color: 'var(--muted)' };
}

function Tiles({ data, loading }: { data: Overview | undefined; loading: boolean }) {
  if (loading || !data) {
    return (
      <>
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="panel tile">
            <Skeleton h={14} w="40%" />
            <Skeleton h={34} />
          </div>
        ))}
      </>
    );
  }
  const tiles = [
    { label: 'Requests / min', value: fmtCompact(data.requestsPerMin.value), unit: '', d: deltaText(data.requestsPerMin, 'count'), color: 'var(--ok)', series: data.requestsPerMin.series },
    { label: 'Error rate', value: (data.errorRate.value * 100).toFixed(1), unit: '%', d: deltaText(data.errorRate, 'rate'), color: 'var(--err)', series: data.errorRate.series },
    { label: 'p95 latency', value: fmtInt(data.p95.value), unit: 'ms', d: deltaText(data.p95, 'ms'), color: 'var(--warn)', series: data.p95.series },
    { label: 'Slow requests', value: fmtCompact(data.slow.value), unit: `> ${fmtMs(data.slowMs)}`, d: { text: `last ${data.minutes} min`, color: 'var(--muted)' }, color: 'var(--accent)', series: data.slow.series },
  ];
  return (
    <>
      {tiles.map((t) => (
        <div key={t.label} className="panel tile">
          <div className="row" style={{ justifyContent: 'space-between' }}>
            <span className="muted" style={{ fontSize: 12 }}>
              {t.label}
            </span>
            <span className="mono" style={{ fontSize: 11.5, color: t.d.color }}>
              {t.d.text}
            </span>
          </div>
          <div className="row" style={{ alignItems: 'flex-end', justifyContent: 'space-between', gap: 12 }}>
            <span className="row" style={{ alignItems: 'baseline', gap: 6 }}>
              <span className="tile-value">{t.value}</span>
              <span className="mono muted" style={{ fontSize: 12 }}>
                {t.unit}
              </span>
            </span>
            <Sparkline values={t.series} color={t.color} label={`${t.label} over the window`} />
          </div>
        </div>
      ))}
    </>
  );
}

const STREAM_COLS = '118px 66px 62px minmax(220px, 1fr) 210px 92px 110px 112px';

function StreamTable({
  rows,
  slowMs,
  selected,
  onOpen,
  onSelect,
}: {
  rows: LiveRequest[];
  slowMs: number;
  selected: number;
  onOpen: (r: LiveRequest) => void;
  onSelect: (i: number) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => parent.current, estimateSize: () => 40, overscan: 12 });
  const maxMs = Math.max(1500, slowMs * 1.5);
  return (
    <div className="scroll-x">
      <div style={{ minWidth: 1000 }}>
        <div className="table-head" style={{ gridTemplateColumns: STREAM_COLS }}>
          <span>Time</span>
          <span>Method</span>
          <span>Status</span>
          <span>Route</span>
          <span>Duration</span>
          <span>User</span>
          <span>Client</span>
          <span>Service</span>
        </div>
        {rows.length === 0 ? (
          <Empty title="Waiting for requests">
            Matching requests stream in here as they happen. Send traffic with an SDK, or run <span className="mono">trailscope demo</span>.
          </Empty>
        ) : (
          <div ref={parent} style={{ height: Math.min(560, rows.length * 40 + 2), overflowY: 'auto' }}>
            <div style={{ height: v.getTotalSize(), position: 'relative' }}>
              {v.getVirtualItems().map((item) => {
                const r = rows[item.index]!;
                const slow = r.durationMs >= slowMs;
                return (
                  <div
                    key={item.key}
                    role="link"
                    tabIndex={0}
                    className={`table-row${r.status >= 500 ? ' err' : ''}${item.index === selected ? ' selected' : ''}`}
                    onClick={() => onOpen(r)}
                    onFocus={() => onSelect(item.index)}
                    onKeyDown={(e) => e.key === 'Enter' && onOpen(r)}
                    style={{ gridTemplateColumns: STREAM_COLS, position: 'absolute', top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}
                  >
                    <span className="cell-mono muted">{fmtClock(r.timestamp)}</span>
                    <span className="cell-mono" style={{ fontWeight: 600, color: 'var(--text-2)' }}>
                      {r.method}
                    </span>
                    <StatusPill status={r.status} />
                    <span className="row" style={{ minWidth: 0 }}>
                      <span className="mono ellipsis" style={{ fontSize: 13 }}>
                        {r.route}
                      </span>
                      {slow && <span className="badge slow">SLOW</span>}
                    </span>
                    <span className="row" style={{ gap: 10 }}>
                      <span className="bar">
                        <span
                          style={{
                            width: `${Math.max(3, Math.min(100, Math.round((r.durationMs / maxMs) * 100)))}%`,
                            background: r.durationMs >= slowMs ? 'var(--err)' : r.durationMs >= slowMs * 0.8 ? 'var(--warn)' : 'var(--faint)',
                          }}
                        />
                      </span>
                      <span className="cell-mono" style={{ width: 72, textAlign: 'right', color: 'var(--text-3)' }}>
                        {fmtMs(r.durationMs)}
                      </span>
                    </span>
                    <span className="cell-mono ellipsis" style={{ color: 'var(--text-2)' }}>
                      {r.userId || '—'}
                    </span>
                    <span className="muted ellipsis" style={{ fontSize: 12 }}>
                      {[r.client, r.clientVersion].filter(Boolean).join(' ') || '—'}
                    </span>
                    <span className="cell-mono muted ellipsis">{r.service}</span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
