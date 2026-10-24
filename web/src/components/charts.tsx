import * as Tooltip from '@radix-ui/react-tooltip';
import { useMemo, useState, type ReactNode } from 'react';

import { fmtCompact, fmtHm, fmtInt, fmtMs, fmtPct } from '../lib/format';
import type { HeatRoute, Release } from '../lib/types';
import { IconMarker } from './icons';

export type HeatMetric = 'p95' | 'errors' | 'volume';

/** Fill for one heatmap cell, from fast to over-SLO. */
function heatFill(value: number, metric: HeatMetric, max: number, slo: number): string {
  if (metric === 'p95') {
    if (value >= slo) return 'var(--err)';
    const v = Math.min(1, value / slo);
    return `rgba(var(--heat-rgb), ${(0.08 + v * 0.8).toFixed(2)})`;
  }
  if (metric === 'errors') {
    if (value >= 0.1) return 'var(--err)';
    return `rgba(var(--heat-rgb), ${(0.08 + Math.min(1, value / 0.1) * 0.8).toFixed(2)})`;
  }
  return `rgba(138, 180, 248, ${(0.08 + (max > 0 ? value / max : 0) * 0.8).toFixed(2)})`;
}

export function Heatmap({
  times,
  routes,
  sloMs,
  metric,
  releases,
  onCell,
}: {
  times: number[];
  routes: HeatRoute[];
  sloMs: number;
  metric: HeatMetric;
  releases: Release[];
  onCell: (route: string, t: number) => void;
}) {
  const step = times.length > 1 ? times[1]! - times[0]! : 60_000;
  const start = times[0] ?? 0;
  const end = (times[times.length - 1] ?? 0) + step;
  const maxVol = useMemo(
    () => Math.max(1, ...routes.flatMap((r) => r.cells.map((c) => c?.requests ?? 0))),
    [routes],
  );
  const markers = releases.filter((r) => r.at >= start && r.at < end);
  const value = (c: HeatRoute['cells'][number]) =>
    !c ? 0 : metric === 'p95' ? c.p95 : metric === 'errors' ? c.errorRate : c.requests;
  const current = (r: HeatRoute) =>
    metric === 'p95' ? fmtMs(r.current.p95) : metric === 'errors' ? fmtPct(r.current.errorRate) : fmtCompact(r.current.requests);
  const hot = (r: HeatRoute) => (metric === 'p95' ? r.current.p95 >= sloMs : metric === 'errors' ? r.current.errorRate >= 0.05 : false);
  const cols = `190px minmax(0, 1fr) 70px`;
  const labels = [0, 1 / 3, 2 / 3].map((f) => fmtHm(start + (end - start) * f));
  return (
    <Tooltip.Provider delayDuration={100}>
      <div className="scroll-x">
        <div style={{ minWidth: 640, display: 'flex', flexDirection: 'column', gap: 5 }}>
          <div style={{ display: 'grid', gridTemplateColumns: cols, gap: 12 }}>
            <span />
            <div style={{ position: 'relative', height: 20 }}>
              {markers.map((m) => (
                <span
                  key={m.id}
                  title={`${m.service} ${m.version}`}
                  className="mono"
                  style={{
                    position: 'absolute',
                    left: `${(((m.at - start) / (end - start)) * 100).toFixed(1)}%`,
                    transform: 'translateX(-4px)',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    whiteSpace: 'nowrap',
                    fontSize: 11,
                    color: 'var(--accent)',
                  }}
                >
                  <IconMarker />
                  {m.version} deployed · {fmtHm(m.at)}
                </span>
              ))}
            </div>
            <span />
          </div>
          {routes.map((r) => (
            <div key={r.route} style={{ display: 'grid', gridTemplateColumns: cols, gap: 12, alignItems: 'center' }}>
              <span className="mono ellipsis" style={{ fontSize: 12, color: 'var(--text-3)' }} title={r.route}>
                {r.route}
              </span>
              <div style={{ display: 'grid', gridTemplateColumns: `repeat(${times.length}, minmax(0, 1fr))`, gap: 3 }}>
                {r.cells.map((c, i) => {
                  const t = times[i] ?? 0;
                  const tip = c
                    ? `${r.route} · ${fmtHm(t)} · p95 ${fmtMs(c.p95)} · ${fmtInt(c.requests)} req · ${fmtPct(c.errorRate)} 5xx`
                    : `${r.route} · ${fmtHm(t)} · no traffic`;
                  return (
                    <Tooltip.Root key={t}>
                      <Tooltip.Trigger asChild>
                        <button
                          type="button"
                          aria-label={tip}
                          onClick={() => onCell(r.route, t)}
                          style={{
                            height: 18,
                            borderRadius: 3,
                            border: 0,
                            padding: 0,
                            cursor: 'pointer',
                            background: c ? heatFill(value(c), metric, maxVol, sloMs) : 'var(--raised)',
                          }}
                        />
                      </Tooltip.Trigger>
                      <Tooltip.Portal>
                        <Tooltip.Content className="tooltip" sideOffset={4}>
                          {tip}
                        </Tooltip.Content>
                      </Tooltip.Portal>
                    </Tooltip.Root>
                  );
                })}
              </div>
              <span className="mono" style={{ fontSize: 12, textAlign: 'right', color: hot(r) ? 'var(--err-text)' : 'var(--text-2)' }}>
                {current(r)}
              </span>
            </div>
          ))}
          <div style={{ display: 'grid', gridTemplateColumns: cols, gap: 12, marginTop: 4 }}>
            <span className="dim" style={{ fontSize: 11 }}>
              route
            </span>
            <div className="mono dim" style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11 }}>
              {labels.map((l) => (
                <span key={l}>{l}</span>
              ))}
              <span>now</span>
            </div>
            <span className="dim" style={{ fontSize: 11, textAlign: 'right' }}>
              {metric === 'p95' ? 'p95 now' : metric === 'errors' ? '5xx now' : 'req now'}
            </span>
          </div>
        </div>
      </div>
      <div className="row mono muted" style={{ fontSize: 11 }}>
        <span>{metric === 'volume' ? 'quiet' : 'fast'}</span>
        {[0.12, 0.35, 0.6, 0.85].map((a) => (
          <span
            key={a}
            style={{
              width: 18,
              height: 10,
              borderRadius: 2,
              background: metric === 'volume' ? `rgba(138,180,248,${a})` : `rgba(var(--heat-rgb), ${a})`,
            }}
          />
        ))}
        {metric !== 'volume' && <span style={{ width: 18, height: 10, borderRadius: 2, background: 'var(--err)' }} />}
        <span>{metric === 'p95' ? `over SLO (${fmtMs(sloMs)})` : metric === 'errors' ? '≥ 10% 5xx' : 'busy'}</span>
      </div>
    </Tooltip.Provider>
  );
}

