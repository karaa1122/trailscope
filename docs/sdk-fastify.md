# Fastify: `@trailscope/fastify`

Request logging, error capture and context for Fastify 5. Built on [`@trailscope/node`](sdk-node.md); shared behaviour is in [sdk-spec.md](sdk-spec.md).

```bash
npm install @trailscope/fastify
```

```ts
import Fastify from 'fastify';
import { trailscope, setCorrelation } from '@trailscope/fastify';

const app = Fastify();
await app.register(trailscope, { service: 'orders-api' }); // TRAILSCOPE_INGEST_KEY from the environment

app.get('/orders/:id', async (request) => {
  setCorrelation('order_id', request.params.id);
  return loadOrder(request.params.id);
});
await app.listen({ port: 3000 });
```

- Options are the [`@trailscope/node` options](sdk-node.md#options), plus `client` to pass an existing `TrailscopeClient`. Without options it uses the client from `init()`.
- The plugin isn't encapsulated: registered once, it applies to every route of the instance, also those registered in other plugins.
- Fastify parses bodies outside the request's async context; the plugin enters the context again right before the handler, so `setUser()`, log lines and jobs in handlers and `preHandler` hooks see it. `onRequest` and `preParsing` hooks run inside it too; `preValidation` hooks run after body parsing and may not, so call `setUser()` in a `preHandler` hook.
- Routes come from `request.routeOptions.url` (`/orders/:id`). Errors are recorded from the `onError` hook; the response body from `onSend` (string and Buffer payloads; streams are skipped).
- The client IP is `request.ip`, which follows the `trustProxy` setting. Requests aborted by the client are recorded with status 499.

A runnable app is in [examples/fastify](../examples/fastify).
