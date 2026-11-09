/** Wire format for `POST /v1/ingest` (camelCase, see docs/ingest-protocol.md). */

export type Attributes = Record<string, string | number | boolean>;

export interface RequestItem {
  type: 'request';
  timestamp?: string | number;
  environment?: string;
  service: string;
  release?: string;
  requestId?: string;
  traceId?: string;
  spanId?: string;
  method: string;
  route?: string;
  path: string;
  query?: string;
  status: number;
  durationMs: number;
  reqSize?: number;
  resSize?: number;
  userId?: string;
  client?: string;
  clientVersion?: string;
  ip?: string;
  userAgent?: string;
  reqHeaders?: Record<string, string>;
  resHeaders?: Record<string, string>;
  reqBody?: string;
  resBody?: string;
  bodyTruncated?: boolean;
  /** The request body specifically was cut; without it, `bodyTruncated` may mean either body. */
  reqBodyTruncated?: boolean;
  errorType?: string;
  errorMessage?: string;
  errorStack?: string;
  attributes?: Attributes;
  correlation?: Record<string, string>;
  sampleRate?: number;
}

export type SpanKind = 'http' | 'db' | 'cache' | 'queue' | 'fn' | 'external';

export interface SpanItem {
  type: 'span';
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  requestId?: string;
  service: string;
  kind?: SpanKind;
  name: string;
  /** Epoch milliseconds (fractional allowed) or RFC 3339. */
  start?: string | number;
  durationMs: number;
  status?: 'ok' | 'error';
  attributes?: Attributes;
}

export type EventKind = 'job' | 'log' | 'custom';
export type Level = 'debug' | 'info' | 'warn' | 'error';

export interface EventItem {
  type: 'event';
  timestamp?: string | number;
  environment?: string;
  service: string;
  release?: string;
  kind?: EventKind;
  name: string;
  level?: Level;
  message?: string;
  status?: 'ok' | 'warn' | 'error';
  durationMs?: number;
  requestId?: string;
  traceId?: string;
  userId?: string;
  attempt?: number;
  errorType?: string;
  errorMessage?: string;
  errorStack?: string;
  attributes?: Attributes;
  correlation?: Record<string, string>;
}

export type Item = RequestItem | SpanItem | EventItem;

export interface IngestResponse {
  accepted: number;
  rejected: number;
  errors?: { index: number; message: string }[];
}
