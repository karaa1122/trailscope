import { gzip } from 'node:zlib';
import { promisify } from 'node:util';

import { type LogboxContext, currentContext, runWithContext } from './context.js';
import { acceptRequestId, newRequestId, newSpanId, newTraceId, parseTraceparent } from './ids.js';
import { type LogboxOptions, type ResolvedOptions, resolveOptions } from './options.js';
import { BoundedQueue } from './queue.js';
import { Redactor } from './redact.js';
import type {
  Attributes,
  EventItem,
  IngestResponse,
  Item,
  Level,
  RequestItem,
  SpanItem,
  SpanKind,
} from './types.js';

const gzipAsync = promisify(gzip);
export const SDK_VERSION = '0.1.0';

export type RequestInput = Omit<RequestItem, 'type' | 'service'> & { service?: string };
export type EventInput = Omit<EventItem, 'type' | 'service'> & { service?: string };
export type SpanInput = Omit<SpanItem, 'type' | 'service' | 'traceId' | 'spanId'> & {
  service?: string;
  traceId?: string;
  spanId?: string;
};

export interface ClientStats {
  queued: number;
  dropped: number;
  sent: number;
  rejected: number;
  failedBatches: number;
}

type SendResult = { kind: 'ok' } | { kind: 'retry'; afterMs?: number } | { kind: 'drop' };

export interface JobOptions {
  /** 1-based attempt number (BullMQ: `job.attemptsMade + 1`). */
  attempt?: number;
  /** Context to restore (e.g. one serialized into job data with `serializeContext`). */
  context?: Partial<LogboxContext>;
  attributes?: Attributes;
}

/**
 * The Logbox client. Every capture method is synchronous and O(1): items are
 * appended to a bounded in-memory queue and a background timer ships them.
 * Redaction, serialization and compression happen during the flush, off the
 * request path.
 */
export class LogboxClient {
  readonly options: ResolvedOptions;
  readonly redactor: Redactor;
  private readonly queue: BoundedQueue<Item>;
  private timer: ReturnType<typeof setInterval> | undefined;
  private flushing: Promise<void> | undefined;
  private flushScheduled = false;
  private backoffUntil = 0;
  private failures = 0;
  private closed = false;
  private warnedAuth = false;
  private readonly counters = { sent: 0, rejected: 0, failedBatches: 0 };

  constructor(options: LogboxOptions = {}) {
    this.options = resolveOptions(options);
    this.redactor = new Redactor(this.options.redact);
    this.queue = new BoundedQueue<Item>(this.options.maxQueueSize);
    if (this.options.enabled) {
      if (!this.options.ingestKey) {
        this.options.log('no ingest key configured (LOGBOX_INGEST_KEY); events will not be sent');
      }
      this.timer = setInterval(() => void this.flush(), this.options.flushIntervalMs);
      this.timer.unref?.();
    }
  }

  // ---- capture ------------------------------------------------------------

  captureRequest(input: RequestInput): void {
    this.enqueue({ ...input, type: 'request', service: input.service ?? this.options.service });
  }

  /** Records an event; IDs, user and correlation default to the current context. */
  captureEvent(input: EventInput): void {
    const ctx = currentContext();
    const correlation =
      input.correlation ?? (ctx && Object.keys(ctx.correlation).length ? { ...ctx.correlation } : undefined);
    this.enqueue({
      ...input,
      type: 'event',
      service: input.service ?? this.options.service,
      requestId: input.requestId ?? ctx?.requestId,
      traceId: input.traceId ?? ctx?.traceId,
      userId: input.userId ?? ctx?.userId,
      correlation,
    });
  }

  captureSpan(input: SpanInput): void {
    const ctx = currentContext();
    const span: SpanItem = {
      ...input,
      type: 'span',
      service: input.service ?? this.options.service,
      traceId: input.traceId ?? ctx?.traceId ?? newTraceId(),
      spanId: input.spanId ?? newSpanId(),
      parentSpanId: input.parentSpanId ?? ctx?.spanId,
      requestId: input.requestId ?? ctx?.requestId,
    };
    this.enqueue(span);
  }

  /** An application log line, linked to the current request if there is one. */
  log(level: Level, message: string, fields: { name?: string; attributes?: Attributes; error?: unknown } = {}): void {
    const err = fields.error instanceof Error ? fields.error : undefined;
    this.captureEvent({
      kind: 'log',
      name: fields.name ?? 'log',
      level,
      message,
      attributes: fields.attributes,
      errorType: err?.name,
      errorMessage: err?.message,
      errorStack: err?.stack,
    });
  }

  /** Reports an exception as an error event (outside an HTTP request). */
  captureException(error: unknown, fields: { name?: string; attributes?: Attributes } = {}): void {
    const e = toError(error);
    this.captureEvent({
      kind: 'log',
      name: fields.name ?? 'exception',
      level: 'error',
      status: 'error',
      message: e.message,
      errorType: e.name,
      errorMessage: e.message,
      errorStack: e.stack,
      attributes: fields.attributes,
    });
  }

