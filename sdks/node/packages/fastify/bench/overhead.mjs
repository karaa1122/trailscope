// Measures what the Fastify plugin adds per request, using `app.inject()`
// (in-process, no sockets) on two otherwise identical apps. Recording happens
// in `onResponse`, inside the measured time; redaction, serialization and gzip
// happen later, off-path. Run after `pnpm build`: node bench/overhead.mjs
import Fastify from 'fastify';

import { TrailscopeClient, trailscope } from '../dist/index.js';

const client = new TrailscopeClient({
  ingestKey: 'trs_ingest_bench',
  service: 'bench',
  flushIntervalMs: 3_600_000,
  flushAt: Number.MAX_SAFE_INTEGER,
  fetch: async () => new Response('{"accepted":0,"rejected":0}'),
});

async function build(withSdk) {
  const app = Fastify();
  if (withSdk) await app.register(trailscope, { client });
  app.post('/projects/:projectId/tasks', async (request, reply) => {
    if (request.query.fail) return reply.code(500).send({ error: 'internal error' });
    return [{ id: 't1', title: 'Write the brief', done: false }];
  });
  await app.ready();
  return app;
}

const headers = {
  'content-type': 'application/json',
  authorization: 'Bearer eyJhbGciOiJIUzI1NiJ9.bench',
  'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15',
};
const payload = JSON.stringify({ title: 'Draft roadmap', due_date: '2026-01-15', tags: ['a', 'b'] });

async function measure(app, n) {
  const samples = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const url = `/projects/prj_${i % 100}/tasks${i % 50 === 0 ? '?fail=1' : ''}`;
    const t = performance.now();
    await app.inject({ method: 'POST', url, headers, payload });
    samples[i] = performance.now() - t;
  }
  return samples.sort();
}

const N = 50_000;
const base = await build(false);
const sdk = await build(true);
await measure(base, 5_000); // warm up
await measure(sdk, 5_000);
const a = await measure(base, N);
const b = await measure(sdk, N);
const pct = (s, p) => s[Math.min(N - 1, Math.floor(N * p))];
const row = (s) => ({ p50_ms: +pct(s, 0.5).toFixed(4), p99_ms: +pct(s, 0.99).toFixed(4) });
const result = {
  iterations: N,
  without_plugin: row(a),
  with_plugin: row(b),
  added_p50_ms: +(pct(b, 0.5) - pct(a, 0.5)).toFixed(4),
  added_p99_ms: +(pct(b, 0.99) - pct(a, 0.99)).toFixed(4),
};
console.log(JSON.stringify(result, null, 2));
await Promise.all([base.close(), sdk.close()]);
await client.shutdown();
if (result.added_p99_ms >= 0.2) {
  console.error('p99 overhead is above the 0.2 ms budget');
  process.exitCode = 1;
}