const SERIES_COLORS: Record<string, string> = {
  '2xx': 'var(--ok)',
  '3xx': 'var(--text-2)',
  '4xx': 'var(--warn)',
  '5xx': 'var(--err)',
  ok: 'var(--ok)',
  warn: 'var(--warn)',
  error: 'var(--err)',
};

/** Stacked bar histogram with release markers; drag to zoom. */
export function Histogram({
  series,
  points,
  bucketMs,
  from,
  to,
  releases,
  onZoom,
}: {
  series: string[];
  points: number[][];
  bucketMs: number;
  from: number;
  to: number;
  releases: Release[];
  onZoom?: (from: number, to: number) => void;
}) {
  const width = 1000;
  const height = 140;
  const [hover, setHover] = useState<number | null>(null);
  const [drag, setDrag] = useState<{ a: number; b: number } | null>(null);
  const span = Math.max(1, to - from);
  const x = (t: number) => ((t - from) / span) * width;
  const totals = points.map((p) => p.slice(1).reduce((a, b) => a + b, 0));
  const max = Math.max(1, ...totals);
  const bw = Math.max(1, (bucketMs / span) * width - 1);
  const toTime = (clientX: number, el: SVGSVGElement) => {
    const rect = el.getBoundingClientRect();
    return from + ((clientX - rect.left) / rect.width) * span;
  };
  const hovered = hover !== null ? points[hover] : undefined;
  return (
    <div style={{ position: 'relative', overflow: 'hidden' }}>
      <svg
        viewBox={`0 0 ${width} ${height + 18}`}
        width="100%"
        height={height + 18}
        preserveAspectRatio="none"
        role="img"
        aria-label="Requests over time by status class"
        onMouseDown={(e) => onZoom && setDrag({ a: toTime(e.clientX, e.currentTarget), b: toTime(e.clientX, e.currentTarget) })}
        onMouseMove={(e) => {
          const t = toTime(e.clientX, e.currentTarget);
          if (drag) setDrag({ ...drag, b: t });
          const i = points.findIndex((p) => t >= p[0]! && t < p[0]! + bucketMs);
          setHover(i >= 0 ? i : null);
        }}
        onMouseLeave={() => {
          setHover(null);
          setDrag(null);
        }}
        onMouseUp={() => {
          if (drag && onZoom && Math.abs(drag.b - drag.a) > bucketMs) onZoom(Math.min(drag.a, drag.b), Math.max(drag.a, drag.b));
          setDrag(null);
        }}
        style={{ display: 'block', cursor: onZoom ? 'crosshair' : 'default' }}
      >
        {points.map((p, i) => {
          let y = height;
          return series.map((s, si) => {
            const v = p[si + 1] ?? 0;
            const h = (v / max) * (height - 8);
            y -= h;
            return v > 0 ? (
              <rect key={`${i}-${s}`} x={x(p[0]!)} y={y} width={bw} height={h} fill={SERIES_COLORS[s] ?? 'var(--ok)'} opacity={hover === null || hover === i ? 0.9 : 0.5} />
            ) : null;
          });
        })}
        {releases
          .filter((r) => r.at >= from && r.at <= to)
          .map((r) => (
            <g key={r.id}>
              <line x1={x(r.at)} x2={x(r.at)} y1={0} y2={height} stroke="var(--accent)" strokeDasharray="3 3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
            </g>
          ))}
        {drag && <rect x={x(Math.min(drag.a, drag.b))} y={0} width={Math.abs(x(drag.b) - x(drag.a))} height={height} fill="var(--accent-soft)" />}
        <line x1={0} x2={width} y1={height} y2={height} stroke="var(--line-strong)" vectorEffect="non-scaling-stroke" />
      </svg>
      <div className="mono dim" style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, marginTop: -14 }}>
        {[0, 0.25, 0.5, 0.75, 1].map((f) => (
          <span key={f}>{fmtHm(from + span * f)}</span>
        ))}
      </div>
      {releases
        .filter((r) => r.at >= from && r.at <= to)
        .map((r) => (
          <span
            key={r.id}
            className="mono"
            style={{ position: 'absolute', top: -2, left: `${(((r.at - from) / span) * 100).toFixed(1)}%`, transform: 'translateX(4px)', fontSize: 10.5, color: 'var(--accent)', whiteSpace: 'nowrap', pointerEvents: 'none' }}
          >
            {r.version}
          </span>
        ))}
      {hovered && (
        <div className="tooltip" style={{ position: 'absolute', top: 6, right: 6, pointerEvents: 'none' }}>
          {fmtHm(hovered[0]!)} ·{' '}
          {series.map((s, i) => `${s} ${fmtInt(hovered[i + 1] ?? 0)}`).join(' · ')}
        </div>
      )}
    </div>
  );
}