  /** Times `fn` as a span of the current request. */
  async span<T>(kind: SpanKind, name: string, fn: () => Promise<T> | T, attributes?: Attributes): Promise<T> {
    const start = Date.now();
    const t0 = performance.now();
    let status: 'ok' | 'error' = 'ok';
    try {
      return await fn();
    } catch (e) {
      status = 'error';
      throw e;
    } finally {
      this.captureSpan({ kind, name, start, durationMs: round(performance.now() - t0), status, attributes });
    }
  }

  /**
   * Runs a background job inside a Logbox context and records it as a
   * `kind=job` event with its duration, outcome and attempt number.
   */
  async trackJob<T>(name: string, fn: () => Promise<T> | T, opts: JobOptions = {}): Promise<T> {
    const parent = opts.context;
    const ctx: LogboxContext = {
      requestId: parent?.requestId ?? newRequestId(),
      traceId: parent?.traceId ?? newTraceId(),
      spanId: newSpanId(),
      parentSpanId: parent?.spanId,
      userId: parent?.userId,
      correlation: { ...(parent?.correlation ?? {}) },
      attributes: { ...(parent?.attributes ?? {}) },
    };
    return runWithContext(ctx, async () => {
      const timestamp = Date.now();
      const t0 = performance.now();
      try {
        const result = await fn();
        this.captureEvent({
          kind: 'job',
          name,
          timestamp,
          level: 'info',
          status: 'ok',
          message: `${name} completed`,
          durationMs: round(performance.now() - t0),
          attempt: opts.attempt,
          attributes: opts.attributes,
          userId: ctx.userId,
          correlation: Object.keys(ctx.correlation).length ? { ...ctx.correlation } : undefined,
        });
        return result;
      } catch (error) {
        const e = toError(error);
        this.captureEvent({
          kind: 'job',
          name,
          timestamp,
          level: 'error',
          status: 'error',
          message: `${name} failed: ${e.message}`,
          durationMs: round(performance.now() - t0),
          attempt: opts.attempt,
          attributes: opts.attributes,
          errorType: e.name,
          errorMessage: e.message,
          errorStack: e.stack,
          userId: ctx.userId,
          correlation: Object.keys(ctx.correlation).length ? { ...ctx.correlation } : undefined,
        });
        throw error;
      }
    });
  }

  // ---- helpers for framework integrations -------------------------------------

  /** Builds the context for an incoming request from its headers. */
  contextFromHeaders(get: (name: string) => string | undefined): LogboxContext {
    const tp = parseTraceparent(get('traceparent'));
    return {
      requestId: acceptRequestId(get(this.options.requestIdHeader)) ?? newRequestId(),
      traceId: tp?.traceId ?? newTraceId(),
      spanId: newSpanId(),
      parentSpanId: tp?.parentSpanId,
      correlation: {},
      attributes: {},
    };
  }

  isIgnored(routeOrPath: string | undefined): boolean {
    if (!routeOrPath) return false;
    return this.options.ignoreRoutes.some((r) =>
      typeof r === 'string' ? r === routeOrPath : r.test(routeOrPath),
    );
  }

  /**
   * Sampling decision: returns the sample rate to record when the request is
   * kept, or 0 when it is dropped. Errors (status >= 400) and slow requests
   * are always kept.
   */
  sample(route: string, status: number, durationMs: number, hasError = false): number {
    if (status >= 400 || hasError || durationMs >= this.options.slowMs) return 1;
    const rate = this.options.routeSampleRates[route] ?? this.options.sampleRate;
    if (rate >= 1) return 1;
    return Math.random() < rate ? rate : 0;
  }

  // ---- lifecycle --------------------------------------------------------

  stats(): ClientStats {
    return { queued: this.queue.length, dropped: this.queue.dropped, ...this.counters };
  }

  /** Sends everything queued. Concurrent calls share one flush. */
  flush(): Promise<void> {
    if (!this.options.enabled) return Promise.resolve();
    if (!this.flushing) {
      this.flushing = this.drain().finally(() => {
        this.flushing = undefined;
      });
    }
    return this.flushing;
  }

  /** Stops the timer and flushes what's left, waiting at most `shutdownTimeoutMs`. */
  async shutdown(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    if (this.timer) clearInterval(this.timer);
    this.backoffUntil = 0;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([
      this.flush(),
      new Promise<void>((resolve) => {
        timeout = setTimeout(resolve, this.options.shutdownTimeoutMs);
        timeout.unref?.();
      }),
    ]);
    if (timeout) clearTimeout(timeout);
  }

  private enqueue(item: Item): void {
    if (!this.options.enabled || this.closed) return;
    if (item.type !== 'span' && !item.environment) item.environment = this.options.environment;
    if (item.type !== 'span' && !item.release && this.options.release) item.release = this.options.release;
    if (item.type !== 'span' && item.timestamp === undefined) item.timestamp = Date.now();
    this.queue.push(item);
    if (this.queue.length >= this.options.flushAt && !this.flushScheduled) {
      this.flushScheduled = true;
      setImmediate(() => {
        this.flushScheduled = false;
        void this.flush();
      });
    }
  }

