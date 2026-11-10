import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useState, type FormEvent, type ReactNode } from 'react';

import { Logo } from '../components/icons';
import { Skeleton, Toast, copyText, useToast } from '../components/ui';
import { ApiError, api, liveUrl } from '../lib/api';
import { useSession } from '../lib/session';

export function AuthCard({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="auth-page">
      <div className="panel auth-card">
        <div className="logo" style={{ padding: 0 }}>
          <Logo />
          <span className="logo-word">trailscope</span>
        </div>
        <div>
          <h1 style={{ margin: '4px 0 6px', fontSize: 20 }}>{title}</h1>
          {subtitle && (
            <p className="muted" style={{ margin: 0, fontSize: 13, lineHeight: 1.6 }}>
              {subtitle}
            </p>
          )}
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
  // Single sign-on failures come back as /login?error=…
  const [error, setError] = useState<string | null>(() =>
    new URLSearchParams(window.location.search).get('error'),
  );
  const [busy, setBusy] = useState(false);
  const oidc = status.data?.oidc;

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
      <form
        onSubmit={(e) => void submit(e)}
        style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
      >
        <label className="field">
          Email
          <input
            className="input"
            type="email"
            autoComplete="username"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoFocus
          />
        </label>
        <label className="field">
          Password
          <input
            className="input"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
          />
        </label>
        {error && (
          <span className="err-text" role="alert" style={{ fontSize: 13 }}>
            {error}
          </span>
        )}
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
        {oidc?.enabled && (
          <a className="btn" href="/api/v1/auth/oidc/start" style={{ justifyContent: 'center' }}>
            Sign in with {oidc.displayName || 'single sign-on'}
          </a>
        )}
      </form>
    </AuthCard>
  );
}

export function InviteAccept({ token }: { token: string }) {
  const navigate = useNavigate();
  const { signedIn } = useSession();
  const invite = useQuery({
    queryKey: ['invite', token],
    queryFn: () => api.lookupInvite(token),
    retry: false,
  });
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
  if (invite.isLoading) {
    return (
      <AuthCard title="Join Trailscope">
        <Skeleton h={120} />
      </AuthCard>
    );
  }
  if (!invite.data) {
    return (
      <AuthCard
        title="This invite doesn't work"
        subtitle="It may have expired or been used already. Ask the person who invited you for a new link."
      >
        <a className="btn" href="/login">
          Go to sign-in
        </a>
      </AuthCard>
    );
  }
  const { email, role, organization, existingAccount } = invite.data;
  return (
    <AuthCard
      title={`Join ${organization}`}
      subtitle={
        existingAccount
          ? `You already have a Trailscope account as ${email}. Enter its password to join ${organization} as ${role}.`
          : `You've been invited to ${organization} as ${role}. Choose a name and a password to finish.`
      }
    >
      <form
        onSubmit={(e) => void submit(e)}
        style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
      >
        {!existingAccount && (
          <label className="field">
            Name
            <input
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </label>
        )}
        <label className="field">
          {existingAccount ? 'Password' : 'Password (at least 10 characters)'}
          <input
            className="input"
            type="password"
            autoComplete={existingAccount ? 'current-password' : 'new-password'}
            minLength={existingAccount ? undefined : 10}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoFocus={existingAccount}
          />
        </label>
        {error && (
          <span className="err-text" role="alert">
            {error}
          </span>
        )}
        <button type="submit" className="btn btn-primary">
          {existingAccount ? `Join ${organization}` : 'Create account'}
        </button>
      </form>
    </AuthCard>
  );
}

type Framework =
  'express' | 'fastify' | 'nestjs' | 'fastapi' | 'django' | 'laravel' | 'spring' | 'http';

const FRAMEWORKS: { id: Framework; label: string }[] = [
  { id: 'express', label: 'Express' },
  { id: 'fastify', label: 'Fastify' },
  { id: 'nestjs', label: 'NestJS' },
  { id: 'fastapi', label: 'FastAPI' },
  { id: 'django', label: 'Django' },
  { id: 'laravel', label: 'Laravel' },
  { id: 'spring', label: 'Spring Boot' },
  { id: 'http', label: 'HTTP' },
];

