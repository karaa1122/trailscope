import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState, type ReactNode } from 'react';

import { Segmented, Skeleton } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { useSession } from '../lib/session';
import type { SendResult, SmtpSettings } from '../lib/types';

/** Environment variables that lock each field when set. */
const SMTP_ENV: Record<string, string> = {
  host: 'LOGBOX_SMTP_HOST',
  port: 'LOGBOX_SMTP_PORT',
  security: 'LOGBOX_SMTP_SECURITY',
  username: 'LOGBOX_SMTP_USERNAME',
  password: 'LOGBOX_SMTP_PASSWORD',
  from: 'LOGBOX_SMTP_FROM',
  from_name: 'LOGBOX_SMTP_FROM_NAME',
};

const errText = (e: unknown) => (e instanceof ApiError || e instanceof Error ? e.message : 'Something went wrong');

function Panel({ title, sub, children }: { title: string; sub?: ReactNode; children: ReactNode }) {
  return (
    <section aria-label={title} className="panel" style={{ overflow: 'hidden' }}>
      <div className="row wrap" style={{ gap: 12, padding: '14px 18px', borderBottom: '1px solid var(--line)' }}>
        <h2 className="panel-title">{title}</h2>
        {sub && <span className="panel-sub">{sub}</span>}
      </div>
      {children}
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="field">
      {label}
      {children}
      {hint && <span className="dim" style={{ fontSize: 11.5 }}>{hint}</span>}
    </label>
  );
}

