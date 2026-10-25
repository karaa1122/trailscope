import type { IncomingMessage, ServerResponse } from 'node:http';

import { Catch, type ArgumentsHost, type CallHandler, type ExecutionContext, Inject, Injectable, type NestInterceptor, type NestMiddleware } from '@nestjs/common';
import { BaseExceptionFilter } from '@nestjs/core';
import { type LogboxClient, type LogboxContext, clientIp, contextOf, enterContext, observeHttp, recordError } from '@logbox/node';
import { type Observable, catchError, throwError } from 'rxjs';

import { LOGBOX_CLIENT } from './tokens.js';

/** Express request, Fastify request or the raw Node request, depending on the platform. */
interface AnyRequest {
  raw?: IncomingMessage;
  body?: unknown;
  ip?: string;
  route?: { path?: unknown };
  routeOptions?: { url?: string };
}

const BODY = Symbol.for('logbox.nestjs.body');
type Raw = IncomingMessage & { [BODY]?: unknown };

/** The Logbox context of a Nest request, on Express or Fastify. */
export function contextOfRequest(req: AnyRequest | undefined): LogboxContext | undefined {
  return contextOf(req?.raw ?? req);
}

function routeOf(req: AnyRequest): string | undefined {
  if (typeof req.route?.path === 'string') return req.route.path;
  return req.routeOptions?.url;
}

/**
 * Builds the context and records every request. `LogboxModule` applies it to
 * all routes. It receives the raw Node request on both platforms.
 */
@Injectable()
export class LogboxMiddleware implements NestMiddleware {
  constructor(@Inject(LOGBOX_CLIENT) private readonly client: LogboxClient) {}

  use(req: IncomingMessage, res: ServerResponse, next: (error?: unknown) => void): void {
    const r = req as Raw & AnyRequest;
    observeHttp(this.client, req, res, () => next(), {
      route: () => contextOf(req)?.route ?? routeOf(r),
      body: () => r.body ?? r[BODY],
      ip: () => r.ip ?? clientIp(req),
    });
  }
}

/**
 * Sets the route pattern, re-enters the context after body parsing (Fastify
 * parses bodies outside it), and records exceptions thrown by handlers.
 */
@Injectable()
export class LogboxInterceptor implements NestInterceptor {
  intercept(context: ExecutionContext, next: CallHandler): Observable<unknown> {
    if (context.getType() !== 'http') return next.handle();
    const req = context.switchToHttp().getRequest<AnyRequest>();
    const ctx = contextOfRequest(req);
    if (!ctx) return next.handle();
    ctx.route ??= routeOf(req);
    if (req.raw && req.body !== undefined) (req.raw as Raw)[BODY] = req.body;
    enterContext(ctx);
    return next.handle().pipe(
      catchError((error: unknown) => {
        recordError(error, ctx);
        return throwError(() => error);
      }),
    );
  }
}

/**
 * Records any exception (also from guards, pipes and middleware), then
 * answers exactly like Nest's default handler. Enable it with
 * `LogboxModule.forRoot({ exceptionFilter: true })`, or extend it.
 */
@Catch()
export class LogboxExceptionFilter extends BaseExceptionFilter {
  override catch(exception: unknown, host: ArgumentsHost): void {
    if (host.getType() === 'http') recordError(exception, contextOfRequest(host.switchToHttp().getRequest<AnyRequest>()));
    super.catch(exception, host);
  }
}