export function Legend({ series }: { series: string[] }) {
  return (
    <span className="row muted" style={{ gap: 14, fontSize: 12 }}>
      {series.map((s) => (
        <span key={s} className="row" style={{ gap: 6 }}>
          <span className="dot" style={{ width: 9, height: 9, background: SERIES_COLORS[s] }} />
          {s}
        </span>
      ))}
    </span>
  );
}

export interface SpanBar {
  id: string;
  kind: string;
  name: string;
  start: number;
  duration: number;
  error: boolean;
  root: boolean;
}

export function Waterfall({ spans, total }: { spans: SpanBar[]; total: number }) {
  const t = Math.max(1, total);
  return (
    <div className="scroll-x">
      <div style={{ minWidth: 640, display: 'flex', flexDirection: 'column', gap: 6 }}>
        {spans.map((s) => (
          <div key={s.id} style={{ display: 'grid', gridTemplateColumns: '240px minmax(0, 1fr) 76px', gap: 12, alignItems: 'center', minHeight: 26 }}>
            <span className="row" style={{ minWidth: 0 }}>
              <span className="mono muted" style={{ fontSize: 10.5, border: '1px solid var(--line-strong)', borderRadius: 4, padding: '0 5px' }}>
                {s.kind}
              </span>
              <span className="mono ellipsis" style={{ fontSize: 12, color: 'var(--text-3)' }} title={s.name}>
                {s.name}
              </span>
            </span>
            <span style={{ position: 'relative', height: 14, background: 'var(--sidebar)', borderRadius: 3 }}>
              <span
                style={{
                  position: 'absolute',
                  top: 0,
                  bottom: 0,
                  borderRadius: 3,
                  left: `${Math.min(100, (s.start / t) * 100).toFixed(2)}%`,
                  width: `${Math.max(0.6, (s.duration / t) * 100).toFixed(2)}%`,
                  background: s.root ? 'var(--span)' : s.error ? 'var(--err)' : 'var(--ok)',
                }}
              />
            </span>
            <span className="mono" style={{ fontSize: 12, textAlign: 'right', color: s.error ? 'var(--err-text)' : 'var(--text-2)' }}>
              {fmtMs(s.duration)}
            </span>
          </div>
        ))}
        <div style={{ display: 'grid', gridTemplateColumns: '240px minmax(0, 1fr) 76px', gap: 12 }}>
          <span />
          <span className="mono dim" style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11 }}>
            {[0, 0.25, 0.5, 0.75, 1].map((f) => (
              <span key={f}>{f === 1 ? fmtMs(t) : fmtInt(t * f)}</span>
            ))}
          </span>
          <span />
        </div>
      </div>
    </div>
  );
}

