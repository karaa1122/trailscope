import { gunzipSync } from 'node:zlib';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  BoundedQueue,
  LogboxClient,
  REDACTED,
  Redactor,
  currentContext,
  deserializeContext,
  formatTraceparent,
  maskIp,
  parseTraceparent,
  resolveOptions,
  runWithContext,
  serializeBody,
  serializeContext,
  setCorrelation,
  setUser,
  shouldCaptureBodies,
  type Item,
  type LogboxContext,
} from '../src/index.js';

interface Call {
  url: string;
  headers: Record<string, string>;
  items: Item[];
}

function fakeFetch(responses: Array<{ status: number; headers?: Record<string, string>; body?: unknown }> = []) {
  const calls: Call[] = [];
  const fn = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    const raw = gunzipSync(init?.body as Buffer).toString('utf8');
    const items = raw.split('\n').map((l) => JSON.parse(l) as Item);
    calls.push({ url: String(url), headers: init?.headers as Record<string, string>, items });
    const next = responses.shift() ?? { status: 200 };
    const body = next.body ?? { accepted: items.length, rejected: 0 };
    return new Response(JSON.stringify(body), { status: next.status, headers: next.headers });
  });
  return { fn: fn as unknown as typeof fetch, calls };
}

function client(fetchImpl: typeof fetch, extra: Partial<ConstructorParameters<typeof LogboxClient>[0]> = {}) {
  return new LogboxClient({
    endpoint: 'http://logbox.test/',
    ingestKey: 'lbx_ingest_test',
    service: 'api',
    environment: 'test',
    release: 'v1',
    flushIntervalMs: 60_000,
    fetch: fetchImpl,
    ...extra,
  });
}

const req = {
  method: 'GET',
  route: '/users/:id',
  path: '/users/1',
  status: 200,
  durationMs: 3,
};

afterEach(() => {
  vi.useRealTimers();
});

describe('BoundedQueue', () => {
  it('drops the oldest when full and counts drops', () => {
    const q = new BoundedQueue<number>(3);
    [1, 2, 3, 4, 5].forEach((n) => q.push(n));
    expect(q.length).toBe(3);
    expect(q.dropped).toBe(2);
    expect(q.take(10)).toEqual([3, 4, 5]);
  });

  it('puts failed items back in order, dropping what does not fit', () => {
    const q = new BoundedQueue<number>(4);
    [1, 2, 3].forEach((n) => q.push(n));
    const batch = q.take(2);
    q.push(9);
    q.push(10);
    q.unshift(batch);
    expect(q.take(10)).toEqual([2, 3, 9, 10]);
    expect(q.dropped).toBe(1);
  });
});

describe('redaction', () => {
  const r = new Redactor(resolveOptions({ redact: { keys: ['dob', 'user.profile.nickname'], patterns: [/\d{4}-\d{4}/] } }).redact);

  it('redacts default and custom keys at any depth, keeping order', () => {
    const out = JSON.parse(
      r.redactBody(
        JSON.stringify({
          email: 'a@example.com',
          Password: 'x',
          nested: { accessToken: 't', list: [{ CVV: '1' }] },
          user: { profile: { nickname: 'n', name: 'keep' } },
          dob: '2000',
          note: 'call 1234-5678',
          pinned: true,
        }),
        'application/json',
      ),
    );
    expect(out.Password).toBe(REDACTED);
    expect(out.nested.accessToken).toBe(REDACTED);
    expect(out.nested.list[0].CVV).toBe(REDACTED);
    expect(out.user.profile).toEqual({ nickname: REDACTED, name: 'keep' });
    expect(out.dob).toBe(REDACTED);
    expect(out.note).toBe(`call ${REDACTED}`);
    expect(out.pinned).toBe(true);
    expect(Object.keys(out)[0]).toBe('email');
  });

  it('handles truncated JSON, forms, queries and headers', () => {
    expect(r.redactBody('{"password": "hunter2", "a":', 'application/json')).not.toContain('hunter2');
    expect(r.redactBody('user=a&token=b', 'application/x-www-form-urlencoded')).toBe(`user=a&token=${REDACTED}`);
    expect(r.redactQuery('?page=1&access_token=x')).toBe(`page=1&access_token=${REDACTED}`);
    expect(r.redactHeaders({ Authorization: 'Bearer x', Accept: '*/*' })).toEqual({
      authorization: REDACTED,
      accept: '*/*',
    });
  });

  it('masks IPs', () => {
    expect(maskIp('203.0.113.42')).toBe('203.0.113.0');
    expect(maskIp('::ffff:10.1.2.3')).toBe('10.1.2.0');
    expect(maskIp('2001:db8:abcd:12::1')).toBe('2001:db8:abcd::');
  });
});

