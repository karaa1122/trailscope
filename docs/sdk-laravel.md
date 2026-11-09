# PHP and Laravel: `trailscope/trailscope`

Request logging for Laravel, job events for Laravel queues, a log channel and Octane support, on top of a framework-agnostic core for any PHP code. Shared behaviour is described in [sdk-spec.md](sdk-spec.md).

| | Supported |
|---|---|
| PHP | 8.2 to 8.5 (Laravel 13 needs 8.3) |
| Laravel | 12 and 13, the versions Laravel still supports |
| Extensions | `json`; `curl` and `zlib` recommended (without them batches go through PHP streams, uncompressed) |

```bash
composer require trailscope/trailscope
```

One package: the core is in the `Trailscope\` namespace and needs nothing else; the Laravel integration (`Trailscope\Laravel\`) is auto-discovered.

## Laravel

Set three variables in `.env`:

```dotenv
TRAILSCOPE_ENDPOINT=https://logs.example.com
TRAILSCOPE_INGEST_KEY=trs_ingest_…
TRAILSCOPE_SERVICE=tasks-api
```

That's all. The service provider:

- adds `Trailscope\Laravel\TrailscopeMiddleware` first in the global middleware stack, so it times everything and sees the final response;
- attaches reported exceptions to the current request or job;
- carries the request's context into queued jobs and records every run;
- registers a `trailscope` log channel;
- hooks into Octane.

Routes come from the matched route's URI: `projects/{project}/tasks` becomes `/projects/:project/tasks`, and `{id?}` becomes `:id`. A request with no matching route (a 404) has no route, and the server derives one from the path. The request ID comes from `X-Request-Id` (or a new one) and is set on the response. A W3C `traceparent` header continues the caller's trace. The client IP is `$request->ip()`, so it follows your trusted proxies.

The authenticated user is recorded by itself when the app has already resolved it (Laravel's `Authenticated` event, or a guard that holds a user at the end of the request). Trailscope never triggers an authentication query. Otherwise, or for a different ID, set it yourself:

```php
use Trailscope\Trailscope;

Trailscope::setUser($user->id);
Trailscope::setCorrelation('project_id', $project->id);  // journey key: the request and everything it triggers
Trailscope::setAttribute('plan', 'pro');
```

Exceptions go through Laravel's exception reporting. A reported exception is attached to the current request, and kept only if the response is a 5xx (type, message and stack). Exceptions Laravel doesn't report (validation errors, 404s, `abort()`) carry no error details; a 5xx without details is grouped as `HTTP <status>`. Outside a request or a job (console commands, scheduled tasks), a reported exception becomes an error event. Laravel also logs every reported exception. When the request or job already carries that exception, the `trailscope` log channel drops the log line, so the error isn't counted twice.

Request bodies are read only when the capture policy keeps them, and at most `max_body_bytes + 1` bytes are read. For multipart forms, the parsed fields are kept and files are listed by name, size and type, never their content. Streamed and file responses are skipped.

To change options, publish the config file:

```bash
php artisan vendor:publish --tag=trailscope-config
```

Set `'middleware' => false` there to add `TrailscopeMiddleware` to your own stack or route groups instead.

## Queues

Nothing to set up. Each queued job's payload carries the context of the request (or job) that dispatched it, in a `trailscope` key. The worker records every run as a `kind=job` event:

- the name is the job class (`$job->resolveName()`), and the attempt is `$job->attempts()` (1-based);
- the event has the duration and the outcome: `ok`, or `error` with the exception's type, message and stack;
- a failed attempt that will be retried is an `error` with level `warn`; the final failure has level `error`;
- a job released back to the queue without an error is `ok`, with the attribute `released`;
- the attributes are `queue`, `connection` and `job_id`;
- the event carries the request ID, trace ID, user and correlation keys of the request that queued the job.

Log lines written during the job carry the job's context. This works with every queue driver and with Horizon. Jobs on the `sync` connection run inside the request and belong to it.

## Logging

The provider registers a `trailscope` log channel. Add it to your stack:

```dotenv
LOG_CHANNEL=stack
LOG_STACK=single,trailscope
TRAILSCOPE_LOG_LEVEL=info   # the default
```

Each record becomes a `log` event named `laravel`, linked to the current request or job. Scalar context values become attributes, and an `exception` in the context becomes the error. To change the channel, define `logging.channels.trailscope` yourself; the provider only adds it when it's missing.

Outside Laravel, push the Monolog 3 handler:

```php
use Monolog\Level;
use Trailscope\Monolog\TrailscopeHandler;

