# @trailscope/nestjs

[Trailscope](https://github.com/karaa1122/trailscope) for NestJS 11 and 12 (Express or Fastify platform): request logging, error capture, a `Logger` bridge and BullMQ job tracking, without blocking your request path.

## Quickstart

```ts
// app.module.ts
@Module({ imports: [TrailscopeModule.forRoot({ service: 'orders-api' })] }) // reads TRAILSCOPE_INGEST_KEY
export class AppModule {}

// main.ts
const app = await NestFactory.create(AppModule, { bufferLogs: true });
app.useLogger(app.get(TrailscopeLogger));
app.enableShutdownHooks();
await app.listen(3000);
```

BullMQ: `queue.add(name, withTrailscopeContext(data))` in the producer and `@TrackJob()` on your processor's `process()` record each run, linked to the request that queued it.

Full documentation: [docs/sdk-nestjs.md](https://github.com/karaa1122/trailscope/blob/main/docs/sdk-nestjs.md).

## License

Apache-2.0
