import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';

import { Logo } from '../components/icons';
import { Toast, copyText, useToast } from '../components/ui';
import { ApiError, api, liveUrl } from '../lib/api';
import { useSession } from '../lib/session';

function AuthCard({ title, subtitle, children }: { title: string; subtitle?: ReactNode; children: ReactNode }) {
  return (
    <div className="auth-page">
      <div className="panel auth-card">
        <div className="logo" style={{ padding: 0 }}>
          <Logo />
          <span className="logo-word">logbox</span>
        </div>
        <div>
          <h1 style={{ margin: '4px 0 6px', fontSize: 20 }}>{title}</h1>
          {subtitle && <p className="muted" style={{ margin: 0, fontSize: 13, lineHeight: 1.6 }}>{subtitle}</p>}
        </div>
        {children}
      </div>
    </div>
  );
}

function errMsg(e: unknown): string {
  return e instanceof ApiError ? e.message : 'Something went wrong. Is the server running?';
}

export function Login() {
  const navigate = useNavigate();
  const { signedIn } = useSession();
  const status = useQuery({ queryKey: ['setup'], queryFn: api.setupStatus });
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (status.data?.needsSetup) void navigate({ to: '/setup' });
  }, [status.data, navigate]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const r = await api.login(email, password);
      await signedIn(r.csrfToken);
      void navigate({ to: '/live' });
    } catch (err) {
      setError(errMsg(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <AuthCard title="Sign in">
      <form onSubmit={(e) => void submit(e)} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
        <label className="field">
          Email
          <input className="input" type="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
        </label>
        <label className="field">
          Password
          <input className="input" type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} required />
        </label>
        {error && (
          <span className="err-text" role="alert" style={{ fontSize: 13 }}>
            {error}
          </span>
        )}
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </AuthCard>
  );
}

export function InviteAccept({ token }: { token: string }) {
  const navigate = useNavigate();
  const { signedIn } = useSession();
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      const r = await api.acceptInvite(token, name, password);
      await signedIn(r.csrfToken);
      void navigate({ to: '/live' });
    } catch (err) {
      setError(errMsg(err));
    }
  };
  return (
    <AuthCard title="Join Logbox" subtitle="You've been invited. Choose a name and a password to finish.">
      <form onSubmit={(e) => void submit(e)} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
        <label className="field">
          Name
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        <label className="field">
          Password (at least 10 characters)
          <input className="input" type="password" autoComplete="new-password" minLength={10} value={password} onChange={(e) => setPassword(e.target.value)} required />
        </label>
        {error && <span className="err-text" role="alert">{error}</span>}
        <button type="submit" className="btn btn-primary">
          Create account
        </button>
      </form>
    </AuthCard>
  );
}

type Framework = 'express' | 'fastify' | 'nestjs' | 'fastapi' | 'django' | 'http';

function snippet(fw: Framework, endpoint: string, key: string): string {
  const env = `LOGBOX_ENDPOINT=${endpoint}\nLOGBOX_INGEST_KEY=${key}\nLOGBOX_SERVICE=api`;
  switch (fw) {
    case 'express':
      return `# npm install @logbox/express\n# ${env.replace(/\n/g, '\n# ')}\nimport express from 'express';\nimport { logbox } from '@logbox/express';\n\nconst app = express();\napp.use(express.json());\napp.use(logbox());   // reads LOGBOX_* from the environment\napp.get('/users/:id', (req, res) => res.json({ id: req.params.id }));\napp.listen(3000);`;
    case 'fastify':
      return `# npm install @logbox/fastify\nimport Fastify from 'fastify';\nimport logbox from '@logbox/fastify';\n\nconst app = Fastify();\nawait app.register(logbox, { endpoint: '${endpoint}', ingestKey: '${key}', service: 'api' });\napp.get('/users/:id', async (req) => ({ id: req.params.id }));\nawait app.listen({ port: 3000 });`;
    case 'nestjs':
      return `// npm install @logbox/nestjs\nimport { Module } from '@nestjs/common';\nimport { LogboxModule } from '@logbox/nestjs';\n\n@Module({\n  imports: [LogboxModule.forRoot({ endpoint: '${endpoint}', ingestKey: '${key}', service: 'api' })],\n})\nexport class AppModule {}`;
    case 'fastapi':
      return `# pip install "logbox[fastapi]"\nfrom fastapi import FastAPI\nimport logbox\nfrom logbox.starlette import LogboxMiddleware\n\nlogbox.init(endpoint="${endpoint}", ingest_key="${key}", service="api")\napp = FastAPI()\napp.add_middleware(LogboxMiddleware)`;
    case 'django':
      return `# pip install "logbox[django]"\n# settings.py\nLOGBOX = {"endpoint": "${endpoint}", "ingest_key": "${key}", "service": "api"}\nMIDDLEWARE = ["logbox.django.LogboxMiddleware", *MIDDLEWARE]`;
    case 'http':
      return `curl -X POST ${endpoint}/v1/ingest \\\n  -H 'Authorization: Bearer ${key}' \\\n  -H 'Content-Type: application/x-ndjson' \\\n  --data-binary '{"type":"request","service":"api","method":"GET","path":"/hello","status":200,"durationMs":12}'`;
  }
}

