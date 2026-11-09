import type { FastifyPluginCallback, FastifyReply, FastifyRequest } from 'fastify';
import {
  TrailscopeClient,
  type TrailscopeOptions,
  attachContext,
  contextOf,
  enterContext,
  getClient,
  init,
  recordError,
  runWithContext,
} from '@trailscope/node';

export {
  TrailscopeClient,
  contextOf,
  currentContext,
  getClient,
  init,
  recordError,
  setAttribute,
  setCorrelation,
  setUser,
} from '@trailscope/node';
export type { TrailscopeContext, TrailscopeOptions } from '@trailscope/node';

export interface TrailscopeFastifyOptions extends TrailscopeOptions {
  /** Use this client instead of creating the process-wide one with `init()`. */
  client?: TrailscopeClient;
}

interface Timing {
  timestamp: number;
  t0: number;
  payload?: unknown;
  recorded?: boolean;
}

const TIMING = Symbol('trailscope.timing');
type Tracked = FastifyRequest & { [TIMING]?: Timing };

function first(v: string | string[] | undefined): string | undefined {
  return Array.isArray(v) ? v[0] : v;
}

function clientFor(opts: TrailscopeFastifyOptions): TrailscopeClient {
  const { client, ...options } = opts;
  if (client) return client;
  return Object.keys(options).length ? init(options) : (getClient() ?? init());
}

function record(client: TrailscopeClient, request: Tracked, status: number, reply?: FastifyReply): void {
  const ctx = contextOf(request);
  const timing = request[TIMING];
  if (!ctx || !timing || timing.recorded) return;
  timing.recorded = true;
  const length = Number(reply?.getHeader('content-length'));
  client.recordHttp(ctx, {
    timestamp: timing.timestamp,
    durationMs: performance.now() - timing.t0,
    method: request.method,
    url: request.url,
    route: request.routeOptions.url,
    status,
    reqHeaders: request.headers,
    resHeaders: reply?.getHeaders(),
    reqBody: request.body,
    resBody: timing.payload,
    ip: request.ip,
    reqSize: Number(request.headers['content-length']) || undefined,
    resSize: Number.isFinite(length) && length > 0 ? length : undefined,
  });
}

/**
 * Request logging for Fastify. Register it before your routes:
 *
 * ```ts
 * await app.register(trailscope, { service: 'orders-api' });
 * ```
 *
 * It applies to the whole instance (not only the plugin's scope), echoes the
 * request ID, restores the context after body parsing so handlers, log lines
 * and jobs see it, and records 5xx errors with their stack.
 */
const plugin: FastifyPluginCallback<TrailscopeFastifyOptions> = (app, opts, done) => {
  const client = clientFor(opts);
  if (!client.options.enabled) {
    done();
    return;
  }

  app.addHook('onRequest', (request: Tracked, reply, next) => {
    const ctx = client.contextFromHeaders((name) => first(request.headers[name]));
    attachContext(request, ctx);
    request[TIMING] = { timestamp: Date.now(), t0: performance.now() };
    void reply.header(client.options.requestIdHeader, ctx.requestId);
    runWithContext(ctx, next);
  });

  // Body parsing runs on stream events, outside the context; enter it again
  // right before the handler.
  app.addHook('preHandler', (request, _reply, next) => {
    const ctx = contextOf(request);
    if (ctx) {
      ctx.route = request.routeOptions.url;
      enterContext(ctx);
    }
    next();
  });

  app.addHook('onSend', (request: Tracked, _reply, payload, next) => {
    const timing = request[TIMING];
    if (timing && (typeof payload === 'string' || Buffer.isBuffer(payload))) timing.payload = payload;
    next(null, payload);
  });

  app.addHook('onError', (request, _reply, error, next) => {
    recordError(error, contextOf(request));
    next();
  });

  app.addHook('onResponse', (request: Tracked, reply, next) => {
    record(client, request, reply.statusCode, reply);
    next();
  });

  // The client went away before the response was sent.
  app.addHook('onRequestAbort', (request: Tracked, next) => {
    record(client, request, 499);
    next();
  });

  done();
};

// What fastify-plugin does: share hooks with the parent instead of an encapsulated child.
Object.assign(plugin, {
  [Symbol.for('skip-override')]: true,
  [Symbol.for('fastify.display-name')]: '@trailscope/fastify',
  [Symbol.for('plugin-meta')]: { name: '@trailscope/fastify', fastify: '5.x' },
});

export const trailscope = plugin;
export default plugin;
