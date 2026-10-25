import type { ErrorRequestHandler, Request, RequestHandler } from 'express';
import { LogboxClient, type LogboxOptions, contextOf, getClient, init, observeHttp, recordError } from '@logbox/node';

export {
  LogboxClient,
  contextOf,
  currentContext,
  getClient,
  init,
  recordError,
  setAttribute,
  setCorrelation,
  setUser,
} from '@logbox/node';
export type { LogboxContext, LogboxOptions } from '@logbox/node';

function clientFor(clientOrOptions: LogboxClient | LogboxOptions | undefined): LogboxClient {
  if (clientOrOptions instanceof LogboxClient) return clientOrOptions;
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
 * app.use(logbox({ service: 'orders-api' }));
 * ```
 *
 * Pass a `LogboxClient`, or options to create the process-wide client with
 * `init()`. Without arguments it uses the client from an earlier `init()`,
 * or creates one from the `LOGBOX_*` environment variables.
 */
export function logbox(clientOrOptions?: LogboxClient | LogboxOptions): RequestHandler {
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
export function logboxErrorHandler(): ErrorRequestHandler {
  return (err: unknown, req, _res, next) => {
    recordError(err, contextOf(req));
    next(err);
  };
}
