import { useEffect, useState, type ReactNode } from 'react';

import { statusClass } from '../lib/format';

export function StatusPill({ status }: { status: number }) {
  return <span className={`pill ${statusClass(status)}`}>{status}</span>;
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd>{children}</kbd>;
}

export function Skeleton({ h = 16, w = '100%' }: { h?: number; w?: number | string }) {
  return <div className="skeleton" style={{ height: h, width: w }} aria-hidden="true" />;
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <h3>{title}</h3>
      {children}
    </div>
  );
}

/** Polyline sparkline in a w×h box. */
export function Sparkline({
  values,
  width = 110,
  height = 34,
  color = 'var(--ok)',
  label,
}: {
  values: number[];
  width?: number;
  height?: number;
  color?: string;
  label?: string;
}) {
  if (values.length < 2) return <svg width={width} height={height} aria-hidden="true" />;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const span = max - min || 1;
  const pts = values
    .map((v, i) => `${((i * width) / (values.length - 1)).toFixed(1)},${(height - 3 - ((v - min) / span) * (height - 6)).toFixed(1)}`)
    .join(' ');
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.6" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

export function useToast(): [string | null, (msg: string) => void] {
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    if (!msg) return;
    const t = window.setTimeout(() => setMsg(null), 2200);
    return () => window.clearTimeout(t);
  }, [msg]);
  return [msg, setMsg];
}

export function Toast({ msg }: { msg: string | null }) {
  if (!msg) return null;
  return (
    <div className="toast" role="status">
      {msg}
    </div>
  );
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  small,
  label,
}: {
  options: { id: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  small?: boolean;
  label: string;
}) {
  return (
    <div role="group" aria-label={label} className={`segmented${small ? ' small' : ''}`}>
      {options.map((o) => (
        <button key={o.id} type="button" aria-pressed={o.id === value} onClick={() => onChange(o.id)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A one-time secret, shown once with a copy button. */
export function SecretOnce({ label, value, onDone }: { label: string; value: string; onDone: () => void }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="panel panel-pad" role="status" style={{ display: 'flex', flexDirection: 'column', gap: 10, borderColor: 'var(--accent-line)' }}>
      <span style={{ fontSize: 13 }}>{label} It is shown only once.</span>
      <code className="mono" style={{ fontSize: 12.5, padding: '10px 12px', background: 'var(--sidebar)', borderRadius: 8, overflowWrap: 'anywhere' }}>
        {value}
      </code>
      <span className="row">
        <button type="button" className="btn btn-sm btn-primary" onClick={() => void copyText(value).then(setCopied)}>
          {copied ? 'Copied' : 'Copy'}
        </button>
        <button type="button" className="btn btn-sm" onClick={onDone}>
          Done
        </button>
      </span>
    </div>
  );
}
