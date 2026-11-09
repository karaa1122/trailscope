import type { IncomingMessage, ServerResponse } from 'node:http';

import type { TrailscopeClient } from './client.js';
import { type TrailscopeContext, runWithContext } from './context.js';

type HeaderBag = Record<string, string | string[] | number | undefined>;

/** One finished HTTP exchange, as framework integrations report it. */
export interface HttpExchange {
  /** Epoch milliseconds when the request started. */
  timestamp: number;
  durationMs: number;
  method: string;
  /** Path with the query string, e.g. `/orders/42?expand=items`. */
  url: string;
  /** Route pattern, e.g. `/orders/:id`. Falls back to the context's route. */
  route?: string;
  status: number;
  reqHeaders?: HeaderBag;
  resHeaders?: HeaderBag;
  reqBody?: unknown;
  resBody?: unknown;
  ip?: string;
  reqSize?: number;
  resSize?: number;
  /** Falls back to the context's error. */
  error?: unknown;
}

export interface ObserveOptions {
  /** The matched route pattern, read when the response closes. */
  route?: () => string | undefined;
  /** The parsed request body, read when the response closes. */
  body?: () => unknown;
  /** The client IP. Default: the first `x-forwarded-for` hop, then the socket address. */
  ip?: () => string | undefined;
}

const CONTEXT = Symbol.for('trailscope.context');

/** The Trailscope context an integration attached to a request object. */
export function contextOf(req: object | undefined): TrailscopeContext | undefined {
  return req ? (req as { [CONTEXT]?: TrailscopeContext })[CONTEXT] : undefined;
}

export function attachContext(req: object, ctx: TrailscopeContext): void {
  (req as { [CONTEXT]?: TrailscopeContext })[CONTEXT] = ctx;
}

function first(v: string | string[] | undefined): string | undefined {
  return Array.isArray(v) ? v[0] : v;
}

/** First `x-forwarded-for` hop, `x-real-ip`, then the socket address. */
export function clientIp(req: IncomingMessage): string | undefined {
  const xff = first(req.headers['x-forwarded-for']);
  if (xff) return xff.split(',')[0]?.trim() || undefined;
  return first(req.headers['x-real-ip']) ?? req.socket?.remoteAddress;
}

/**
 * Instruments one Node.js HTTP exchange: builds the context from the request
 * headers (`x-request-id`, `traceparent`), returns the request ID in the
 * response, keeps the first `maxBodyBytes` of the response body, runs `next`
 * inside the context and records the request when the response closes.
 * Requests whose connection closes before the response finishes are
 * recorded with status 499.
 */
export function observeHttp(
  client: TrailscopeClient,
  req: IncomingMessage,
  res: ServerResponse,
  next: () => void,
  opts: ObserveOptions = {},
): void {
  if (!client.options.enabled || contextOf(req)) {
    next();
    return;
  }
  const ctx = client.contextFromHeaders((name) => first(req.headers[name]));
  attachContext(req, ctx);
  const timestamp = Date.now();
  const t0 = performance.now();
  if (!res.headersSent) res.setHeader(client.options.requestIdHeader, ctx.requestId);
  const body = client.options.captureBodies === 'never' ? undefined : keepResponseBody(res, client.options.maxBodyBytes);
  res.once('close', () => {
    const length = Number(res.getHeader('content-length'));
    client.recordHttp(ctx, {
      timestamp,
      durationMs: performance.now() - t0,
      method: req.method ?? 'GET',
      url: (req as { originalUrl?: string }).originalUrl ?? req.url ?? '/',
      route: opts.route?.(),
      status: res.writableFinished ? res.statusCode : 499,
      reqHeaders: req.headers,
      resHeaders: res.getHeaders(),
      reqBody: opts.body?.(),
      resBody: body?.read(),
      ip: opts.ip ? opts.ip() : clientIp(req),
      reqSize: Number(req.headers['content-length']) || undefined,
      resSize: Number.isFinite(length) && length > 0 ? length : body?.total() || undefined,
    });
  });
  runWithContext(ctx, next);
}

/**
 * Wraps `res.write` and `res.end` to copy the first `limit + 1` bytes of the
 * body (one extra byte marks truncation). Large chunks are sliced before
 * copying, so the cost stays bounded whatever the response size.
 */
function keepResponseBody(res: ServerResponse, limit: number): { read: () => Buffer | undefined; total: () => number } {
  const chunks: Buffer[] = [];
  let kept = 0;
  let total = 0;
  const keep = (chunk: unknown, encoding: unknown): void => {
    if (typeof chunk === 'string') {
      total += Buffer.byteLength(chunk);
      if (kept <= limit) {
        const enc = typeof encoding === 'string' && Buffer.isEncoding(encoding) ? encoding : 'utf8';
        const part = Buffer.from(chunk.slice(0, limit + 1 - kept), enc);
        chunks.push(part);
        kept += part.length;
      }
    } else if (chunk instanceof Uint8Array) {
      total += chunk.byteLength;
      if (kept <= limit) {
        const part = Buffer.from(chunk.subarray(0, limit + 1 - kept));
        chunks.push(part);
        kept += part.length;
      }
    }
  };
  const write = res.write;
  const end = res.end;
  res.write = function (this: ServerResponse, ...args: unknown[]): boolean {
    keep(args[0], args[1]);
    return (write as (...a: unknown[]) => boolean).apply(this, args);
  } as typeof res.write;
  res.end = function (this: ServerResponse, ...args: unknown[]): ServerResponse {
    if (typeof args[0] !== 'function') keep(args[0], args[1]);
    return (end as (...a: unknown[]) => ServerResponse).apply(this, args);
  } as typeof res.end;
  return {
    read: () => (chunks.length ? Buffer.concat(chunks) : undefined),
    total: () => total,
  };
}
