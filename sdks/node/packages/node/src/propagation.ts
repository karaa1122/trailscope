import { type TrailscopeContext, currentContext } from './context.js';

/** The part of a context that travels with a background job. */
export interface SerializedContext {
  requestId: string;
  traceId: string;
  spanId: string;
  userId?: string;
  correlation?: Record<string, string>;
}

/**
 * Captures the current context so it can be stored in job data and restored
 * by the worker (`client.trackJob(name, fn, { context: deserializeContext(data) })`).
 */
export function serializeContext(ctx: TrailscopeContext | undefined = currentContext()): SerializedContext | undefined {
  if (!ctx) return undefined;
  return {
    requestId: ctx.requestId,
    traceId: ctx.traceId,
    spanId: ctx.spanId,
    userId: ctx.userId,
    correlation: Object.keys(ctx.correlation).length ? { ...ctx.correlation } : undefined,
  };
}

export function deserializeContext(value: unknown): Partial<TrailscopeContext> | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const v = value as Record<string, unknown>;
  const str = (x: unknown) => (typeof x === 'string' && x.length <= 128 ? x : undefined);
  const correlation: Record<string, string> = {};
  if (v.correlation && typeof v.correlation === 'object') {
    for (const [k, val] of Object.entries(v.correlation as Record<string, unknown>)) {
      if (typeof val === 'string' || typeof val === 'number') correlation[k] = String(val);
    }
  }
  return {
    requestId: str(v.requestId),
    traceId: str(v.traceId),
    spanId: str(v.spanId),
    userId: str(v.userId),
    correlation,
  };
}
