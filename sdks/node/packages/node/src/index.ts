export { TrailscopeClient, init, getClient, toError, SDK_VERSION } from './client.js';
export type { ClientStats, EventInput, JobOptions, RemoteConfig, RequestInput, SpanInput } from './client.js';
export {
  currentContext,
  enterContext,
  recordError,
  runWithContext,
  setAttribute,
  setCorrelation,
  setUser,
} from './context.js';
export type { TrailscopeContext } from './context.js';
export { attachContext, clientIp, contextOf, observeHttp } from './http.js';
export type { HttpExchange, ObserveOptions } from './http.js';
export {
  acceptRequestId,
  formatTraceparent,
  newRequestId,
  newSpanId,
  newTraceId,
  parseTraceparent,
} from './ids.js';
export type { TraceParent } from './ids.js';
export { DEFAULT_IGNORE_ROUTES, resolveOptions } from './options.js';
export type { BodyCapture, TrailscopeOptions, RedactOptions, ResolvedOptions } from './options.js';
export { BoundedQueue } from './queue.js';
export {
  DEFAULT_REDACT_HEADERS,
  DEFAULT_REDACT_KEYS,
  REDACTED,
  Redactor,
  maskIp,
} from './redact.js';
export {
  flattenHeaders,
  isBinaryContentType,
  serializeBody,
  shouldCaptureBodies,
  splitUrl,
  truncate,
} from './capture.js';
export type { CapturedBody } from './capture.js';
export { serializeContext, deserializeContext } from './propagation.js';
export type * from './types.js';
