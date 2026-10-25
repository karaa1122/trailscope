// Minimal Fastify app wired to Logbox. Start Logbox, then:
//   LOGBOX_INGEST_KEY=lbx_ingest_… pnpm start
//   curl localhost:3000/projects/p1/tasks
import Fastify from 'fastify';
import { init, logbox, setCorrelation, setUser } from '@logbox/fastify';

const client = init({ service: 'tasks-api-fastify', endpoint: process.env.LOGBOX_ENDPOINT ?? 'http://localhost:8080' });

const app = Fastify();
await app.register(logbox, { client });

// Pretend authentication: attach the caller to everything this request does.
app.addHook('preHandler', async (request) => {
  const user = request.headers['x-user-id'];
  if (typeof user === 'string') setUser(user);
});

app.get('/health', async () => 'ok'); // ignored by default

app.get('/projects/:projectId/tasks', async (request) => {
  setCorrelation('project_id', request.params.projectId);
  return [{ id: 't1', title: 'Write the brief', done: false }];
});

app.post('/projects/:projectId/tasks', async (request, reply) => {
  setCorrelation('project_id', request.params.projectId);
  const task = { id: `t${Date.now()}`, ...request.body };
  void client.trackJob('task.notify', async () => {
    await new Promise((r) => setTimeout(r, 50));
    client.log('info', `notified watchers of ${task.id}`);
  });
  return reply.code(201).send(task);
});

app.get('/projects/:projectId/export', async () => {
  throw new TypeError("Cannot read properties of undefined (reading 'columns')");
});

const port = Number(process.env.PORT ?? 3000);
await app.listen({ port });
console.log(`listening on http://localhost:${port}`);
const stop = async () => {
  await app.close();
  await client.shutdown();
};
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
