import { randomBytes, randomUUID } from 'node:crypto';

/** 32 hex characters, like the server's generated IDs. */
export function newRequestId(): string {
  return randomUUID().replace(/-/g, '');
}

export function newTraceId(): string {
  return randomBytes(16).toString('hex');
}

export function newSpanId(): string {
  return randomBytes(8).toString('hex');
}

export interface TraceParent {
  traceId: string;
  parentSpanId: string;
  sampled: boolean;
}

const TRACEPARENT = /^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;

/** Parses a W3C `traceparent` header. Returns undefined when invalid. */
export function parseTraceparent(header: string | undefined | null): TraceParent | undefined {
  if (!header) return undefined;
  const m = TRACEPARENT.exec(header.trim().toLowerCase());
  if (!m) return undefined;
  const [, version, traceId, parentSpanId, flags] = m;
  if (version === 'ff' || !traceId || !parentSpanId || !flags) return undefined;
  if (/^0+$/.test(traceId) || /^0+$/.test(parentSpanId)) return undefined;
  return { traceId, parentSpanId, sampled: (parseInt(flags, 16) & 1) === 1 };
}

export function formatTraceparent(traceId: string, spanId: string, sampled = true): string {
  return `00-${traceId}-${spanId}-${sampled ? '01' : '00'}`;
}

/** Accepts a client-provided request ID only if it looks sane. */
export function acceptRequestId(value: string | undefined | null): string | undefined {
  if (!value) return undefined;
  const v = value.trim();
  return /^[A-Za-z0-9._:-]{1,128}$/.test(v) ? v : undefined;
}
