// Measures what the SDK adds to the request path: building the event and
// enqueueing it (redaction, serialization and gzip happen later, off-path).
// Run after `pnpm build`: node bench/overhead.mjs
import { LogboxClient, serializeBody, shouldCaptureBodies } from '../dist/index.js';

const client = new LogboxClient({
  ingestKey: 'lbx_ingest_bench',
  service: 'bench',
  flushIntervalMs: 3_600_000,
  maxQueueSize: 10_000,
  flushAt: Number.MAX_SAFE_INTEGER,
  // Never actually send: the benchmark is about the request path.
  fetch: async () => new Response('{"accepted":0,"rejected":0}'),
});

const headers = {
  'content-type': 'application/json',
  authorization: 'Bearer eyJhbGciOiJIUzI1NiJ9.bench',
  'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15',
  'x-request-id': 'req_0123456789abcdef',
  accept: 'application/json',
};
const body = { title: 'Draft roadmap', due_date: '2026-01-15', tags: ['a', 'b', 'c'], password: 'x' };

function oneRequest(i) {
  const status = i % 50 === 0 ? 500 : 200;
  const durationMs = 12.5;
  const capture = shouldCaptureBodies('errors', status, durationMs, 1000);
  const req = capture ? serializeBody(body, 'application/json', 8192) : undefined;
  client.captureRequest({
    method: 'POST',
    route: '/projects/:id/tasks',
    path: `/projects/prj_${i % 100}/tasks`,
    status,
    durationMs,
    userId: `u_${i % 1000}`,
    ip: '198.51.100.7',
    reqHeaders: headers,
    resHeaders: { 'content-type': 'application/json' },
    reqBody: req?.body,
    bodyTruncated: req?.truncated,
    requestId: `req_${i}`,
    traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
    spanId: '00f067aa0ba902b7',
    correlation: { project_id: `prj_${i % 100}` },
  });
}

const N = 200_000;
for (let i = 0; i < 20_000; i++) oneRequest(i); // warm up
const samples = new Float64Array(N);
for (let i = 0; i < N; i++) {
  const t = performance.now();
  oneRequest(i);
  samples[i] = performance.now() - t;
}
samples.sort();
const pct = (p) => samples[Math.min(N - 1, Math.floor(N * p))];
const result = {
  iterations: N,
  p50_ms: +pct(0.5).toFixed(4),
  p99_ms: +pct(0.99).toFixed(4),
  p999_ms: +pct(0.999).toFixed(4),
  max_ms: +samples[N - 1].toFixed(4),
  dropped: client.stats().dropped,
};
console.log(JSON.stringify(result, null, 2));
await client.shutdown();
if (result.p99_ms >= 0.2) {
  console.error('p99 overhead is above the 0.2 ms budget');
  process.exitCode = 1;
}