export function Setup() {
  const navigate = useNavigate();
  const { signedIn, refresh } = useSession();
  const [step, setStep] = useState(0);
  const [form, setForm] = useState({ email: '', name: '', password: '', projectName: 'My API' });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ key: string; projectId: string } | null>(null);
  const [fw, setFw] = useState<Framework>('express');
  const [received, setReceived] = useState(false);
  const [toast, setToast] = useToast();
  const endpoint = window.location.origin;

  useEffect(() => {
    if (step !== 3 || !result) return;
    const es = new EventSource(liveUrl(result.projectId, '', 'sql', null), { withCredentials: true });
    es.addEventListener('request', () => {
      setReceived(true);
      es.close();
    });
    return () => es.close();
  }, [step, result]);

  const create = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const r = await api.setup(form);
      await signedIn(r.csrfToken);
      await refresh();
      setResult({ key: r.ingestKey, projectId: r.projectId });
      setStep(1);
    } catch (err) {
      setError(errMsg(err));
    } finally {
      setBusy(false);
    }
  };

  const steps = (
    <div className="steps" aria-label={`Step ${step + 1} of 4`}>
      {[0, 1, 2, 3].map((i) => (
        <span key={i} className={i <= step ? 'on' : undefined} />
      ))}
    </div>
  );

  if (step === 0) {
    return (
      <AuthCard title="Set up Logbox" subtitle="Create the owner account and your first project. You can invite your team afterwards.">
        {steps}
        <form onSubmit={(e) => void create(e)} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          <label className="field">
            Your email
            <input className="input" type="email" autoComplete="username" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} required autoFocus />
          </label>
          <label className="field">
            Your name
            <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
          </label>
          <label className="field">
            Password (at least 10 characters)
            <input className="input" type="password" autoComplete="new-password" minLength={10} value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} required />
          </label>
          <label className="field">
            First project
            <input className="input" value={form.projectName} onChange={(e) => setForm({ ...form, projectName: e.target.value })} required />
          </label>
          {error && <span className="err-text" role="alert">{error}</span>}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? 'Creating…' : 'Create owner and project'}
          </button>
        </form>
      </AuthCard>
    );
  }
  if (step === 1 && result) {
    return (
      <AuthCard title="Your ingest key" subtitle="SDKs use this key to send events. It's shown only once; store it in your secrets manager now. You can create more keys later.">
        {steps}
        <div className="row" style={{ gap: 8 }}>
          <code className="input mono" style={{ display: 'flex', alignItems: 'center', overflowX: 'auto', whiteSpace: 'nowrap' }}>
            {result.key}
          </code>
          <button type="button" className="btn" onClick={() => void copyText(result.key).then((ok) => setToast(ok ? 'Key copied' : 'Copy failed'))}>
            Copy
          </button>
        </div>
        <button type="button" className="btn btn-primary" onClick={() => setStep(2)}>
          I've saved it
        </button>
        <Toast msg={toast} />
      </AuthCard>
    );
  }
  if (step === 2 && result) {
    return (
      <div className="auth-page">
        <div className="panel auth-card" style={{ width: 'min(760px, 100%)' }}>
          {steps}
          <h1 style={{ margin: 0, fontSize: 20 }}>Send your first event</h1>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            Add the SDK for your framework. Requests start showing up in the live tail within a couple of seconds.
          </p>
          <div className="segmented" role="group" aria-label="Framework" style={{ flexWrap: 'wrap' }}>
            {(['express', 'fastify', 'nestjs', 'fastapi', 'django', 'http'] as Framework[]).map((f) => (
              <button key={f} type="button" aria-pressed={fw === f} onClick={() => setFw(f)}>
                {f === 'http' ? 'HTTP' : f === 'nestjs' ? 'NestJS' : f === 'fastapi' ? 'FastAPI' : f[0]!.toUpperCase() + f.slice(1)}
              </button>
            ))}
          </div>
          <pre className="code panel" style={{ background: 'var(--sidebar)' }}>
            {snippet(fw, endpoint, result.key)}
          </pre>
          <div className="row">
            <button type="button" className="btn" onClick={() => void copyText(snippet(fw, endpoint, result.key)).then((ok) => setToast(ok ? 'Copied' : 'Copy failed'))}>
              Copy snippet
            </button>
            <button type="button" className="btn btn-primary" style={{ marginLeft: 'auto' }} onClick={() => setStep(3)}>
              Next
            </button>
          </div>
          <Toast msg={toast} />
        </div>
      </div>
    );
  }
  return (
    <AuthCard title={received ? 'First event received' : 'Waiting for the first event…'}>
      {steps}
      {received ? (
        <p style={{ margin: 0, fontSize: 14, color: 'var(--accent)' }}>✓ Logbox is receiving data from your service.</p>
      ) : (
        <p className="muted" style={{ margin: 0, fontSize: 13, lineHeight: 1.6 }}>
          Start your service with the SDK installed and make a request. No app handy? Generate demo traffic:
          <br />
          <code className="mono" style={{ color: 'var(--text)' }}>
            logbox demo --url {endpoint} --key {result?.key.slice(0, 18)}…
          </code>
        </p>
      )}
      <button type="button" className={`btn${received ? ' btn-primary' : ''}`} onClick={() => void navigate({ to: '/live' })}>
        {received ? 'Open the live tail' : 'Skip for now'}
      </button>
    </AuthCard>
  );
}