function snippet(fw: Framework, endpoint: string, key: string): string {
  const env = `TRAILSCOPE_ENDPOINT=${endpoint}\nTRAILSCOPE_INGEST_KEY=${key}\nTRAILSCOPE_SERVICE=api`;
  switch (fw) {
    case 'express':
      return `# npm install @trailscope/express\n# ${env.replace(/\n/g, '\n# ')}\nimport express from 'express';\nimport { trailscope } from '@trailscope/express';\n\nconst app = express();\napp.use(trailscope());   // first; reads TRAILSCOPE_* from the environment\napp.use(express.json());\napp.get('/users/:id', (req, res) => res.json({ id: req.params.id }));\napp.listen(3000);`;
    case 'fastify':
      return `# npm install @trailscope/fastify\nimport Fastify from 'fastify';\nimport trailscope from '@trailscope/fastify';\n\nconst app = Fastify();\nawait app.register(trailscope, { endpoint: '${endpoint}', ingestKey: '${key}', service: 'api' });\napp.get('/users/:id', async (req) => ({ id: req.params.id }));\nawait app.listen({ port: 3000 });`;
    case 'nestjs':
      return `// npm install @trailscope/nestjs\nimport { Module } from '@nestjs/common';\nimport { TrailscopeModule } from '@trailscope/nestjs';\n\n@Module({\n  imports: [TrailscopeModule.forRoot({ endpoint: '${endpoint}', ingestKey: '${key}', service: 'api' })],\n})\nexport class AppModule {}`;
    case 'fastapi':
      return `# pip install "trailscope[fastapi]"\nfrom fastapi import FastAPI\nimport trailscope\nfrom trailscope.asgi import TrailscopeMiddleware\n\ntrailscope.init(endpoint="${endpoint}", ingest_key="${key}", service="api")\napp = FastAPI()\napp.add_middleware(TrailscopeMiddleware)  # add it last, so it runs first`;
    case 'django':
      return `# pip install "trailscope[django]"\n# settings.py\nTRAILSCOPE = {"endpoint": "${endpoint}", "ingest_key": "${key}", "service": "api"}\nMIDDLEWARE = ["trailscope.django.TrailscopeMiddleware", *MIDDLEWARE]`;
    case 'laravel':
      return `# composer require trailscope/trailscope\n# .env\nTRAILSCOPE_ENDPOINT=${endpoint}\nTRAILSCOPE_INGEST_KEY=${key}\nTRAILSCOPE_SERVICE=api\n# optional: send log lines too\nLOG_STACK=single,trailscope`;
    case 'spring':
      return `<!-- pom.xml -->\n<dependency>\n  <groupId>pro.trailscope</groupId>\n  <artifactId>trailscope-spring-boot-starter</artifactId>\n  <version>0.1.0</version>\n</dependency>\n\n# application.properties\ntrailscope.endpoint=${endpoint}\ntrailscope.ingest-key=${key}\ntrailscope.service=api`;
    case 'http':
      return `curl -X POST ${endpoint}/v1/ingest \\\n  -H 'Authorization: Bearer ${key}' \\\n  -H 'Content-Type: application/x-ndjson' \\\n  --data-binary '{"type":"request","service":"api","method":"GET","path":"/hello","status":200,"durationMs":12}'`;
  }
}

