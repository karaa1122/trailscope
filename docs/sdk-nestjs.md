# NestJS: `@logbox/nestjs`

Request logging, error capture, a `Logger` bridge and BullMQ job tracking for NestJS 11 and 12, on the Express or Fastify platform. Built on [`@logbox/node`](sdk-node.md); shared behaviour is in [sdk-spec.md](sdk-spec.md).

```bash
npm install @logbox/nestjs
```

```ts
// app.module.ts
@Module({ imports: [LogboxModule.forRoot({ service: 'orders-api' })] })
export class AppModule {}

// main.ts
const app = await NestFactory.create(AppModule, { bufferLogs: true });
app.useLogger(app.get(LogboxLogger));   // optional: Logger calls become Logbox events
app.enableShutdownHooks();              // flush on SIGTERM
await app.listen(3000);
```

## Module

`LogboxModule.forRoot(options)` is global. It:

- applies `LogboxMiddleware` to every route: it builds the context from `X-Request-Id` and `traceparent`, returns the request ID and records each request when the response closes;
- registers `LogboxInterceptor` globally: it sets the route pattern (`/orders/:id`, including the global prefix), enters the context again after body parsing (needed on Fastify), and records exceptions thrown by handlers;
- provides the client (`@Inject(LOGBOX_CLIENT) client: LogboxClient`) and `LogboxLogger`;
- flushes the client on application shutdown.

Options are the [`@logbox/node` options](sdk-node.md#options), plus:

| Option | Default | |
|---|---|---|
| `client` | | Use an existing `LogboxClient` |
| `exceptionFilter` | `false` | Register `LogboxExceptionFilter` globally (see below) |
| `loggerLevel` | `'info'` | Lowest level `LogboxLogger` forwards |
| `loggerIgnoreContexts` | Nest's startup contexts and `ExceptionsHandler` | Logger contexts not forwarded |

`LogboxModule.forRootAsync({ imports, inject, useFactory, exceptionFilter })` takes the options from other providers, such as `ConfigService`.

## Errors

The interceptor records exceptions thrown by controllers. Exceptions thrown earlier, in guards, pipes or middleware, are seen only by exception filters:

- With `exceptionFilter: true`, `LogboxExceptionFilter` (a `BaseExceptionFilter`) records every exception and then answers exactly like Nest's default handler.
- If you have your own catch-all filter, call `recordError(exception, contextOfRequest(request))` in it instead.

Error details are kept for 5xx responses only: `NotFoundException`, `ForbiddenException` and other 4xx outcomes are recorded as requests without an error.

## Logger bridge

`LogboxLogger` is a `ConsoleLogger` that also sends each line at `loggerLevel` or above to Logbox as a `log` event, named after the logger's context, with the current request's IDs, user and correlation keys. `logger.error(message, stack)` keeps the stack and takes the error type from its first line.

## BullMQ

```ts
// Producer: carry the request's context in the job data.
await queue.add('invoice.send', withLogboxContext({ invoiceId }));

// @nestjs/bullmq processor
@Processor('invoices')
export class InvoiceProcessor extends WorkerHost {
  @TrackJob()
  async process(job: Job) { await send(job.data.invoiceId); }
}

// Plain BullMQ worker
new Worker('invoices', trackJob(async (job) => send(job.data.invoiceId)), { connection });
```

Each run is recorded as a `kind=job` event with its duration, outcome and attempt number (`job.attemptsStarted`), linked to the request that queued it, with the queue name and job ID as attributes. Log lines written during the run carry the job's context. BullMQ 6 needs `ioredis` installed next to it.

## Context in your code

`setUser()`, `setCorrelation()`, `setAttribute()`, `currentContext()` and `recordError()` are re-exported. Call `setUser()` once the user is known (a guard or the controller), and `setCorrelation()` with the journey keys of the request.

A runnable app with a BullMQ queue is in [examples/nestjs](../examples/nestjs).
