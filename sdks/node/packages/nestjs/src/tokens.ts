import type { Level, TrailscopeClient, TrailscopeOptions } from '@trailscope/node';

/** Injection token for the `TrailscopeClient`. */
export const TRAILSCOPE_CLIENT = Symbol.for('trailscope.nestjs.client');
export const TRAILSCOPE_MODULE_OPTIONS = Symbol.for('trailscope.nestjs.options');

export interface TrailscopeModuleOptions extends TrailscopeOptions {
  /** Use this client instead of creating the process-wide one with `init()`. */
  client?: TrailscopeClient;
  /**
   * Register `TrailscopeExceptionFilter` globally, so errors thrown in guards,
   * pipes and middleware are recorded too. It keeps Nest's default error
   * responses. Leave it off if you have your own catch-all filter and call
   * `recordError()` there instead. Default false.
   */
  exceptionFilter?: boolean;
  /** Lowest level the `TrailscopeLogger` bridge forwards to Trailscope. Default `info`. */
  loggerLevel?: Level;
  /** Logger contexts not forwarded. Default: Nest's startup contexts. */
  loggerIgnoreContexts?: string[];
}