  private async drain(): Promise<void> {
    while (this.queue.length > 0) {
      if (!this.options.ingestKey) {
        this.queue.take(this.queue.length);
        return;
      }
      if (Date.now() < this.backoffUntil) return;
      const batch = this.queue.take(this.options.maxBatchSize);
      const prepared = this.prepare(batch);
      if (prepared.length === 0) continue;
      const result = await this.send(prepared);
      if (result.kind === 'ok') {
        this.failures = 0;
        continue;
      }
      if (result.kind === 'drop') continue;
      this.counters.failedBatches++;
      this.failures++;
      this.queue.unshift(batch);
      const backoff = Math.min(60_000, 1000 * 2 ** (this.failures - 1));
      const jittered = backoff / 2 + Math.random() * (backoff / 2);
      this.backoffUntil = Date.now() + Math.max(result.afterMs ?? 0, jittered);
      this.options.log(`flush failed; retrying in ${Math.round(this.backoffUntil - Date.now())} ms`);
      return;
    }
  }

  /** Redaction and `beforeSend`, applied off the request path. */
  private prepare(batch: Item[]): Item[] {
    const out: Item[] = [];
    const r = this.redactor;
    for (const original of batch) {
      let item: Item | null = original;
      if (item.type === 'request') {
        const ct = (h: Record<string, string> | undefined) => h?.['content-type'] ?? '';
        item = {
          ...item,
          reqHeaders: item.reqHeaders ? r.redactHeaders(item.reqHeaders) : undefined,
          resHeaders: item.resHeaders ? r.redactHeaders(item.resHeaders) : undefined,
          reqBody: item.reqBody ? r.redactBody(item.reqBody, ct(item.reqHeaders)) : undefined,
          resBody: item.resBody ? r.redactBody(item.resBody, ct(item.resHeaders)) : undefined,
          query: item.query ? r.redactQuery(item.query) : item.query,
          ip: r.ip(item.ip),
          errorMessage: item.errorMessage ? r.applyPatterns(item.errorMessage) : undefined,
        };
      } else if (item.type === 'event') {
        item = {
          ...item,
          message: item.message ? r.applyPatterns(item.message) : item.message,
          errorMessage: item.errorMessage ? r.applyPatterns(item.errorMessage) : undefined,
        };
      }
      if (this.options.beforeSend) {
        try {
          item = this.options.beforeSend(item);
        } catch (e) {
          this.options.log(`beforeSend threw: ${String(e)}`);
        }
      }
      if (item) out.push(item);
    }
    return out;
  }

  private async send(items: Item[]): Promise<SendResult> {
    let body: Buffer;
    try {
      body = await gzipAsync(Buffer.from(items.map((i) => JSON.stringify(i)).join('\n')));
    } catch (e) {
      this.options.log(`could not serialize batch: ${String(e)}`);
      return { kind: 'drop' };
    }
    let res: Response;
    try {
      res = await this.options.fetch(`${this.options.endpoint}/v1/ingest`, {
        method: 'POST',
        headers: {
          authorization: `Bearer ${this.options.ingestKey}`,
          'content-type': 'application/x-ndjson',
          'content-encoding': 'gzip',
          'user-agent': `logbox-node/${SDK_VERSION}`,
        },
        body: new Uint8Array(body),
        signal: AbortSignal.timeout(this.options.requestTimeoutMs),
      });
    } catch (e) {
      this.options.log(`network error: ${String(e)}`);
      return { kind: 'retry' };
    }
    if (res.status === 200 || res.status === 207) {
      const parsed = (await res.json().catch(() => undefined)) as IngestResponse | undefined;
      this.counters.sent += parsed?.accepted ?? items.length;
      if (parsed?.rejected) {
        this.counters.rejected += parsed.rejected;
        this.options.log(`server rejected ${parsed.rejected} items: ${parsed.errors?.[0]?.message ?? ''}`);
      }
      return { kind: 'ok' };
    }
    await res.body?.cancel().catch(() => undefined);
    if (res.status === 429 || res.status === 503 || res.status >= 500) {
      const ra = Number(res.headers.get('retry-after'));
      return { kind: 'retry', afterMs: Number.isFinite(ra) && ra > 0 ? ra * 1000 : undefined };
    }
    if ((res.status === 401 || res.status === 403) && !this.warnedAuth) {
      this.warnedAuth = true;
      console.warn('[logbox] ingest key rejected (401); check LOGBOX_INGEST_KEY');
    }
    this.counters.rejected += items.length;
    this.options.log(`batch dropped: HTTP ${res.status}`);
    return { kind: 'drop' };
  }
}

export function toError(e: unknown): Error {
  if (e instanceof Error) return e;
  return new Error(typeof e === 'string' ? e : JSON.stringify(e));
}

function round(ms: number): number {
  return Math.round(ms * 100) / 100;
}

let globalClient: LogboxClient | undefined;

/** Creates the process-wide client used by the framework integrations. */
export function init(options: LogboxOptions = {}): LogboxClient {
  if (globalClient) void globalClient.shutdown();
  globalClient = new LogboxClient(options);
  const client = globalClient;
  process.once('beforeExit', () => void client.shutdown());
  return client;
}

export function getClient(): LogboxClient | undefined {
  return globalClient;
}
