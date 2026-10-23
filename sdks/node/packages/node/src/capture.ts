import type { BodyCapture } from './options.js';

const BINARY_TYPES = [
  'multipart/',
  'application/octet-stream',
  'image/',
  'audio/',
  'video/',
  'font/',
  'application/pdf',
  'application/zip',
  'application/gzip',
  'application/x-protobuf',
  'application/grpc',
];

/** Whether a body with this content type should never be captured. */
export function isBinaryContentType(contentType: string | undefined): boolean {
  if (!contentType) return false;
  const ct = contentType.toLowerCase();
  return BINARY_TYPES.some((t) => ct.startsWith(t));
}

/** Whether bodies should be kept for a request with this outcome. */
export function shouldCaptureBodies(
  policy: BodyCapture,
  status: number,
  durationMs: number,
  slowMs: number,
): boolean {
  if (policy === 'never') return false;
  if (policy === 'always') return true;
  return status >= 400 || durationMs >= slowMs;
}

export interface CapturedBody {
  body: string;
  truncated: boolean;
}

/**
 * Serializes a body-like value (string, Buffer, object) and truncates it to
 * `limit` bytes. Returns undefined for binary or empty bodies.
 */
export function serializeBody(
  value: unknown,
  contentType: string | undefined,
  limit: number,
): CapturedBody | undefined {
  if (value === undefined || value === null || isBinaryContentType(contentType)) return undefined;
  let text: string;
  if (typeof value === 'string') {
    text = value;
  } else if (Buffer.isBuffer(value) || value instanceof Uint8Array) {
    const buf = Buffer.from(value);
    if (looksBinary(buf)) return undefined;
    text = buf.subarray(0, limit + 4).toString('utf8');
  } else if (typeof value === 'object') {
    try {
      text = JSON.stringify(value);
    } catch {
      return undefined;
    }
  } else {
    text = String(value);
  }
  if (!text) return undefined;
  return truncate(text, limit);
}

export function truncate(text: string, limit: number): CapturedBody {
  if (Buffer.byteLength(text, 'utf8') <= limit) return { body: text, truncated: false };
  let cut = Buffer.from(text, 'utf8').subarray(0, limit).toString('utf8');
  // Drop a trailing partial character.
  if (cut.endsWith('�')) cut = cut.slice(0, -1);
  return { body: cut, truncated: true };
}

function looksBinary(buf: Buffer): boolean {
  const n = Math.min(buf.length, 512);
  for (let i = 0; i < n; i++) {
    if (buf[i] === 0) return true;
  }
  return false;
}

/** Normalizes header values (arrays joined, numbers stringified). */
export function flattenHeaders(
  headers: Record<string, string | string[] | number | undefined> | undefined,
): Record<string, string> {
  const out: Record<string, string> = {};
  if (!headers) return out;
  for (const [k, v] of Object.entries(headers)) {
    if (v === undefined) continue;
    out[k.toLowerCase()] = Array.isArray(v) ? v.join(', ') : String(v);
  }
  return out;
}

/** Splits `/a/b?x=1` into path and query. */
export function splitUrl(url: string): { path: string; query: string } {
  const i = url.indexOf('?');
  return i < 0 ? { path: url || '/', query: '' } : { path: url.slice(0, i) || '/', query: url.slice(i + 1) };
}
