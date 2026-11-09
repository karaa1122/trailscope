# Java: `pro.trailscope`

Request logging for Spring Boot and any Jakarta Servlet 6 container, job events for background work, and a Logback appender. Java 17+. The core has no dependencies. Shared behaviour is described in [sdk-spec.md](sdk-spec.md).

| Artifact | What it is |
|---|---|
| `pro.trailscope:trailscope-spring-boot-starter` | Spring Boot 3 auto-configuration. Brings the three below |
| `pro.trailscope:trailscope-servlet` | A servlet `Filter` for Tomcat, Jetty, Undertow and other Jakarta Servlet 6 containers |
| `pro.trailscope:trailscope-logback` | A Logback appender |
| `pro.trailscope:trailscope-java` | The core client, for workers and scripts |

## Spring Boot

```xml
<dependency>
  <groupId>pro.trailscope</groupId>
  <artifactId>trailscope-spring-boot-starter</artifactId>
  <version>0.1.0</version>
</dependency>
```

```properties
# application.properties; the ingest key comes from TRAILSCOPE_INGEST_KEY (or trailscope.ingest-key)
trailscope.endpoint=https://logs.example.com
```

```java
@GetMapping("/orders/{orderId}")
Order get(@PathVariable String orderId) {
  Trailscope.setCorrelation("order_id", orderId);
  return orders.load(orderId);
}
```

That's all the wiring. The starter:

- registers the request filter first in the chain;
- takes the route from Spring MVC: `/orders/{orderId}` becomes `/orders/:orderId`, with the context path in front;
- notes exceptions thrown by controllers, then lets Spring answer as usual. The type, message and stack are recorded when the response is a 5xx;
- records the authenticated principal's name as the user (Spring Security included), unless you call `Trailscope.setUser()`. Turn it off with `trailscope.capture-principal=false`;
- runs `@Async` methods inside the caller's context, with a `TaskDecorator`. If your application defines its own `TaskDecorator`, the starter leaves it alone: wrap with `Trailscope.wrap(runnable)` there;
- attaches the Logback appender to the root logger (see [Logback](#logback));
- skips the actuator endpoints (`trailscope.ignore-actuator=false` to record them);
- creates a `TrailscopeClient` bean you can inject, and flushes it when the application stops.

The service name defaults to `spring.application.name`. Every option is a `trailscope.*` property; the `TRAILSCOPE_*` environment variables still work. Spring Boot 3.x on Spring MVC (servlet stack) is supported. WebFlux isn't.

## Any servlet container

Add `trailscope-servlet` and register the filter first, for `/*`, with async support:

```xml
<!-- web.xml -->
<filter>
  <filter-name>trailscope</filter-name>
  <filter-class>pro.trailscope.servlet.TrailscopeFilter</filter-class>
  <async-supported>true</async-supported>
  <init-param><param-name>service</param-name><param-value>orders-api</param-value></init-param>
</filter>
<filter-mapping>
  <filter-name>trailscope</filter-name>
  <url-pattern>/*</url-pattern>
</filter-mapping>
```

Or in code, for example in a `ServletContainerInitializer`: `ctx.addFilter("trailscope", new TrailscopeFilter(client))`, then `setAsyncSupported(true)` and `addMappingForUrlPatterns(null, false, "/*")` on the registration. Without a client, the filter uses the one from `Trailscope.init()`, or creates one from the `TRAILSCOPE_*` environment variables and its init parameters (`endpoint`, `service`, `environment`, `release`).

The filter:

- reads `X-Request-Id` and `traceparent`, and returns the request ID in `X-Request-Id`;
- makes the context current while the request runs, so `Trailscope.setUser()` and friends work anywhere in it;
- copies at most `maxBodyBytes + 1` bytes of each body while the application reads and writes it. It never reads a body itself and never buffers a large one. Form fields the container parsed are kept; multipart files are listed by name, size and type;
- records the request when the response is done, async requests included;
- records an exception that escapes the servlet as a 500 with its stack;
- reads the route from Spring MVC if it is there, from the servlet's exact mapping, or from `Trailscope.setRoute()`. Otherwise the server derives one from the path. Plug in another source with `filter.routeResolver(request -> …)`;
- takes the client IP from the first `X-Forwarded-For` hop, then `X-Real-IP`, then `getRemoteAddr()`.

A client that goes away is recorded with status 499 when the container reports it, which happens only when a write fails.

## Background jobs

`trackJob` runs a block inside a job context and records it as a `kind=job` event with its duration, outcome and attempt number. Inside a request it belongs to that request: same request ID, trace, user and correlation keys. The block's own exceptions, checked ones included, propagate unchanged.

```java
client.trackJob("report.build", JobOptions.attempt(1), () -> build());
```

The context lives in a `ThreadLocal`, so work handed to another thread needs to take it along:

```java
ExecutorService pool = Trailscope.wrap(Executors.newFixedThreadPool(4)); // tasks run in the submitter's context
pool.submit(() -> client.trackJob("report.build", () -> build()));

executor.execute(Trailscope.wrap(() -> …));                              // one task
```

In Spring Boot, `@Async` methods get the context without any of this.

A failed run that will be retried can say so: `JobOptions.attempt(n).willRetry(true)` records it with level `warn` instead of `error`.

To carry the context to another process (a message queue, a job table), store it with the job and restore it in the worker:

```java
Map<String, Object> carried = Trailscope.serializeContext();   // or serializeContextJson() for a message header
// ... in the worker
client.trackJob("report.build",
    JobOptions.attempt(attempt).context(Trailscope.deserializeContext(carried)),
    () -> build());
```

## Logback

The Spring Boot starter attaches the appender to the root logger. It sends `WARN` and above by default; set `trailscope.logback.level=INFO` for more, or `trailscope.logback.enabled=false` to skip it. Below `WARN`, nothing is sent until the application has started.

Without Spring Boot, add `trailscope-logback` and the appender to `logback.xml`:

```xml
<appender name="TRAILSCOPE" class="pro.trailscope.logback.TrailscopeAppender">
  <level>WARN</level>
</appender>
<root level="INFO">
  <appender-ref ref="TRAILSCOPE" />
</root>
```

Each log line becomes a `log` event named after its logger, with the thread as an attribute, the attached exception as the error, and the current request's or job's context. It uses the client from `Trailscope.init()` (or the starter's). The appender skips Trailscope's own logs, the container's report of an exception that failed a request (the request already carries it), and framework loggers (`org.springframework.`, `org.apache.`, `org.hibernate.`…) below `WARN`. Don't put it behind an `AsyncAppender`: the context lives on the logging thread, and the client never blocks.

## Client

```java
TrailscopeClient client = Trailscope.init(new TrailscopeOptions().service("orders-api")); // process-wide, flushed by a JVM shutdown hook
// new TrailscopeClient(options) for an independent one

// Context of the current request (no-ops outside one)
Trailscope.setUser(userId); Trailscope.setCorrelation("account_id", accountId); Trailscope.setAttribute("plan", "pro");
Trailscope.recordError(e); Trailscope.setRoute("/orders/:id"); Trailscope.currentContext();

// Events, logs and spans
client.captureEvent(Event.of("export.created").attribute("rows", 1200));
client.log(Level.WARN, "cache miss storm");
client.captureException(e);
List<Row> rows = client.span(SpanKind.DB, "SELECT orders", () -> query());
try (Span span = client.startSpan(SpanKind.EXTERNAL, "GET billing")) { … }

client.flush(); client.shutdown(); client.stats();
```

### Options

| Option | Spring property | Env | Default | |
|---|---|---|---|---|
| `endpoint` | `trailscope.endpoint` | `TRAILSCOPE_ENDPOINT` | `http://localhost:8080` | |
| `ingestKey` | `trailscope.ingest-key` | `TRAILSCOPE_INGEST_KEY` | | Without one nothing is sent |
| `service` | `trailscope.service` | `TRAILSCOPE_SERVICE` | `unknown-service` | Spring Boot: `spring.application.name` |
| `environment` | `trailscope.environment` | `TRAILSCOPE_ENVIRONMENT` | `production` | |
| `release` | `trailscope.release` | `TRAILSCOPE_RELEASE` | | |
| `enabled` | `trailscope.enabled` | `TRAILSCOPE_ENABLED=0` disables | `true` | |
| `debug` | `trailscope.debug` | `TRAILSCOPE_DEBUG=1` | `false` | Logs SDK diagnostics on the `pro.trailscope` logger |
| `maxQueueSize` | `trailscope.max-queue-size` | | `10000` | Oldest dropped beyond this |
| `flushInterval` / `flushAt` / `maxBatchSize` | `trailscope.flush-interval` … | | `1s` / `500` / `1000` | |
| `requestTimeout` / `shutdownTimeout` | `trailscope.request-timeout` … | | `10s` / `5s` | |
| `sampleRate` / `routeSampleRates` | `trailscope.sample-rate`, `trailscope.route-sample-rates[/search]` | | `1` / none | Errors and slow requests are always kept |
| `slowMs` | `trailscope.slow-ms` | | `1000` | |
| `captureBodies` | `trailscope.capture-bodies` | | `ERRORS` | `ALWAYS`, `NEVER` |
| `maxBodyBytes` | `trailscope.max-body-bytes` | | `8192` | Per body |
| `ignoreRoutes` | `trailscope.ignore-routes` | | health and readiness routes | Exact routes or paths |
| `ignoreRoutePattern` | `trailscope.ignore-route-patterns` | | | Regular expressions, on top of the list |
| | `trailscope.ignore-actuator` | | `true` | Spring Boot only |
| `requestIdHeader` | `trailscope.request-id-header` | | `x-request-id` | |
| `redactHeaders` / `redactKeys` / `redactPatterns` | `trailscope.redact-headers` … | | | Added to the built-in rules; keys may be dotted paths with `*` |
| `maskIp` | `trailscope.mask-ip` | | `true` | |
| `disableDefaultRedaction` | `trailscope.disable-default-redaction` | | `false` | Not recommended |
| `remoteConfig` | `trailscope.remote-config` | | `true` | Load sampling and capture settings from `/v1/config` |
| `capturePrincipal` | `trailscope.capture-principal` | | `true` | The principal's name as the user |
| `shutdownHook` | | | `true` | `Trailscope.init()` registers a JVM shutdown hook |
| `beforeSend` | | | | Edit an item map or return `null` to drop it; runs in the flush thread |
| | `trailscope.filter.order` | | first | Spring Boot only |
| | `trailscope.logback.enabled` / `.level` | | `true` / `WARN` | Spring Boot only |

Sampling and capture options set here win over the project settings from the server.

Runnable app: [examples/spring-boot](../examples/spring-boot) (an `@Async` job with a retry, Logback, a span).