describe('ids and context', () => {
  it('parses and formats traceparent', () => {
    const tp = parseTraceparent('00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01');
    expect(tp).toEqual({ traceId: '4bf92f3577b34da6a3ce929d0e0e4736', parentSpanId: '00f067aa0ba902b7', sampled: true });
    expect(parseTraceparent('00-00000000000000000000000000000000-00f067aa0ba902b7-01')).toBeUndefined();
    expect(parseTraceparent('garbage')).toBeUndefined();
    expect(formatTraceparent('a'.repeat(32), 'b'.repeat(16))).toBe(`00-${'a'.repeat(32)}-${'b'.repeat(16)}-01`);
  });

  it('carries context across async boundaries and serializes for jobs', async () => {
    const c = client(fakeFetch().fn);
    const ctx = c.contextFromHeaders((n) => (n === 'x-request-id' ? 'req-1' : undefined));
    await runWithContext(ctx, async () => {
      await new Promise((r) => setTimeout(r, 1));
      setUser(42);
      setCorrelation('account_id', 'acc_1');
      expect(currentContext()?.userId).toBe('42');
      const s = serializeContext();
      expect(s).toMatchObject({ requestId: 'req-1', userId: '42', correlation: { account_id: 'acc_1' } });
      expect(deserializeContext(JSON.parse(JSON.stringify(s)))).toMatchObject({ requestId: 'req-1' });
    });
    expect(currentContext()).toBeUndefined();
  });
});

describe('capture helpers', () => {
  it('decides body capture by policy', () => {
    expect(shouldCaptureBodies('errors', 200, 10, 1000)).toBe(false);
    expect(shouldCaptureBodies('errors', 500, 10, 1000)).toBe(true);
    expect(shouldCaptureBodies('errors', 200, 1500, 1000)).toBe(true);
    expect(shouldCaptureBodies('never', 500, 10, 1000)).toBe(false);
    expect(shouldCaptureBodies('always', 200, 1, 1000)).toBe(true);
  });

  it('serializes, truncates and skips binary bodies', () => {
    expect(serializeBody({ a: 1 }, 'application/json', 100)).toEqual({ body: '{"a":1}', truncated: false });
    expect(serializeBody('x'.repeat(20), 'text/plain', 8)).toEqual({ body: 'x'.repeat(8), truncated: true });
    expect(serializeBody('é'.repeat(10), 'text/plain', 5)?.body).toBe('éé');
    expect(serializeBody('data', 'multipart/form-data; boundary=x', 100)).toBeUndefined();
    expect(serializeBody(Buffer.from([0, 1, 2]), undefined, 100)).toBeUndefined();
  });
});

