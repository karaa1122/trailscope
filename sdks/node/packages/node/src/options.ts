import type { Item } from './types.js';

export type BodyCapture = 'errors' | 'always' | 'never';

export interface RedactOptions {
  /** Extra header names to redact (case-insensitive). */
  headers?: string[];
  /**
   * Extra body keys to redact at any depth (case-insensitive, `_`/`-` ignored),
   * or dotted paths such as `user.profile.dob` where `*` matches any key or index.
   */
  keys?: string[];
  /** Regexes whose matches are replaced in string values, header values and the query. */
  patterns?: RegExp[];
  /** Mask the last octet of client IPv4 addresses (IPv6 beyond /48). Default true. */
  maskIp?: boolean;
  /** Turn off the built-in header and key lists. Not recommended. */
  disableDefaults?: boolean;
}

export interface TrailscopeOptions {
  /** Trailscope base URL. Default `$TRAILSCOPE_ENDPOINT` or `http://localhost:8080`. */
  endpoint?: string;
  /** Ingest key (`trs_ingest_…`). Default `$TRAILSCOPE_INGEST_KEY`. */
  ingestKey?: string;
  /** Service name. Default `$TRAILSCOPE_SERVICE`. */
  service?: string;
  /** Default `$TRAILSCOPE_ENVIRONMENT`, then `$NODE_ENV`, then `production`. */
  environment?: string;
  /** Release or version. Default `$TRAILSCOPE_RELEASE`. */
  release?: string;
  /** Disable to make every call a no-op (e.g. in unit tests). Default true. */
  enabled?: boolean;

  /** Events held in memory; when full the oldest are dropped. Default 10 000. */
  maxQueueSize?: number;
  /** Flush interval. Default 1000 ms. */
  flushIntervalMs?: number;
  /** Flush early once this many events are queued. Default 500. */
  flushAt?: number;
  /** Items per HTTP request. Default 1000 (the server accepts up to 10 000). */
  maxBatchSize?: number;
  /** Per-request timeout. Default 10 000 ms. */
  requestTimeoutMs?: number;
  /** How long `shutdown()` waits for the final flush. Default 5000 ms. */
  shutdownTimeoutMs?: number;

  /** Sample rate for healthy requests (errors and slow requests are always kept). Default 1. */
  sampleRate?: number;
  /** Per-route overrides of `sampleRate`, keyed by route pattern. */
  routeSampleRates?: Record<string, number>;
  /** Requests slower than this are "slow". Default 1000 ms. */
  slowMs?: number;
  /** When to capture bodies. Default `errors` (status >= 400 or slow). */
  captureBodies?: BodyCapture;
  /** Per-body capture limit. Default 8192 bytes. */
  maxBodyBytes?: number;
  /** Routes or paths never reported. Default: common health and readiness routes. */
  ignoreRoutes?: (string | RegExp)[];
  /** Header carrying the request ID. Default `x-request-id`. */
  requestIdHeader?: string;

  redact?: RedactOptions;

  /**
   * Fetch the project's sampling and capture settings from `/v1/config` at
   * startup and every 5 minutes, so they can be tuned in the UI. Options set
   * here always win. Default true, or false when a custom `fetch` is given.
   */
  remoteConfig?: boolean;

  /** Last chance to edit or drop an item (return null to drop). Runs during flush. */
  beforeSend?: (item: Item) => Item | null;
  /** Log SDK diagnostics. `true` logs to console.debug. */
  debug?: boolean | ((message: string) => void);
  /** Custom fetch, mainly for tests. */
  fetch?: typeof fetch;
}

export const DEFAULT_IGNORE_ROUTES = ['/health', '/healthz', '/ready', '/readyz', '/livez'];

export interface ResolvedOptions {
  endpoint: string;
  ingestKey: string;
  service: string;
  environment: string;
  release: string;
  enabled: boolean;
  maxQueueSize: number;
  flushIntervalMs: number;
  flushAt: number;
  maxBatchSize: number;
  requestTimeoutMs: number;
  shutdownTimeoutMs: number;
  sampleRate: number;
  routeSampleRates: Record<string, number>;
  slowMs: number;
  captureBodies: BodyCapture;
  maxBodyBytes: number;
  ignoreRoutes: (string | RegExp)[];
  requestIdHeader: string;
  redact: Required<Omit<RedactOptions, 'disableDefaults'>> & { disableDefaults: boolean };
  remoteConfig: boolean;
  beforeSend: ((item: Item) => Item | null) | undefined;
  log: (message: string) => void;
  fetch: typeof fetch;
}

function env(name: string): string | undefined {
  const v = typeof process !== 'undefined' ? process.env[name] : undefined;
  return v === undefined || v === '' ? undefined : v;
}

function clampRate(r: number | undefined, fallback: number): number {
  if (r === undefined || Number.isNaN(r)) return fallback;
  return Math.min(1, Math.max(0, r));
}

export function resolveOptions(o: TrailscopeOptions = {}): ResolvedOptions {
  const debug = o.debug ?? env('TRAILSCOPE_DEBUG') === '1';
  const log =
    typeof debug === 'function'
      ? debug
      : debug
        ? (m: string) => console.debug(`[trailscope] ${m}`)
        : () => undefined;
  return {
    endpoint: (o.endpoint ?? env('TRAILSCOPE_ENDPOINT') ?? 'http://localhost:8080').replace(/\/+$/, ''),
    ingestKey: o.ingestKey ?? env('TRAILSCOPE_INGEST_KEY') ?? '',
    service: o.service ?? env('TRAILSCOPE_SERVICE') ?? 'unknown-service',
    environment: o.environment ?? env('TRAILSCOPE_ENVIRONMENT') ?? env('NODE_ENV') ?? 'production',
    release: o.release ?? env('TRAILSCOPE_RELEASE') ?? '',
    enabled: o.enabled ?? env('TRAILSCOPE_ENABLED') !== '0',
    maxQueueSize: Math.max(1, o.maxQueueSize ?? 10_000),
    flushIntervalMs: Math.max(10, o.flushIntervalMs ?? 1000),
    flushAt: Math.max(1, o.flushAt ?? 500),
    maxBatchSize: Math.min(10_000, Math.max(1, o.maxBatchSize ?? 1000)),
    requestTimeoutMs: o.requestTimeoutMs ?? 10_000,
    shutdownTimeoutMs: o.shutdownTimeoutMs ?? 5000,
    sampleRate: clampRate(o.sampleRate, 1),
    routeSampleRates: o.routeSampleRates ?? {},
    slowMs: o.slowMs ?? 1000,
    captureBodies: o.captureBodies ?? 'errors',
    maxBodyBytes: o.maxBodyBytes ?? 8192,
    ignoreRoutes: o.ignoreRoutes ?? DEFAULT_IGNORE_ROUTES,
    requestIdHeader: (o.requestIdHeader ?? 'x-request-id').toLowerCase(),
    redact: {
      headers: o.redact?.headers ?? [],
      keys: o.redact?.keys ?? [],
      patterns: o.redact?.patterns ?? [],
      maskIp: o.redact?.maskIp ?? true,
      disableDefaults: o.redact?.disableDefaults ?? false,
    },
    remoteConfig: o.remoteConfig ?? o.fetch === undefined,
    beforeSend: o.beforeSend,
    log,
    fetch: o.fetch ?? globalThis.fetch.bind(globalThis),
  };
}
