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
import { type TrailscopeClient, contextOf, enterContext, getClient, init } from '@trailscope/node';

import { TrailscopeExceptionFilter, TrailscopeInterceptor, TrailscopeMiddleware } from './http.js';
import { TrailscopeLogger } from './logger.js';
import { TRAILSCOPE_CLIENT, TRAILSCOPE_MODULE_OPTIONS, type TrailscopeModuleOptions } from './tokens.js';

export interface TrailscopeModuleAsyncOptions extends Pick<ModuleMetadata, 'imports'> {
  // Nest's own async module APIs type injection tokens and factory arguments this way.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  inject?: any[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  useFactory: (...args: any[]) => TrailscopeModuleOptions | Promise<TrailscopeModuleOptions>;
}

/** The part of a Fastify instance the module uses (no dependency on fastify). */
interface FastifyLike {
  addHook(
    name: 'preHandler',
    hook: (request: { raw: IncomingMessage }, reply: unknown, done: () => void) => void,
  ): unknown;
}

function createClient(options: TrailscopeModuleOptions): TrailscopeClient {
  // Strip the module-only options before handing the rest to the client.
  const { client, exceptionFilter: _f, loggerLevel: _l, loggerIgnoreContexts: _i, ...clientOptions } = options;
  if (client) return client;
  return Object.keys(clientOptions).length ? init(clientOptions) : (getClient() ?? init());
}

function providers(optionsProvider: Provider, exceptionFilter: boolean): Provider[] {
  const list: Provider[] = [
    optionsProvider,
    { provide: TRAILSCOPE_CLIENT, useFactory: createClient, inject: [TRAILSCOPE_MODULE_OPTIONS] },
    TrailscopeMiddleware,
    TrailscopeLogger,
    { provide: APP_INTERCEPTOR, useClass: TrailscopeInterceptor },
  ];
  if (exceptionFilter) list.push({ provide: APP_FILTER, useClass: TrailscopeExceptionFilter });
  return list;
}

/**
 * Request logging for the whole application:
 *
 * ```ts
 * @Module({ imports: [TrailscopeModule.forRoot({ service: 'orders-api' })] })
 * export class AppModule {}
 * ```
 *
 * It applies `TrailscopeMiddleware` to every route, registers `TrailscopeInterceptor`
 * globally, exports the client (`@Inject(TRAILSCOPE_CLIENT)`) and `TrailscopeLogger`,
 * and flushes on application shutdown (with `app.enableShutdownHooks()`).
 */
@Global()
@Module({})
export class TrailscopeModule implements NestModule, OnApplicationShutdown {
  constructor(
    @Inject(TRAILSCOPE_CLIENT) private readonly client: TrailscopeClient,
    @Optional() @Inject(HttpAdapterHost) private readonly adapterHost?: HttpAdapterHost,
  ) {}

  static forRoot(options: TrailscopeModuleOptions = {}): DynamicModule {
    return {
      module: TrailscopeModule,
      providers: providers({ provide: TRAILSCOPE_MODULE_OPTIONS, useValue: options }, !!options.exceptionFilter),
      exports: [TRAILSCOPE_CLIENT, TrailscopeLogger],
    };
  }

  /**
   * Options from other providers, e.g. `ConfigService`. Set `exceptionFilter`
   * here (not in the factory), since providers are fixed before it runs.
   */
  static forRootAsync(options: TrailscopeModuleAsyncOptions & { exceptionFilter?: boolean }): DynamicModule {
    return {
      module: TrailscopeModule,
      imports: options.imports ?? [],
      providers: providers(
        { provide: TRAILSCOPE_MODULE_OPTIONS, useFactory: options.useFactory, inject: options.inject ?? [] },
        !!options.exceptionFilter,
      ),
      exports: [TRAILSCOPE_CLIENT, TrailscopeLogger],
    };
  }

  configure(consumer: MiddlewareConsumer): void {
    consumer.apply(TrailscopeMiddleware).forRoutes('{*path}');
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
