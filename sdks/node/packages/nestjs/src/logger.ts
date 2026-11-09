import { ConsoleLogger, Inject, Injectable } from '@nestjs/common';
import type { Level, TrailscopeClient } from '@trailscope/node';

import { TRAILSCOPE_CLIENT, TRAILSCOPE_MODULE_OPTIONS, type TrailscopeModuleOptions } from './tokens.js';

const RANK: Record<Level, number> = { debug: 0, info: 1, warn: 2, error: 3 };

/**
 * Nest's startup chatter, and `ExceptionsHandler`, whose unhandled-exception
 * lines would duplicate the error already recorded with the request.
 */
export const DEFAULT_IGNORED_LOGGER_CONTEXTS = [
  'NestFactory',
  'NestApplication',
  'InstanceLoader',
  'RoutesResolver',
  'RouterExplorer',
  'NestMicroservice',
  'ExceptionsHandler',
];

/**
 * A `ConsoleLogger` that also sends each line to Trailscope as a `log` event,
 * with the current request's IDs, user and correlation keys:
 *
 * ```ts
 * const app = await NestFactory.create(AppModule, { bufferLogs: true });
 * app.useLogger(app.get(TrailscopeLogger));
 * ```
 */
@Injectable()
export class TrailscopeLogger extends ConsoleLogger {
  private readonly minRank: number;
  private readonly ignored: Set<string>;

  constructor(
    @Inject(TRAILSCOPE_CLIENT) private readonly client: TrailscopeClient,
    @Inject(TRAILSCOPE_MODULE_OPTIONS) options: TrailscopeModuleOptions,
  ) {
    super();
    this.minRank = RANK[options.loggerLevel ?? 'info'];
    this.ignored = new Set(options.loggerIgnoreContexts ?? DEFAULT_IGNORED_LOGGER_CONTEXTS);
  }

  // Nest's logger methods are typed with `any`.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override log(message: any, ...rest: any[]): void {
    super.log(message, ...rest);
    this.forward('info', message, rest);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override warn(message: any, ...rest: any[]): void {
    super.warn(message, ...rest);
    this.forward('warn', message, rest);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override error(message: any, ...rest: any[]): void {
    super.error(message, ...rest);
    this.forward('error', message, rest);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override fatal(message: any, ...rest: any[]): void {
    super.fatal(message, ...rest);
    this.forward('error', message, rest);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override debug(message: any, ...rest: any[]): void {
    super.debug(message, ...rest);
    this.forward('debug', message, rest);
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  override verbose(message: any, ...rest: any[]): void {
    super.verbose(message, ...rest);
    this.forward('debug', message, rest);
  }

  private forward(level: Level, message: unknown, rest: unknown[]): void {
    if (RANK[level] < this.minRank) return;
    const last = rest[rest.length - 1];
    const context = typeof last === 'string' ? last : this.context;
    if (context && this.ignored.has(context)) return;
    let error = message instanceof Error ? message : undefined;
    const text = error ? error.message : typeof message === 'string' ? message : stringify(message);
    // `logger.error(message, stack, context)`: take the type from the stack's first line.
    if (!error && level === 'error' && rest.length >= 2 && typeof rest[0] === 'string') {
      const name = /^([A-Za-z_$][\w$.]*):/.exec(rest[0])?.[1] ?? 'Error';
      error = Object.assign(new Error(text), { name, stack: rest[0] });
    }
    this.client.log(level, text, { name: context ?? 'nest', error });
  }
}

function stringify(v: unknown): string {
  try {
    return JSON.stringify(v) ?? String(v);
  } catch {
    return String(v);
  }
}