const REDACTED = '[REDACTED]';

/** Pretty-printed JSON with syntax colours and redacted values as chips. */
export function JsonView({ text }: { text: string }) {
  const parsed = useMemo(() => {
    try {
      return { ok: true as const, value: JSON.parse(text) as unknown };
    } catch {
      return { ok: false as const };
    }
  }, [text]);
  if (!text) return <pre className="code muted">(empty)</pre>;
  if (!parsed.ok) return <pre className="code">{highlightRedacted(text)}</pre>;
  return <pre className="code">{render(parsed.value, 0)}</pre>;
}

function highlightRedacted(text: string): ReactNode[] {
  return text.split(REDACTED).flatMap((part, i) => (i === 0 ? [part] : [<span key={i} className="redacted">REDACTED</span>, part]));
}

function render(v: unknown, depth: number): ReactNode {
  const pad = '  '.repeat(depth);
  const padIn = '  '.repeat(depth + 1);
  if (v === null) return <span className="json-lit">null</span>;
  if (typeof v === 'string') {
    if (v === REDACTED) return <span className="redacted">REDACTED</span>;
    return <span className="json-str">{JSON.stringify(v)}</span>;
  }
  if (typeof v === 'number') return <span className="json-num">{v}</span>;
  if (typeof v === 'boolean') return <span className="json-lit">{String(v)}</span>;
  if (Array.isArray(v)) {
    if (v.length === 0) return '[]';
    return (
      <>
        {'[\n'}
        {v.map((item, i) => (
          <span key={i}>
            {padIn}
            {render(item, depth + 1)}
            {i < v.length - 1 ? ',\n' : '\n'}
          </span>
        ))}
        {pad}]
      </>
    );
  }
  if (typeof v === 'object') {
    const entries = Object.entries(v as Record<string, unknown>);
    if (entries.length === 0) return '{}';
    return (
      <>
        {'{\n'}
        {entries.map(([k, item], i) => (
          <span key={k}>
            {padIn}
            <span className="json-key">{JSON.stringify(k)}</span>: {render(item, depth + 1)}
            {i < entries.length - 1 ? ',\n' : '\n'}
          </span>
        ))}
        {pad}
        {'}'}
      </>
    );
  }
  return String(v);
}

export function countRedacted(text: string): number {
  return text.split(REDACTED).length - 1;
}

export interface ChartSeries {
  name: string;
  values: number[];
  color: string;
  /** Bars are drawn under lines. */
  kind?: 'line' | 'bar';
  dashed?: boolean;
}

