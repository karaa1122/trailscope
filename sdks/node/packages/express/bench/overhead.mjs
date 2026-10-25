// Measures what the Express middleware adds to the request path: context
// setup, request ID header, response body buffering and recording the
// request when the response closes (redaction, serialization and gzip happen
// later, off-path). Synthetic request/response objects keep network noise
// out; the response emits `close` synchronously so recording is timed too.
// Run after `pnpm build`: node bench/overhead.mjs
import { EventEmitter } from 'node:events';

import { LogboxClient, logbox } from '../dist/index.js';

const client = new LogboxClient({
  ingestKey: 'lbx_ingest_bench',
  service: 'bench',
  flushIntervalMs: 3_600_000,
  flushAt: Number.MAX_SAFE_INTEGER,
  fetch: async () => new Response('{"accepted":0,"rejected":0}'),
});
const middleware = logbox(client);

class FakeResponse extends EventEmitter {
  statusCode = 200;
  headersSent = false;
  writableFinished = false;
  #headers = {};
  setHeader(name, value) {
    this.#headers[name.toLowerCase()] = value;
    return this;
  }
  getHeader(name) {
    return this.#headers[name.toLowerCase()];
  }
  getHeaders() {
    return { ...this.#headers };
  }
  write() {
    return true;
  }
  end() {
    this.headersSent = true;
    this.writableFinished = true;
    this.emit('close');
    return this;
  }
}

const headers = {
  host: 'api.example.com',
  'content-type': 'application/json',
  authorization: 'Bearer eyJhbGciOiJIUzI1NiJ9.bench',
  'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15',
  accept: 'application/json',
  'content-length': '64',
};
const payload = JSON.stringify([{ id: 't1', title: 'Write the brief', done: false }]);

function request(i) {
  return {
    method: 'POST',
    url: `/projects/prj_${i % 100}/tasks?expand=owner`,
    originalUrl: `/projects/prj_${i % 100}/tasks?expand=owner`,
    headers,
    socket: { remoteAddress: '198.51.100.7' },
    ip: '198.51.100.7',
    baseUrl: '',
    route: { path: '/projects/:projectId/tasks' },
    body: { title: 'Draft roadmap', due_date: '2026-01-15', tags: ['a', 'b'] },
  };
}

// The route handler both variants run.
function handler(res, i) {
  res.statusCode = i % 50 === 0 ? 500 : 200;
  res.setHeader('content-type', 'application/json');
  res.end(payload);
}

function measure(withSdk, n) {
  const samples = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const req = request(i);
    const res = new FakeResponse();
    const t = performance.now();
    if (withSdk) middleware(req, res, () => handler(res, i));
    else handler(res, i);
    samples[i] = performance.now() - t;
  }
  return samples.sort();
}

const N = 200_000;
measure(true, 20_000); // warm up
measure(false, 20_000);
const base = measure(false, N);
const sdk = measure(true, N);
const pct = (s, p) => s[Math.min(N - 1, Math.floor(N * p))];
const row = (s) => ({ p50_ms: +pct(s, 0.5).toFixed(4), p99_ms: +pct(s, 0.99).toFixed(4), p999_ms: +pct(s, 0.999).toFixed(4) });
const result = {
  iterations: N,
  handler_only: row(base),
  with_middleware: row(sdk),
  added_p99_ms: +(pct(sdk, 0.99) - pct(base, 0.99)).toFixed(4),
  dropped: client.stats().dropped,
};
console.log(JSON.stringify(result, null, 2));
await client.shutdown();
if (result.added_p99_ms >= 0.2) {
  console.error('p99 overhead is above the 0.2 ms budget');
  process.exitCode = 1;
}
