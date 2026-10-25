// Minimal Express app wired to Logbox. Start Logbox, then:
//   LOGBOX_INGEST_KEY=lbx_ingest_… pnpm start
//   curl localhost:3000/projects/p1/tasks
import express from 'express';
import { init, logbox, logboxErrorHandler, setCorrelation, setUser } from '@logbox/express';

const client = init({ service: 'tasks-api', endpoint: process.env.LOGBOX_ENDPOINT ?? 'http://localhost:8080' });

const app = express();
app.use(logbox(client));
app.use(express.json());

// Pretend authentication: attach the caller to everything this request does.
app.use((req, _res, next) => {
  const user = req.get('x-user-id');
  if (user) setUser(user);
  next();
});

app.get('/health', (_req, res) => res.send('ok')); // ignored by default

app.get('/projects/:projectId/tasks', (req, res) => {
  setCorrelation('project_id', req.params.projectId);
  res.json([{ id: 't1', title: 'Write the brief', done: false }]);
});

app.post('/projects/:projectId/tasks', async (req, res) => {
  setCorrelation('project_id', req.params.projectId);
  const task = { id: `t${Date.now()}`, ...req.body };
  // A background job linked to this request (same request ID, user and project).
  void client.trackJob('task.notify', async () => {
    await new Promise((r) => setTimeout(r, 50));
    client.log('info', `notified watchers of ${task.id}`);
  });
  res.status(201).json(task);
});

app.get('/projects/:projectId/report', async (req, res) => {
  setCorrelation('project_id', req.params.projectId);
  const rows = await client.span('db', 'SELECT tasks', () => new Promise((r) => setTimeout(() => r(42), 30)));
  res.json({ rows });
});

app.get('/projects/:projectId/export', () => {
  throw new TypeError("Cannot read properties of undefined (reading 'columns')");
});

app.get('/slow', async (_req, res) => {
  await new Promise((r) => setTimeout(r, 1200));
  res.json({ ok: true });
});

app.use(logboxErrorHandler());
app.use((err, _req, res, _next) => {
  res.status(500).json({ error: 'internal error' });
});

const port = Number(process.env.PORT ?? 3000);
const server = app.listen(port, () => console.log(`listening on http://localhost:${port}`));
process.on('SIGTERM', () => server.close(() => void client.shutdown()));
process.on('SIGINT', () => server.close(() => void client.shutdown()));
