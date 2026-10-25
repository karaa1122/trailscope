# @logbox/express

[Logbox](https://github.com/karaa1122/LogBox) middleware for Express 4 and 5: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, without blocking your request path.

## Quickstart

```ts
import express from 'express';
import { init, logbox, logboxErrorHandler, setUser } from '@logbox/express';

init({ service: 'orders-api' }); // reads LOGBOX_ENDPOINT and LOGBOX_INGEST_KEY

const app = express();
app.use(logbox());
app.use(express.json());
app.get('/orders/:id', (req, res) => res.json({ id: req.params.id }));
app.use(logboxErrorHandler());
app.listen(3000);
```

Call `setUser(id)` in your auth middleware and `setCorrelation('order_id', id)` where a journey key is known.

Full documentation: [docs/sdk-express.md](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-express.md). Options: [`@logbox/node`](https://github.com/karaa1122/LogBox/blob/main/docs/sdk-node.md#options).

## License

Apache-2.0
