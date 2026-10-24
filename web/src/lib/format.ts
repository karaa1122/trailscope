const nf = new Intl.NumberFormat('en-US');

export function fmtInt(n: number): string {
  return nf.format(Math.round(n));
}

/** 1234 → "1.2k", 4812 → "4,812" under 10k. */
export function fmtCompact(n: number): string {
  if (Math.abs(n) >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (Math.abs(n) >= 10_000) return `${(n / 1000).toFixed(1)}k`;
  return fmtInt(n);
}

export function fmtMs(ms: number): string {
  if (!Number.isFinite(ms)) return '–';
  if (ms >= 10_000) return `${(ms / 1000).toFixed(1)} s`;
  if (ms >= 100) return `${fmtInt(ms)} ms`;
  if (ms >= 10) return `${ms.toFixed(0)} ms`;
  return `${ms.toFixed(1)} ms`;
}

export function fmtPct(ratio: number, digits = 1): string {
  return `${(ratio * 100).toFixed(digits)}%`;
}

export function fmtBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${fmtInt(n)} B`;
}

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

/** "21:29:58.412" in local time. */
export function fmtClock(ms: number, withMs = true): string {
  const d = new Date(ms);
  const base = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  return withMs ? `${base}.${pad(d.getMilliseconds(), 3)}` : base;
}

export function fmtHm(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function fmtDateTime(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${fmtClock(ms, false)}`;
}

export function fmtAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

export function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

export function statusClass(status: number): 's2' | 's3' | 's4' | 's5' {
  if (status >= 500) return 's5';
  if (status >= 400) return 's4';
  if (status >= 300) return 's3';
  return 's2';
}

export function str(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  return String(v);
}

export function num(v: unknown): number {
  if (typeof v === 'number') return v;
  if (typeof v === 'string') return Number(v) || 0;
  return 0;
}

/** Relative ranges used by the time picker. */
export const RANGES: { id: string; label: string; ms: number }[] = [
  { id: '5m', label: 'Last 5 min', ms: 5 * 60_000 },
  { id: '15m', label: 'Last 15 min', ms: 15 * 60_000 },
  { id: '30m', label: 'Last 30 min', ms: 30 * 60_000 },
  { id: '1h', label: 'Last hour', ms: 3_600_000 },
  { id: '6h', label: 'Last 6 hours', ms: 6 * 3_600_000 },
  { id: '24h', label: 'Last 24 hours', ms: 24 * 3_600_000 },
  { id: '7d', label: 'Last 7 days', ms: 7 * 86_400_000 },
  { id: '30d', label: 'Last 30 days', ms: 30 * 86_400_000 },
];

export function rangeMs(id: string | undefined, fallback = '15m'): number {
  return (RANGES.find((r) => r.id === id) ?? RANGES.find((r) => r.id === fallback) ?? RANGES[1]!).ms;
}

export function rangeLabel(id: string | undefined, fallback = '15m'): string {
  return (RANGES.find((r) => r.id === id) ?? RANGES.find((r) => r.id === fallback) ?? RANGES[1]!).label;
}