export function NotificationsSection({ toast }: { toast: (msg: string) => void }) {
  const qc = useQueryClient();
  const { me } = useSession();
  const smtp = useQuery({ queryKey: ['smtp'], queryFn: api.smtp });
  const [form, setForm] = useState<SmtpSettings | null>(null);
  const [password, setPassword] = useState('');
  const [clearPassword, setClearPassword] = useState(false);
  const [to, setTo] = useState('');
  const [result, setResult] = useState<SendResult | null>(null);
  useEffect(() => {
    if (smtp.data) setForm(smtp.data);
  }, [smtp.data]);
  useEffect(() => {
    if (me) setTo((t) => t || me.user.email);
  }, [me]);

  const save = useMutation({
    mutationFn: (f: SmtpSettings) =>
      api.saveSmtp({
        host: f.host,
        port: f.port,
        security: f.security,
        username: f.username,
        from: f.from,
        fromName: f.fromName,
        // null keeps the stored password, '' removes it.
        password: clearPassword ? '' : password ? password : null,
      }),
    onSuccess: (r) => {
      qc.setQueryData(['smtp'], r);
      void qc.invalidateQueries({ queryKey: ['email-rule'] });
      setPassword('');
      setClearPassword(false);
      toast('Email settings saved');
    },
    onError: (e) => toast(errText(e)),
  });
  const test = useMutation({
    mutationFn: () => api.testSmtp(to.trim()),
    onSuccess: setResult,
    onError: (e) => setResult({ ok: false, error: errText(e), response: null, recipients: [] }),
  });
  if (!form || !smtp.data) return <Skeleton h={300} />;
  const s = smtp.data;
  const locked = (f: string) => s.locked.includes(f);
  const lockHint = (f: string) => (locked(f) ? `locked: set by ${SMTP_ENV[f] ?? 'configuration'}` : undefined);

  return (
    <>
      <Panel title="Email (SMTP)" sub={s.configured ? <span className="ok-text">configured</span> : 'not configured: no emails are sent'}>
        {!s.secretKeyConfigured && (
          <div role="note" style={{ margin: '18px 18px 0', padding: '10px 12px', borderRadius: 8, background: 'var(--warn-soft)', fontSize: 12.5 }}>
            LOGBOX_SECRET_KEY is not set, so the password is encrypted with a key stored in the database. Set LOGBOX_SECRET_KEY in production.
          </div>
        )}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate(form);
          }}
        >
          <div style={{ padding: 18, display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 16 }}>
            <Field label="Host" hint={lockHint('host')}>
              <input className="input mono" value={form.host} disabled={locked('host')} onChange={(e) => setForm({ ...form, host: e.target.value })} placeholder="smtp.example.com" />
            </Field>
            <Field label="Port" hint={lockHint('port') ?? '587 for STARTTLS, 465 for implicit TLS'}>
              <input
                className="input mono"
                type="number"
                min={1}
                max={65535}
                value={form.port}
                disabled={locked('port')}
                onChange={(e) => setForm({ ...form, port: Number(e.target.value) })}
              />
            </Field>
            <Field label="Security" hint={lockHint('security')}>
              <Segmented
                small
                label="Security"
                value={form.security}
                onChange={(v) => {
                  if (!locked('security')) setForm({ ...form, security: v });
                }}
                options={[
                  { id: 'starttls', label: 'STARTTLS' },
                  { id: 'tls', label: 'implicit TLS' },
                  { id: 'none', label: 'none' },
                ]}
              />
            </Field>
            <Field label="Username" hint={lockHint('username')}>
              <input
                className="input mono"
                value={form.username}
                disabled={locked('username')}
                onChange={(e) => setForm({ ...form, username: e.target.value })}
                autoComplete="off"
              />
            </Field>
            <Field label="Password" hint={lockHint('password') ?? (s.passwordSet ? 'set (never shown); type to replace it' : 'not set')}>
              <input
                className="input"
                type="password"
                value={password}
                disabled={locked('password') || clearPassword}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={s.passwordSet ? '••••••••' : ''}
                autoComplete="new-password"
              />
              {s.passwordSet && !locked('password') && (
                <span className="row" style={{ fontSize: 12 }}>
                  <input type="checkbox" checked={clearPassword} onChange={(e) => setClearPassword(e.target.checked)} aria-label="Remove the stored password" />
                  remove the stored password
                </span>
              )}
            </Field>
            <Field label="From address" hint={lockHint('from')}>
              <input className="input mono" value={form.from} disabled={locked('from')} onChange={(e) => setForm({ ...form, from: e.target.value })} placeholder="logbox@example.com" />
            </Field>
            <Field label="From name" hint={lockHint('from_name')}>
              <input className="input" value={form.fromName} disabled={locked('from_name')} onChange={(e) => setForm({ ...form, fromName: e.target.value })} placeholder="Logbox" />
            </Field>
          </div>
          <div className="row" style={{ padding: '0 18px 18px' }}>
            <button type="submit" className="btn btn-primary" disabled={save.isPending}>
              Save email settings
            </button>
          </div>
        </form>
      </Panel>

      <Panel title="Send a test email" sub="shows exactly what the SMTP server answered">
        <form
          className="row wrap"
          style={{ padding: 18 }}
          onSubmit={(e) => {
            e.preventDefault();
            test.mutate();
          }}
        >
          <input className="input" type="email" value={to} onChange={(e) => setTo(e.target.value)} aria-label="Send to" style={{ maxWidth: 320 }} required />
          <button type="submit" className="btn" disabled={test.isPending}>
            {test.isPending ? 'Sending…' : 'Send test email'}
          </button>
        </form>
        {result && (
          <div
            role="status"
            className="mono"
            style={{
              margin: '0 18px 18px',
              padding: '10px 12px',
              borderRadius: 8,
              fontSize: 12.5,
              overflowWrap: 'anywhere',
              background: result.ok ? 'var(--ok-soft)' : 'var(--err-soft)',
              color: result.ok ? 'var(--text)' : 'var(--err-text)',
            }}
          >
            {result.ok ? `Sent. Server said: ${result.response ?? 'ok'}` : `Failed: ${result.error ?? 'unknown error'}`}
          </div>
        )}
      </Panel>
    </>
  );
}
