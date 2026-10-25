import {
  type DynamicModule,
  Global,
  Inject,
  type MiddlewareConsumer,
  Module,
  type ModuleMetadata,
  type NestModule,
  type OnApplicationShutdown,
  Optional,
  type Provider,
} from '@nestjs/common';
import type { IncomingMessage } from 'node:http';

import { APP_FILTER, APP_INTERCEPTOR, HttpAdapterHost } from '@nestjs/core';
import { type LogboxClient, contextOf, enterContext, getClient, init } from '@logbox/node';

import { LogboxExceptionFilter, LogboxInterceptor, LogboxMiddleware } from './http.js';
import { LogboxLogger } from './logger.js';
import { LOGBOX_CLIENT, LOGBOX_MODULE_OPTIONS, type LogboxModuleOptions } from './tokens.js';

export interface LogboxModuleAsyncOptions extends Pick<ModuleMetadata, 'imports'> {
  // Nest's own async module APIs type injection tokens and factory arguments this way.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  inject?: any[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  useFactory: (...args: any[]) => LogboxModuleOptions | Promise<LogboxModuleOptions>;
}

/** The part of a Fastify instance the module uses (no dependency on fastify). */
interface FastifyLike {
  addHook(
    name: 'preHandler',
    hook: (request: { raw: IncomingMessage }, reply: unknown, done: () => void) => void,
  ): unknown;
}

function createClient(options: LogboxModuleOptions): LogboxClient {
  // Strip the module-only options before handing the rest to the client.
  const { client, exceptionFilter: _f, loggerLevel: _l, loggerIgnoreContexts: _i, ...clientOptions } = options;
  if (client) return client;
  return Object.keys(clientOptions).length ? init(clientOptions) : (getClient() ?? init());
}

function providers(optionsProvider: Provider, exceptionFilter: boolean): Provider[] {
  const list: Provider[] = [
    optionsProvider,
    { provide: LOGBOX_CLIENT, useFactory: createClient, inject: [LOGBOX_MODULE_OPTIONS] },
    LogboxMiddleware,
    LogboxLogger,
    { provide: APP_INTERCEPTOR, useClass: LogboxInterceptor },
  ];
  if (exceptionFilter) list.push({ provide: APP_FILTER, useClass: LogboxExceptionFilter });
  return list;
}

/**
 * Request logging for the whole application:
 *
 * ```ts
 * @Module({ imports: [LogboxModule.forRoot({ service: 'orders-api' })] })
 * export class AppModule {}
 * ```
 *
 * It applies `LogboxMiddleware` to every route, registers `LogboxInterceptor`
 * globally, exports the client (`@Inject(LOGBOX_CLIENT)`) and `LogboxLogger`,
 * and flushes on application shutdown (with `app.enableShutdownHooks()`).
 */
@Global()
@Module({})
export class LogboxModule implements NestModule, OnApplicationShutdown {
  constructor(
    @Inject(LOGBOX_CLIENT) private readonly client: LogboxClient,
    @Optional() @Inject(HttpAdapterHost) private readonly adapterHost?: HttpAdapterHost,
  ) {}

  static forRoot(options: LogboxModuleOptions = {}): DynamicModule {
    return {
      module: LogboxModule,
      providers: providers({ provide: LOGBOX_MODULE_OPTIONS, useValue: options }, !!options.exceptionFilter),
      exports: [LOGBOX_CLIENT, LogboxLogger],
    };
  }

  /**
   * Options from other providers, e.g. `ConfigService`. Set `exceptionFilter`
   * here (not in the factory), since providers are fixed before it runs.
   */
  static forRootAsync(options: LogboxModuleAsyncOptions & { exceptionFilter?: boolean }): DynamicModule {
    return {
      module: LogboxModule,
      imports: options.imports ?? [],
      providers: providers(
        { provide: LOGBOX_MODULE_OPTIONS, useFactory: options.useFactory, inject: options.inject ?? [] },
        !!options.exceptionFilter,
      ),
      exports: [LOGBOX_CLIENT, LogboxLogger],
    };
  }

  configure(consumer: MiddlewareConsumer): void {
    consumer.apply(LogboxMiddleware).forRoutes('{*path}');
    // Fastify parses bodies outside the request's async context, and guards
    // run before interceptors: enter the context again before guards run.
    const adapter = this.adapterHost?.httpAdapter;
    if (adapter?.getType() === 'fastify') {
      adapter.getInstance<FastifyLike>().addHook('preHandler', (request, _reply, done) => {
        const ctx = contextOf(request.raw);
        if (ctx) enterContext(ctx);
        done();
      });
    }
  }

  async onApplicationShutdown(): Promise<void> {
    await this.client.shutdown();
  }
}