export function Setup() {
  const navigate = useNavigate();
  const { signedIn, refresh } = useSession();
  const [step, setStep] = useState(0);
  const [form, setForm] = useState({
    email: '',
    name: '',
    password: '',
    organizationName: '',
    projectName: 'My API',
  });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ key: string; projectId: string } | null>(null);
  const [fw, setFw] = useState<Framework>('express');
  const [received, setReceived] = useState(false);
  const [toast, setToast] = useToast();
  const endpoint = window.location.origin;

  useEffect(() => {
    if (step !== 3 || !result) return;
    const es = new EventSource(liveUrl(result.projectId, '', 'sql', null), {
      withCredentials: true,
    });
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
      <AuthCard
        title="Set up Trailscope"
        subtitle="Create the owner account and your first project. You can invite your team afterwards."
      >
        {steps}
        <form
          onSubmit={(e) => void create(e)}
          style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
        >
          <label className="field">
            Your email
            <input
              className="input"
              type="email"
              autoComplete="username"
              value={form.email}
              onChange={(e) => setForm({ ...form, email: e.target.value })}
              required
              autoFocus
            />
          </label>
          <label className="field">
            Your name
            <input
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label className="field">
            Password (at least 10 characters)
            <input
              className="input"
              type="password"
              autoComplete="new-password"
              minLength={10}
              value={form.password}
              onChange={(e) => setForm({ ...form, password: e.target.value })}
              required
            />
          </label>
          <label className="field">
            Organization
            <input
              className="input"
              value={form.organizationName}
              onChange={(e) => setForm({ ...form, organizationName: e.target.value })}
              placeholder="Your company or team (optional)"
              maxLength={100}
            />
          </label>
          <label className="field">
            First project
            <input
              className="input"
              value={form.projectName}
              onChange={(e) => setForm({ ...form, projectName: e.target.value })}
              required
            />
          </label>
          {error && (
            <span className="err-text" role="alert">
              {error}
            </span>
          )}
          <button type="submit" className="btn btn-primary" disabled={busy}>
            {busy ? 'Creating…' : 'Create owner and project'}
          </button>
        </form>
      </AuthCard>
    );
  }
  if (step === 1 && result) {
    return (
      <AuthCard
        title="Your ingest key"
        subtitle="SDKs use this key to send events. It's shown only once; store it in your secrets manager now. You can create more keys later."
      >
        {steps}
        <div className="row" style={{ gap: 8 }}>
          <code
            className="input mono"
            style={{
              display: 'flex',
              alignItems: 'center',
              overflowX: 'auto',
              whiteSpace: 'nowrap',
            }}
          >
            {result.key}
          </code>
          <button
            type="button"
            className="btn"
            onClick={() =>
              void copyText(result.key).then((ok) => setToast(ok ? 'Key copied' : 'Copy failed'))
            }
          >
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
            Add the SDK for your framework. Requests start showing up in the live tail within a
            couple of seconds.
          </p>
          <div
            className="segmented"
            role="group"
            aria-label="Framework"
            style={{ flexWrap: 'wrap' }}
          >
            {FRAMEWORKS.map((f) => (
              <button
                key={f.id}
                type="button"
                aria-pressed={fw === f.id}
                onClick={() => setFw(f.id)}
              >
                {f.label}
              </button>
            ))}
          </div>
          <pre className="code panel" style={{ background: 'var(--sidebar)' }}>
            {snippet(fw, endpoint, result.key)}
          </pre>
          <div className="row">
            <button
              type="button"
              className="btn"
              onClick={() =>
                void copyText(snippet(fw, endpoint, result.key)).then((ok) =>
                  setToast(ok ? 'Copied' : 'Copy failed'),
                )
              }
            >
              Copy snippet
            </button>
            <button
              type="button"
              className="btn btn-primary"
              style={{ marginLeft: 'auto' }}
              onClick={() => setStep(3)}
            >
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
        <p style={{ margin: 0, fontSize: 14, color: 'var(--accent)' }}>
          ✓ Trailscope is receiving data from your service.
        </p>
      ) : (
        <p className="muted" style={{ margin: 0, fontSize: 13, lineHeight: 1.6 }}>
          Start your service with the SDK installed and make a request. No app handy? Generate demo
          traffic:
          <br />
          <code className="mono" style={{ color: 'var(--text)' }}>
            trailscope demo --url {endpoint} --key {result?.key.slice(0, 18)}…
          </code>
        </p>
      )}
      <button
        type="button"
        className={`btn${received ? ' btn-primary' : ''}`}
        onClick={() => void navigate({ to: '/live' })}
      >
        {received ? 'Open the live tail' : 'Skip for now'}
      </button>
    </AuthCard>
  );
}