$logger->pushHandler(new TrailscopeHandler(level: Level::Info));   // the event name is the channel
```

## Octane

Octane (Swoole, RoadRunner, FrankenPHP) is supported without configuration. The client lives for the whole worker. The current context is cleared when a request starts and ends, so nothing leaks from one request to the next. The queue is sent at the end of a request once `flush_interval` has passed or `flush_at` items are waiting, on Swoole ticks, and when the worker stops.

## How sending works without background threads

PHP has no background threads, so the SDK can't flush on a timer. Instead:

- Capture calls only append to an in-memory queue, bounded at `max_queue_size` items (oldest dropped). They never touch the network.
- The queue is sent at points where nobody waits:
  - **PHP-FPM:** after the response. Laravel calls `fastcgi_finish_request()` when it sends the response, then runs its terminating callbacks. Trailscope flushes in one of them, so the client already has the full response.
  - **Shutdown:** at the end of every PHP process, for whatever is left.
  - **Queue workers:** between jobs and every few seconds while idle, once `flush_interval` has passed or `flush_at` items are waiting.
  - **Octane:** at the end of requests on the same schedule.
- Each HTTP call has short timeouts (`connect_timeout` 1 s, `timeout` 3 s). One flush spends at most `shutdown_timeout` (5 s).
- A failed send keeps the items for the next flush within the queue limit, and pauses sending with exponential backoff (1 s doubling to 60 s, or the server's `Retry-After`). Other `4xx` responses drop the batch.
- PHP-FPM keeps nothing between requests, so the project settings from `/v1/config` and the retry pause are shared through a small state file (`storage/framework/cache/trailscope-<hash>.json` in Laravel, the system temp directory elsewhere). Without it, every request would fetch the settings, and every request would wait for a timeout while Trailscope is unreachable. Only one process refreshes the settings every 5 minutes. When a send fails, the others skip sending until the pause ends.
- Under PHP-FPM, items that couldn't be sent by the end of the request are dropped, because the process forgets them.
- The client doesn't wait for a flush, but the FPM worker does: normally a few milliseconds on a local network, at most `connect_timeout` once per pause while Trailscope is unreachable. Size the pool with that in mind.
- `php artisan serve` (PHP's built-in server) closes the connection only after the terminating callbacks, so there the client also waits for the flush. Use PHP-FPM, Octane or FrankenPHP in production.
- A full queue of 10,000 requests takes about 20 MB of memory. Lower `max_queue_size` for workers with a tight `memory_limit`.

## Plain PHP

```php
use Trailscope\Trailscope;

$client = Trailscope::init(['service' => 'tasks-worker']);   // reads TRAILSCOPE_ENDPOINT and TRAILSCOPE_INGEST_KEY

// Events, logs, spans
$client->captureEvent('export.created', attributes: ['rows' => 1200]);
$client->log('warn', 'cache miss storm', ['keyspace' => 'sessions']);
$client->captureException($e);
$rows = $client->span('db', 'SELECT tasks', fn () => $db->query('SELECT * FROM tasks'));

// Jobs: inside a request a job belongs to it; across processes, carry the context
$carried = Trailscope::serializeContext();                    // store with the job...
$client->trackJob('report.build', fn () => build($id), attempt: 1, context: $carried);   // ...restore in the worker

// Long-running loops: send between units of work
while ($task = $queue->next()) {
    handle($task);
    $client->flushIfDue();
}

