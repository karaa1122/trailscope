# @logbox/nestjs

[Logbox](https://github.com/karaa1122/LogBox) for NestJS 11 and 12 (Express or Fastify platform): request logging, error capture, a `Logger` bridge and BullMQ job tracking, without blocking your request path.

## Quickstart

```ts
// app.module.ts
@Module({ imports: [LogboxModule.forRoot({ service: 'orders-api' })] }) // reads LOGBOX_INGEST_KEY
export class AppModule {}

// main.ts
const app = await NestFactory.create(AppModule, { bufferLogs: true });
app.useLogger(app.get(LogboxLogger));
app.enableShutdownHooks();
await app.listen(3000);
```

BullMQ: `queue.add(name, withLogboxContext(data))` in the producer and `@TrackJob()` on your processor's `process()` record each run, linked to the request that queued it.

Full documentation: [docs/sdk-nestjs.md](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-nestjs.md).

## License

Apache-2.0