describe('LogboxClient', () => {
  it('batches, gzips, redacts at flush and stamps environment/release', async () => {
    const f = fakeFetch();
    const c = client(f.fn);
    c.captureRequest({
      ...req,
      reqHeaders: { authorization: 'Bearer x', 'content-type': 'application/json' },
      reqBody: '{"password":"p","q":1}',
      ip: '198.51.100.7',
    });
    c.captureEvent({ kind: 'log', name: 'app', message: 'hello' });
    await c.flush();
    expect(f.calls).toHaveLength(1);
    const call = f.calls[0];
    if (!call) throw new Error("no request sent");
    expect(call.url).toBe('http://logbox.test/v1/ingest');
    expect(call.headers.authorization).toBe('Bearer lbx_ingest_test');
    expect(call.headers['content-encoding']).toBe('gzip');
    const [r, e] = call.items;
    expect(r).toMatchObject({
      type: 'request',
      service: 'api',
      environment: 'test',
      release: 'v1',
      reqBody: `{"password":"${REDACTED}","q":1}`,
      ip: '198.51.100.0',
    });
    expect(r?.type === 'request' && r.reqHeaders?.authorization).toBe(REDACTED);
    expect(e).toMatchObject({ type: 'event', name: 'app' });
    expect(c.stats()).toMatchObject({ queued: 0, sent: 2 });
  });

  it('flushes early at flushAt and splits by maxBatchSize', async () => {
    const f = fakeFetch();
    const c = client(f.fn, { flushAt: 5, maxBatchSize: 2 });
    for (let i = 0; i < 5; i++) c.captureRequest(req);
    await new Promise((r) => setImmediate(r));
    await c.flush();
    expect(f.calls.map((x) => x.items.length)).toEqual([2, 2, 1]);
  });

  it('honors 429 Retry-After and keeps the batch', async () => {
    const f = fakeFetch([{ status: 429, headers: { 'retry-after': '3' } }]);
    const c = client(f.fn);
    c.captureRequest(req);
    await c.flush();
    expect(c.stats()).toMatchObject({ queued: 1, failedBatches: 1 });
    // Still backing off: no new request.
    await c.flush();
    expect(f.calls).toHaveLength(1);
    // Shutdown ignores the backoff and delivers.
    await c.shutdown();
    expect(f.calls).toHaveLength(2);
    expect(c.stats().queued).toBe(0);
  });

  it('drops batches the server refuses and never throws', async () => {
    const f = fakeFetch([{ status: 400, body: { error: 'bad' } }]);
    const c = client(f.fn);
    c.captureRequest(req);
    await expect(c.flush()).resolves.toBeUndefined();
    expect(c.stats()).toMatchObject({ queued: 0, rejected: 1 });
  });

  it('survives network errors with backoff', async () => {
    const fetchImpl = vi.fn(async () => {
      throw new Error('ECONNREFUSED');
    }) as unknown as typeof fetch;
    const c = client(fetchImpl);
    c.captureRequest(req);
    await c.flush();
    expect(c.stats()).toMatchObject({ queued: 1, failedBatches: 1 });
  });

  it('bounds memory when the server is unreachable', () => {
    const c = client(fakeFetch().fn, { maxQueueSize: 10, flushAt: 1_000 });
    for (let i = 0; i < 25; i++) c.captureRequest(req);
    expect(c.stats()).toMatchObject({ queued: 10, dropped: 15 });
  });

  it('applies beforeSend and can drop items', async () => {
    const f = fakeFetch();
    const c = client(f.fn, {
      beforeSend: (item) => (item.type === 'request' && item.route === '/drop' ? null : item),
    });
    c.captureRequest({ ...req, route: '/drop' });
    c.captureRequest(req);
    await c.flush();
    expect(f.calls[0]?.items).toHaveLength(1);
  });

  it('samples healthy requests but keeps errors and slow ones', () => {
    const c = client(fakeFetch().fn, { sampleRate: 0, routeSampleRates: { '/hot': 1 } });
    expect(c.sample('/x', 200, 5)).toBe(0);
    expect(c.sample('/x', 500, 5)).toBe(1);
    expect(c.sample('/x', 200, 5000)).toBe(1);
    expect(c.sample('/hot', 200, 5)).toBe(1);
    expect(c.isIgnored('/healthz')).toBe(true);
    expect(c.isIgnored('/users')).toBe(false);
  });

  it('tracks jobs with context, attempts and errors', async () => {
    const f = fakeFetch();
    const c = client(f.fn);
    const parent: Partial<LogboxContext> = { requestId: 'req-9', traceId: 't'.repeat(32), correlation: { account_id: 'a1' } };
    await c.trackJob('email.send', async () => {
      c.log('info', 'sending');
    }, { attempt: 1, context: parent });
    await expect(
      c.trackJob('email.send', async () => {
        throw new TypeError('boom');
      }, { attempt: 2, context: parent }),
    ).rejects.toThrow('boom');
    await c.flush();
    const items = f.calls.flatMap((x) => x.items);
    expect(items).toHaveLength(3);
    expect(items[0]).toMatchObject({ kind: 'log', requestId: 'req-9', correlation: { account_id: 'a1' } });
    expect(items[1]).toMatchObject({ kind: 'job', status: 'ok', attempt: 1, requestId: 'req-9' });
    expect(items[2]).toMatchObject({ kind: 'job', status: 'error', attempt: 2, errorType: 'TypeError' });
  });

  it('is a no-op when disabled', async () => {
    const f = fakeFetch();
    const c = client(f.fn, { enabled: false });
    c.captureRequest(req);
    await c.flush();
    expect(f.calls).toHaveLength(0);
    expect(c.stats().queued).toBe(0);
  });
});
