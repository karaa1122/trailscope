import { AsyncLocalStorage } from 'node:async_hooks';

/** Request-scoped context carried through async calls via AsyncLocalStorage. */
export interface TrailscopeContext {
  requestId: string;
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  userId?: string;
  /** Journey keys, e.g. `{ account_id: 'acc_1' }`. */
  correlation: Record<string, string>;
  attributes: Record<string, string | number | boolean>;
  /** Route pattern, once the framework has matched one. */
  route?: string;
  /** The exception that failed the request, recorded by an error handler. */
  error?: unknown;
}

const storage = new AsyncLocalStorage<TrailscopeContext>();

export function runWithContext<T>(ctx: TrailscopeContext, fn: () => T): T {
  return storage.run(ctx, fn);
}

/** Enters a context for the rest of the current synchronous execution and its async continuations. */
export function enterContext(ctx: TrailscopeContext): void {
  storage.enterWith(ctx);
}

export function currentContext(): TrailscopeContext | undefined {
  return storage.getStore();
}

/** Attaches a user ID to the current request. */
export function setUser(userId: string | number | undefined | null): void {
  const ctx = storage.getStore();
  if (ctx && userId !== undefined && userId !== null && userId !== '') ctx.userId = String(userId);
}

/** Adds a correlation (journey) key to the current request and everything it triggers. */
export function setCorrelation(key: string, value: string | number): void {
  const ctx = storage.getStore();
  if (ctx && key) ctx.correlation[key] = String(value);
}

/** Adds a free-form attribute to the current request. */
export function setAttribute(key: string, value: string | number | boolean): void {
  const ctx = storage.getStore();
  if (ctx && key) ctx.attributes[key] = value;
}

/**
 * Attaches the exception that failed the current request, so its type,
 * message and stack are recorded with it (when the response is a 5xx).
 */
export function recordError(error: unknown, ctx: TrailscopeContext | undefined = storage.getStore()): void {
  if (ctx && error !== undefined && ctx.error === undefined) ctx.error = error;
}