/** Time series as lines and bars, with an optional threshold and release markers. */
export function TimeChart({
  times,
  step,
  series,
  releases = [],
  height = 150,
  threshold,
  format = fmtInt,
  label,
}: {
  times: number[];
  step: number;
  series: ChartSeries[];
  releases?: Release[];
  height?: number;
  threshold?: { value: number; label: string };
  format?: (v: number) => string;
  label: string;
}) {
  const width = 1000;
  const [hover, setHover] = useState<number | null>(null);
  const from = times[0] ?? 0;
  const to = (times[times.length - 1] ?? 0) + step;
  const span = Math.max(1, to - from);
  const x = (t: number) => ((t - from) / span) * width;
  const max = Math.max(1, threshold?.value ?? 0, ...series.flatMap((s) => s.values.filter(Number.isFinite))) * 1.08;
  const y = (v: number) => height - (v / max) * (height - 6);
  const bw = Math.max(1, (step / span) * width - 1.5);
  const markers = releases.filter((r) => r.at >= from && r.at <= to).sort((a, b) => a.at - b.at);
  // Labels closer than ~8% of the width would overlap; their lines still show.
  const labelled = markers.filter((r, i, all) => i === 0 || (r.at - all[i - 1]!.at) / span > 0.08);
  return (
    <div style={{ position: 'relative', overflow: 'hidden' }}>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        height={height}
        preserveAspectRatio="none"
        role="img"
        aria-label={label}
        style={{ display: 'block' }}
        onMouseMove={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          const t = from + ((e.clientX - rect.left) / rect.width) * span;
          const i = Math.floor((t - from) / step);
          setHover(i >= 0 && i < times.length ? i : null);
        }}
        onMouseLeave={() => setHover(null)}
      >
        {series
          .filter((s) => s.kind === 'bar')
          .map((s) =>
            s.values.map((v, i) =>
              v > 0 ? <rect key={`${s.name}-${i}`} x={x(times[i]!)} y={y(v)} width={bw} height={height - y(v)} fill={s.color} opacity={hover === null || hover === i ? 0.8 : 0.45} /> : null,
            ),
          )}
        {threshold && (
          <line x1={0} x2={width} y1={y(threshold.value)} y2={y(threshold.value)} stroke="var(--err)" strokeDasharray="4 4" strokeWidth={1} vectorEffect="non-scaling-stroke" opacity={0.7} />
        )}
        {series
          .filter((s) => s.kind !== 'bar')
          .flatMap((s) =>
            segments(s.values).map(([a, b]) => (
              <polyline
                key={`${s.name}-${a}`}
                points={s.values
                  .slice(a, b)
                  .map((v, i) => `${(x(times[a + i]!) + bw / 2).toFixed(1)},${y(v).toFixed(1)}`)
                  .join(' ')}
                fill="none"
                stroke={s.color}
                strokeWidth={1.6}
                strokeDasharray={s.dashed ? '4 3' : undefined}
                strokeLinejoin="round"
                strokeLinecap="round"
                vectorEffect="non-scaling-stroke"
              />
            )),
          )}
        {markers.map((r) => (
          <line key={r.id} x1={x(r.at)} x2={x(r.at)} y1={0} y2={height} stroke="var(--accent)" strokeDasharray="3 3" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
        ))}
        {hover !== null && <line x1={x(times[hover]!) + bw / 2} x2={x(times[hover]!) + bw / 2} y1={0} y2={height} stroke="var(--line-focus)" vectorEffect="non-scaling-stroke" />}
        <line x1={0} x2={width} y1={height - 0.5} y2={height - 0.5} stroke="var(--line-strong)" vectorEffect="non-scaling-stroke" />
      </svg>
      {labelled.map((r) => (
        <span
          key={r.id}
          className="mono"
          title={`${r.service} ${r.version}`}
          style={{ position: 'absolute', top: 0, left: `${(((r.at - from) / span) * 100).toFixed(1)}%`, transform: 'translateX(4px)', fontSize: 10.5, color: 'var(--accent)', whiteSpace: 'nowrap', pointerEvents: 'none' }}
        >
          {r.version}
        </span>
      ))}
      {threshold && (
        <span className="mono" style={{ position: 'absolute', right: 4, top: Math.max(0, y(threshold.value) - 16), fontSize: 10.5, color: 'var(--err-text)', pointerEvents: 'none' }}>
          {threshold.label}
        </span>
      )}
      <div className="mono dim" style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, marginTop: 4 }}>
        {[0, 0.25, 0.5, 0.75, 1].map((f) => (
          <span key={f}>{fmtTick(from + span * f, span)}</span>
        ))}
      </div>
      {hover !== null && (
        <div className="tooltip" style={{ position: 'absolute', top: 6, left: 6, pointerEvents: 'none' }}>
          {fmtTick(times[hover]!, span)} · {series.map((s) => `${s.name} ${Number.isFinite(s.values[hover]) ? format(s.values[hover] ?? 0) : '–'}`).join(' · ')}
        </div>
      )}
    </div>
  );
}

/** Index ranges of consecutive finite values; NaN marks a gap. */
function segments(values: number[]): [number, number][] {
  const out: [number, number][] = [];
  let start = -1;
  values.forEach((v, i) => {
    if (Number.isFinite(v)) {
      if (start < 0) start = i;
    } else if (start >= 0) {
      out.push([start, i]);
      start = -1;
    }
  });
  if (start >= 0) out.push([start, values.length]);
  return out;
}

function fmtTick(t: number, span: number): string {
  if (span <= 36 * 3_600_000) return fmtHm(t);
  const d = new Date(t);
  return `${d.getMonth() + 1}/${d.getDate()} ${fmtHm(t)}`;
}
