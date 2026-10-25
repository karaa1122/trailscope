# Express: `@logbox/express`

Request logging, error capture and context for Express 4 and 5. Built on [`@logbox/node`](sdk-node.md); shared behaviour is in [sdk-spec.md](sdk-spec.md).

```bash
npm install @logbox/express
```

```ts
import express from 'express';
import { init, logbox, logboxErrorHandler, setCorrelation, setUser } from '@logbox/express';

init({ service: 'orders-api' });     // LOGBOX_INGEST_KEY from the environment

const app = express();
app.use(logbox());                   // first, so it times everything
app.use(express.json());
app.get('/orders/:id', (req, res) => {
  setCorrelation('order_id', req.params.id);
  res.json(loadOrder(req.params.id));
});
app.use(logboxErrorHandler());       // after the routes, before your own error handlers
```

- `logbox(clientOrOptions?)` is the middleware. Pass a `LogboxClient`, or options to create the process-wide client. Without arguments it uses the client from `init()`, or creates one from the `LOGBOX_*` variables.
- `logboxErrorHandler()` records the error (type, message and stack, kept when the response is a 5xx) and passes it on with `next(err)`, so your own error handlers still answer.
- Routes are recorded as Express defines them: `req.baseUrl + req.route.path`, so a router mounted at `/orders` with `/:id` gives `/orders/:id`.
- The request body is the parsed `req.body`, so register body parsers as usual; the response body is copied as it is written.
- The client IP is `req.ip`, which follows Express's `trust proxy` setting.
- Call `setUser()` in your authentication middleware, and `setCorrelation()` wherever a journey key (an order, an account, a session) is known.

Everything from `@logbox/node` used in request handlers is re-exported: `init`, `getClient`, `LogboxClient`, `setUser`, `setCorrelation`, `setAttribute`, `recordError`, `currentContext`, `contextOf`.

A runnable app is in [examples/express](../examples/express).
