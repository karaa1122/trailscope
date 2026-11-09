# @trailscope/express

[Trailscope](https://github.com/karaa1122/trailscope) middleware for Express 4 and 5: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, without blocking your request path.

## Quickstart

```ts
import express from 'express';
import { init, trailscope, trailscopeErrorHandler, setUser } from '@trailscope/express';

init({ service: 'orders-api' }); // reads TRAILSCOPE_ENDPOINT and TRAILSCOPE_INGEST_KEY

const app = express();
app.use(trailscope());
app.use(express.json());
app.get('/orders/:id', (req, res) => res.json({ id: req.params.id }));
app.use(trailscopeErrorHandler());
app.listen(3000);
```

Call `setUser(id)` in your auth middleware and `setCorrelation('order_id', id)` where a journey key is known.

Full documentation: [docs/sdk-express.md](https://github.com/karaa1122/trailscope/blob/main/docs/sdk-express.md). Options: [`@trailscope/node`](https://github.com/karaa1122/trailscope/blob/main/docs/sdk-node.md#options).

## License

Apache-2.0
