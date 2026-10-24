import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';

import { Heatmap, TimeChart, type HeatMetric } from '../components/charts';
import { Empty, Segmented, Skeleton } from '../components/ui';
import { api } from '../lib/api';
import { fmtCompact, fmtInt, fmtMs, fmtPct } from '../lib/format';
import { useSession } from '../lib/session';

export interface RoutesSearch {
  route?: string;
  minutes?: number;
  metric?: HeatMetric;
}

const WINDOWS = [
  { id: '30', label: '30m' },
  { id: '60', label: '1h' },
  { id: '360', label: '6h' },
  { id: '1440', label: '24h' },
];

const sqlString = (s: string) => `'${s.replace(/\\/g, '\\\\').replace(/'/g, "''")}'`;

export function Routes({ search }: { search: RoutesSearch }) {
  const { project, environment } = useSession();
  const navigate = useNavigate();
  const minutes = search.minutes ?? 60;
  const metric = search.metric ?? 'p95';
  const [now] = useState(() => Date.now());
  const set = (patch: Partial<RoutesSearch>) => void navigate({ to: '/routes', search: { ...search, ...patch } });

  const heat = useQuery({
    queryKey: ['heatmap', 'full', project?.id, environment, minutes],
    enabled: !!project,
    queryFn: () => api.heatmap(project?.id ?? '', environment, minutes, 100),
    refetchInterval: 30_000,
  });
  const releases = useQuery({
    queryKey: ['releases', project?.id, environment, 'routes', minutes],
    enabled: !!project,
    queryFn: () => api.releases(project?.id ?? '', now - minutes * 60_000, now + 60_000, environment),
  });
  const route = search.route ?? heat.data?.routes[0]?.route;
  const series = useQuery({
    queryKey: ['route-series', project?.id, environment, route, minutes],
    enabled: !!project && !!route,
    queryFn: () => api.routeSeries(project?.id ?? '', route ?? '', environment, minutes),
    refetchInterval: 30_000,
  });
  const s = series.data;
  const step = (s?.bucketMinutes ?? 1) * 60_000;
  // Buckets without traffic have no latency: draw them as gaps.
  const gap = (values: number[]) => values.map((v, i) => (s?.requests[i] ? v : NaN));
  const totals = s
    ? {
        requests: s.requests.reduce((a, b) => a + b, 0),
        e5: s.errors5xx.reduce((a, b) => a + b, 0),
        e4: s.errors4xx.reduce((a, b) => a + b, 0),
        p95: Math.max(0, ...s.p95),
      }
    : null;

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Routes</h1>
        <span className="panel-sub">every route × time; click a cell or a route to see its latency</span>
        <span className="row" style={{ marginLeft: 'auto', gap: 10 }}>
          <Segmented
            small
            label="Heatmap metric"
            value={metric}
            onChange={(m) => set({ metric: m })}
            options={[
              { id: 'p95', label: 'p95' },
              { id: 'errors', label: 'errors' },
              { id: 'volume', label: 'volume' },
            ]}
          />
          <Segmented small label="Window" value={String(minutes)} onChange={(m) => set({ minutes: Number(m) })} options={WINDOWS} />
        </span>
      </header>

      <section aria-label="Route heatmap" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        {heat.isLoading ? (
          <Skeleton h={320} />
        ) : heat.data && heat.data.routes.length ? (
          <Heatmap
            times={heat.data.times}
            routes={heat.data.routes}
            sloMs={heat.data.sloMs}
            metric={metric}
            releases={releases.data ?? []}
            onCell={(r) => set({ route: r })}
          />
        ) : (
          <Empty title="No traffic yet">Routes appear here a few seconds after your services send requests.</Empty>
        )}
      </section>

      {route && (
        <section aria-label="Route latency" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          <div className="row wrap" style={{ gap: 12 }}>
            <h2 className="panel-title mono" style={{ fontWeight: 500 }}>
              {route}
            </h2>
            {s && (
              <span className="row" style={{ gap: 6 }}>
                {Object.entries(s.methods)
                  .sort((a, b) => b[1] - a[1])
                  .map(([m, n]) => (
                    <span key={m} className="chip">
                      {m} {fmtCompact(n)}
                    </span>
                  ))}
              </span>
            )}
            <span className="row" style={{ marginLeft: 'auto', gap: 8 }}>
              <Link to="/explore" search={{ q: `route = ${sqlString(route)}`, mode: 'sql', range: minutes <= 60 ? '1h' : minutes <= 360 ? '6h' : '24h' }} className="btn btn-sm">
                Requests in Explore
              </Link>
              <Link
                to="/explore"
                search={{ q: `route = ${sqlString(route)} AND status >= 500`, mode: 'sql', range: minutes <= 60 ? '1h' : minutes <= 360 ? '6h' : '24h' }}
                className="btn btn-sm"
              >
                Failures
              </Link>
            </span>
          </div>
          {totals && (
            <div className="status-line">
              <span>
                <b className="mono">{fmtInt(totals.requests)}</b> requests
              </span>
              <span>
                <b className="mono">{fmtPct(totals.requests ? totals.e5 / totals.requests : 0)}</b> 5xx
              </span>
              <span>
                <b className="mono">{fmtPct(totals.requests ? totals.e4 / totals.requests : 0)}</b> 4xx
              </span>
              <span>
                worst p95 <b className="mono">{fmtMs(totals.p95)}</b>
              </span>
              <span>SLO {fmtMs(s?.sloMs ?? 0)}</span>
            </div>
          )}
          {series.isLoading || !s ? (
            <Skeleton h={220} />
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(420px, 1fr))', gap: 18 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <span className="row muted" style={{ fontSize: 12, gap: 14 }}>
                  Latency
                  <Key color="var(--ok)" label="p50" />
                  <Key color="var(--warn)" label="p95" />
                  <Key color="var(--err)" label="p99" />
                </span>
                <TimeChart
                  times={s.times}
                  step={step}
                  label={`Latency percentiles for ${route}`}
                  format={fmtMs}
                  threshold={{ value: s.sloMs, label: `SLO ${fmtMs(s.sloMs)}` }}
                  releases={releases.data ?? []}
                  series={[
                    { name: 'p50', values: gap(s.p50), color: 'var(--ok)' },
                    { name: 'p95', values: gap(s.p95), color: 'var(--warn)' },
                    { name: 'p99', values: gap(s.p99), color: 'var(--err)', dashed: true },
                  ]}
                />
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <span className="row muted" style={{ fontSize: 12, gap: 14 }}>
                  Volume
                  <Key color="var(--span)" label="requests" />
                  <Key color="var(--warn)" label="4xx" />
                  <Key color="var(--err)" label="5xx" />
                </span>
                <TimeChart
                  times={s.times}
                  step={step}
                  label={`Requests and errors for ${route}`}
                  releases={releases.data ?? []}
                  series={[
                    { name: 'requests', values: s.requests, color: 'var(--span)', kind: 'bar' },
                    { name: '4xx', values: s.errors4xx, color: 'var(--warn)' },
                    { name: '5xx', values: s.errors5xx, color: 'var(--err)' },
                  ]}
                />
              </div>
            </div>
          )}
        </section>
      )}
    </main>
  );
}

function Key({ color, label }: { color: string; label: string }) {
  return (
    <span className="row" style={{ gap: 6 }}>
      <span className="dot" style={{ width: 9, height: 9, background: color }} />
      {label}
    </span>
  );
}
