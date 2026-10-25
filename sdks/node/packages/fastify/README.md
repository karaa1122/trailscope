# @logbox/fastify

[Logbox](https://github.com/karaa1122/LogBox) plugin for Fastify 5: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, without blocking your request path.

## Quickstart

```ts
import Fastify from 'fastify';
import { logbox, setCorrelation } from '@logbox/fastify';

const app = Fastify();
await app.register(logbox, { service: 'orders-api' }); // reads LOGBOX_ENDPOINT and LOGBOX_INGEST_KEY

app.get('/orders/:id', async (request) => {
  setCorrelation('order_id', request.params.id);
  return { id: request.params.id };
});
await app.listen({ port: 3000 });
```

Full documentation: [docs/sdk-fastify.md](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-fastify.md). Options: [`@logbox/node`](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-node.md#options).

## License

Apache-2.0