$client->flush(); $client->shutdown(); $client->stats();
```

`Trailscope::init()` makes the client process-wide, for the shortcuts (`Trailscope::setUser()`, `Trailscope::log()`, `Trailscope::span()`…) and the Monolog handler; `new Trailscope\Client([...])` makes an independent one. Every client flushes at shutdown.

To record HTTP requests from another framework, build the context from the request headers, then record the exchange when the response is ready:

```php
use Trailscope\Context;
use Trailscope\HttpExchange;

$ctx = $client->contextFromHeaders(fn (string $name) => $request->getHeaderLine($name) ?: null);
Context::activate($ctx);
$start = microtime(true) * 1000;
// ... handle the request ...
$client->recordHttp($ctx, new HttpExchange(
    timestamp: $start, durationMs: microtime(true) * 1000 - $start,
    method: 'GET', url: '/projects/p1/tasks?page=2', status: 200, route: '/projects/:projectId/tasks',
    reqHeaders: $request->getHeaders(), ip: $clientIp,
));
```

Under PHP-FPM, call `fastcgi_finish_request()` before `$client->flush()` (or before the script ends) so the client doesn't wait.

### Options

Core options are camelCase. In Laravel, `config/trailscope.php` takes the same options in snake_case. Options left `null` fall back to the environment variable, then to the default.

| Option (`config/trailscope.php`) | Env | Default | |
|---|---|---|---|
| `endpoint` | `TRAILSCOPE_ENDPOINT` | `http://localhost:8080` | |
| `ingestKey` (`ingest_key`) | `TRAILSCOPE_INGEST_KEY` | | Without one nothing is sent |
| `service` | `TRAILSCOPE_SERVICE` | `unknown-service` | |
| `environment` | `TRAILSCOPE_ENVIRONMENT` (Laravel also `APP_ENV`) | `production` | |
| `release` | `TRAILSCOPE_RELEASE` | | |
| `enabled` | `TRAILSCOPE_ENABLED=0` disables | `true` | |
| `debug` | `TRAILSCOPE_DEBUG=1` | `false` | Writes SDK diagnostics to the PHP error log |
| `maxQueueSize` | | `10000` | Oldest dropped beyond this |
| `flushInterval` / `flushAt` / `maxBatchSize` | | `1.0` s / `500` / `1000` | When long-running processes flush; batch size |
| `timeout` / `connectTimeout` / `shutdownTimeout` | | `3.0` / `1.0` / `5.0` s | Per HTTP call; longest flush |
| `sampleRate` / `routeSampleRates` | | `1.0` / `[]` | Errors and slow requests are always kept |
| `slowMs` | | `1000` | |
| `captureBodies` | | `'errors'` | `'always'`, `'never'` |
| `maxBodyBytes` | | `8192` | Per body |
| `ignoreRoutes` / `ignoreRoutePatterns` | | health and readiness routes / `[]` | Exact routes or paths / PCRE patterns |
| `requestIdHeader` | | `x-request-id` | |
| `redactHeaders` / `redactKeys` / `redactPatterns` | | | Added to the built-in rules; keys may be dotted paths with `*`; patterns are PCRE |
| `maskIp` | | `true` | |
| `disableDefaultRedaction` | | `false` | Not recommended |
| `remoteConfig` | | `true` | Load sampling and capture settings from `/v1/config` |
| `stateFile` | | system temp directory (Laravel: `storage/framework/cache`) | A file, or a directory for one file per endpoint and key; `false` disables it |
| `beforeSend` | | | Edit an item array or return `null` to drop it; runs in the flush |
| `transport` | | curl, else streams | A `Trailscope\Transport\Transport` |

Laravel-only keys: `middleware` (`true`), `queue` (`true`) and `log_level` (`TRAILSCOPE_LOG_LEVEL`, `info`).

Sampling and capture options set in code or in `config/trailscope.php` win over the project settings from the server.

Runnable app: [examples/laravel](../examples/laravel) (with a database queue).
