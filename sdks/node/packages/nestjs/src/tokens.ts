import type { Level, LogboxClient, LogboxOptions } from '@logbox/node';

/** Injection token for the `LogboxClient`. */
export const LOGBOX_CLIENT = Symbol.for('logbox.nestjs.client');
export const LOGBOX_MODULE_OPTIONS = Symbol.for('logbox.nestjs.options');

export interface LogboxModuleOptions extends LogboxOptions {
  /** Use this client instead of creating the process-wide one with `init()`. */
  client?: LogboxClient;
  /**
   * Register `LogboxExceptionFilter` globally, so errors thrown in guards,
   * pipes and middleware are recorded too. It keeps Nest's default error
   * responses. Leave it off if you have your own catch-all filter and call
   * `recordError()` there instead. Default false.
   */
  exceptionFilter?: boolean;
  /** Lowest level the `LogboxLogger` bridge forwards to Logbox. Default `info`. */
  loggerLevel?: Level;
  /** Logger contexts not forwarded. Default: Nest's startup contexts. */
  loggerIgnoreContexts?: string[];
}
