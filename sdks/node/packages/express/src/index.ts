import type { ErrorRequestHandler, Request, RequestHandler } from 'express';
import { TrailscopeClient, type TrailscopeOptions, contextOf, getClient, init, observeHttp, recordError } from '@trailscope/node';

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

function clientFor(clientOrOptions: TrailscopeClient | TrailscopeOptions | undefined): TrailscopeClient {
  if (clientOrOptions instanceof TrailscopeClient) return clientOrOptions;
  if (!clientOrOptions) return getClient() ?? init();
  return init(clientOrOptions);
}

/** `/orders` + `/:id`; undefined when no route matched (the server then templates the path). */
function routeOf(req: Request): string | undefined {
  const path: unknown = (req.route as { path?: unknown } | undefined)?.path;
  if (typeof path !== 'string') return undefined;
  return `${req.baseUrl ?? ''}${path}` || '/';
}

/**
 * Request logging middleware. Register it before your routes:
 *
 * ```ts
 * app.use(trailscope({ service: 'orders-api' }));
 * ```
 *
 * Pass a `TrailscopeClient`, or options to create the process-wide client with
 * `init()`. Without arguments it uses the client from an earlier `init()`,
 * or creates one from the `TRAILSCOPE_*` environment variables.
 */
export function trailscope(clientOrOptions?: TrailscopeClient | TrailscopeOptions): RequestHandler {
  const client = clientFor(clientOrOptions);
  return (req, res, next) => {
    observeHttp(client, req, res, next, {
      route: () => routeOf(req),
      body: () => req.body as unknown,
      ip: () => req.ip,
    });
  };
}

/**
 * Records the error that failed a request (type, message and stack, kept
 * when the response is a 5xx) and passes it on. Register it after your
 * routes and before your own error handlers.
 */
export function trailscopeErrorHandler(): ErrorRequestHandler {
  return (err: unknown, req, _res, next) => {
    recordError(err, contextOf(req));
    next(err);
  };
}
