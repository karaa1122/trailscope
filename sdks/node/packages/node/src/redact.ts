import type { RedactOptions } from './options.js';

export const REDACTED = '[REDACTED]';

export const DEFAULT_REDACT_HEADERS = ['authorization', 'cookie', 'set-cookie', 'x-api-key'];

export const DEFAULT_REDACT_KEYS = [
  'password',
  'passwd',
  'secret',
  'token',
  'access_token',
  'refresh_token',
  'otp',
  'pin',
  'cvv',
  'card',
  'card_number',
  'iban',
  'ssn',
];

/** `accessToken`, `access-token` and `ACCESS_TOKEN` all normalize to `accesstoken`. */
function normKey(k: string): string {
  return k.replace(/[_-]/g, '').toLowerCase();
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Redacts headers, JSON bodies (any depth), urlencoded bodies, query strings
 * and IPs. The same rules run server-side as defense in depth.
 */
export class Redactor {
  private readonly headers: Set<string>;
  private readonly keys: Set<string>;
  private readonly paths: string[][];
  private readonly patterns: RegExp[];
  private readonly rawKeys: RegExp | undefined;
  readonly maskIp: boolean;

  constructor(o: Required<Omit<RedactOptions, 'disableDefaults'>> & { disableDefaults: boolean }) {
    const baseHeaders = o.disableDefaults ? [] : DEFAULT_REDACT_HEADERS;
    const baseKeys = o.disableDefaults ? [] : DEFAULT_REDACT_KEYS;
    this.headers = new Set([...baseHeaders, ...o.headers].map((h) => h.toLowerCase()));
    const simple = [...baseKeys];
    this.paths = [];
    for (const k of o.keys) {
      if (k.includes('.')) this.paths.push(k.split('.').map(normKey));
      else if (k) simple.push(k);
    }
    this.keys = new Set(simple.map(normKey));
    this.patterns = o.patterns.map((p) => (p.global ? p : new RegExp(p.source, `${p.flags}g`)));
    this.maskIp = o.maskIp;
    this.rawKeys = simple.length
      ? new RegExp(
          `("(?:${simple.map((k) => k.split('_').map(escapeRegex).join('[_-]?')).join('|')})"\\s*:\\s*)("(?:[^"\\\\]|\\\\.)*"?|[^,}\\]\\s]+)`,
          'gi',
        )
      : undefined;
  }

  redactHeaders(headers: Record<string, string>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [name, value] of Object.entries(headers)) {
      const lower = name.toLowerCase();
      out[lower] = this.headers.has(lower) ? REDACTED : this.applyPatterns(value);
    }
    return out;
  }

  /** Redacts a serialized body according to its content type. */
  redactBody(body: string, contentType = ''): string {
    if (!body) return body;
    const t = body.trimStart();
    if (t.startsWith('{') || t.startsWith('[')) {
      try {
        const parsed: unknown = JSON.parse(body);
        return JSON.stringify(this.redactValue(parsed, []));
      } catch {
        return this.redactRawText(body);
      }
    }
    if (contentType.includes('x-www-form-urlencoded')) return this.redactUrlencoded(body);
    return this.redactRawText(body);
  }

  redactUrlencoded(s: string): string {
    return s
      .split('&')
      .map((pair) => {
        const i = pair.indexOf('=');
        if (i < 0) return pair;
        const k = pair.slice(0, i);
        let decoded = k;
        try {
          decoded = decodeURIComponent(k.replace(/\+/g, ' '));
        } catch {
          // keep raw key
        }
        if (this.keys.has(normKey(decoded))) return `${k}=${REDACTED}`;
        return `${k}=${this.applyPatterns(pair.slice(i + 1))}`;
      })
      .join('&');
  }

  redactQuery(q: string): string {
    return q ? this.redactUrlencoded(q.replace(/^\?/, '')) : q;
  }

  /** Redacts a structured value; used for JSON bodies and job payloads. */
  redactValue(v: unknown, path: string[]): unknown {
    if (Array.isArray(v)) {
      return v.map((item) => this.redactValue(item, [...path, '*']));
    }
    if (v !== null && typeof v === 'object') {
      const out: Record<string, unknown> = {};
      for (const [k, child] of Object.entries(v as Record<string, unknown>)) {
        const nk = normKey(k);
        const p = [...path, nk];
        out[k] =
          (this.keys.has(nk) || this.pathMatches(p)) && child !== null && child !== undefined
            ? REDACTED
            : this.redactValue(child, p);
      }
      return out;
    }
    if (typeof v === 'string') return this.applyPatterns(v);
    return v;
  }

  applyPatterns(s: string): string {
    let out = s;
    for (const re of this.patterns) {
      re.lastIndex = 0;
      out = out.replace(re, REDACTED);
    }
    return out;
  }

  ip(ip: string | undefined): string | undefined {
    if (!ip || !this.maskIp) return ip;
    return maskIp(ip);
  }

  private redactRawText(s: string): string {
    let out = s;
    if (this.rawKeys) {
      this.rawKeys.lastIndex = 0;
      out = out.replace(this.rawKeys, `$1"${REDACTED}"`);
    }
    return this.applyPatterns(out);
  }

  private pathMatches(path: string[]): boolean {
    return this.paths.some(
      (p) =>
        p.length === path.length && p.every((want, i) => want === '*' || path[i] === '*' || want === path[i]),
    );
  }
}

export function maskIp(raw: string): string {
  let ip = raw.trim();
  if (ip.startsWith('::ffff:') && ip.includes('.')) ip = ip.slice(7);
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.\d{1,3}$/.exec(ip);
  if (v4) return `${v4[1]}.${v4[2]}.${v4[3]}.0`;
  if (ip.includes(':')) {
    const head = ip.split('::')[0] ?? '';
    const parts = head.split(':').filter(Boolean);
    return `${parts.slice(0, 3).join(':')}::`;
  }
  return '';
}
