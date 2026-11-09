package dev.trailscope;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.locks.ReentrantLock;
import java.util.function.Function;
import java.util.function.UnaryOperator;
import java.util.regex.Pattern;
import java.util.zip.GZIPOutputStream;

/**
 * Ships requests, spans, job runs and log lines to Trailscope.
 *
 * <p>Every capture method is cheap and never blocks on the network: items go into a bounded
 * in-memory queue (the oldest are dropped when it is full) and a daemon thread sends them as
 * gzipped NDJSON. Redaction and serialization happen in that thread, off the request path. No
 * method throws because of Trailscope; problems are reported only on the {@code dev.trailscope}
 * logger when {@code debug} is on.
 */
public final class TrailscopeClient {
  /** Routes that aren't recorded unless {@code ignoreRoutes} or the server says otherwise. */
  public static final List<String> DEFAULT_IGNORE_ROUTES =
      List.of("/health", "/healthz", "/ready", "/readyz", "/livez");

  private static final long CONFIG_REFRESH_NANOS = Duration.ofMinutes(5).toNanos();
  private static final String USER_AGENT = "trailscope-java/" + Trailscope.VERSION;
  private static final System.Logger LOG = System.getLogger("dev.trailscope");

  private final String endpoint;
  private final String ingestKey;
  private final String service;
  private final String environment;
  private final String release;
  private final boolean enabled;
  private final boolean debug;
  private final int maxQueueSize;
  private final long flushIntervalMillis;
  private final int flushAt;
  private final int maxBatchSize;
  private final Duration requestTimeout;
  private final Duration shutdownTimeout;
  private final String requestIdHeader;
  private final boolean remoteConfig;
  private final boolean capturePrincipal;
  private final UnaryOperator<Map<String, Object>> beforeSend;
  private final List<Pattern> ignoreRoutePatterns;
  private final Set<String> explicit = new HashSet<>();
  private final List<String> redactHeaders;
  private final List<String> redactKeys;
  private final List<Pattern> redactPatterns;
  private final boolean disableDefaultRedaction;

  // Settings the server may change (see applyRemoteConfig).
  private volatile double sampleRate;
  private volatile Map<String, Double> routeSampleRates;
  private volatile double slowMs;
  private volatile BodyCapture captureBodies;
  private volatile int maxBodyBytes;
  private volatile List<String> ignoreRoutes;
  private volatile Redactor redactor;

  private final Object lock = new Object();
  private final ArrayDeque<Map<String, Object>> queue = new ArrayDeque<>();
  private boolean wake; // guarded by lock
  private long dropped; // guarded by lock
  private final AtomicLong sent = new AtomicLong();
  private final AtomicLong rejected = new AtomicLong();
  private final AtomicLong failedBatches = new AtomicLong();
  private final ReentrantLock sendLock = new ReentrantLock();
  private int failures; // guarded by sendLock
  private volatile long backoffUntil; // System.nanoTime()
  private volatile boolean backingOff;
  private volatile boolean warnedAuth;
  private volatile boolean closed;
  private volatile boolean stopping;
  private final HttpClient http;
  private final URI ingestUri;
  private final URI configUri;
  private final Thread worker;

  /** Creates a client; {@link Trailscope#init(TrailscopeOptions)} also makes it process-wide. */
  public TrailscopeClient(TrailscopeOptions o) {
    String ep = firstNonEmpty(o.endpoint, env("TRAILSCOPE_ENDPOINT"), "http://localhost:8080");
    while (ep.endsWith("/")) {
      ep = ep.substring(0, ep.length() - 1);
    }
    this.endpoint = ep;
    this.ingestKey = firstNonEmpty(o.ingestKey, env("TRAILSCOPE_INGEST_KEY"), "");
    this.service = firstNonEmpty(o.service, env("TRAILSCOPE_SERVICE"), "unknown-service");
    this.environment = firstNonEmpty(o.environment, env("TRAILSCOPE_ENVIRONMENT"), "production");
    this.release = firstNonEmpty(o.release, env("TRAILSCOPE_RELEASE"), "");
    this.enabled = o.enabled != null ? o.enabled : !"0".equals(env("TRAILSCOPE_ENABLED"));
    this.debug = o.debug != null ? o.debug : "1".equals(env("TRAILSCOPE_DEBUG"));
    this.maxQueueSize = Math.max(1, o.maxQueueSize);
    this.flushIntervalMillis = Math.max(10, millis(o.flushInterval, 1000));
    this.flushAt = Math.max(1, o.flushAt);
    this.maxBatchSize = Math.min(10_000, Math.max(1, o.maxBatchSize));
    this.requestTimeout = o.requestTimeout != null ? o.requestTimeout : Duration.ofSeconds(10);
    this.shutdownTimeout = o.shutdownTimeout != null ? o.shutdownTimeout : Duration.ofSeconds(5);
    this.requestIdHeader =
        firstNonEmpty(o.requestIdHeader, null, "x-request-id").toLowerCase(Locale.ROOT);
    this.remoteConfig = o.remoteConfig;
    this.capturePrincipal = o.capturePrincipal;
    this.beforeSend = o.beforeSend;
    this.ignoreRoutePatterns = List.copyOf(o.ignoreRoutePatterns);

    this.sampleRate = o.sampleRate != null ? clamp(o.sampleRate) : 1.0;
    this.routeSampleRates = clampRates(o.routeSampleRates);
    this.slowMs = o.slowMs != null ? o.slowMs : 1000.0;
    this.captureBodies = o.captureBodies != null ? o.captureBodies : BodyCapture.ERRORS;
    this.maxBodyBytes = o.maxBodyBytes != null ? Math.max(0, o.maxBodyBytes) : 8192;
    this.ignoreRoutes =
        o.ignoreRoutes != null ? List.copyOf(o.ignoreRoutes) : DEFAULT_IGNORE_ROUTES;
    this.redactHeaders = List.copyOf(o.redactHeaders);
    this.redactKeys = List.copyOf(o.redactKeys);
    this.redactPatterns = List.copyOf(o.redactPatterns);
    this.disableDefaultRedaction = o.disableDefaultRedaction;
    boolean mask = o.maskIp != null ? o.maskIp : true;
    this.redactor = newRedactor(mask);
    if (o.sampleRate != null) {
      explicit.add("sampleRate");
    }
    if (o.routeSampleRates != null) {
      explicit.add("routeSampleRates");
    }
    if (o.slowMs != null) {
      explicit.add("slowMs");
    }
    if (o.captureBodies != null) {
      explicit.add("bodyCapture");
    }
    if (o.maxBodyBytes != null) {
      explicit.add("maxBodyBytes");
    }
    if (o.ignoreRoutes != null) {
      explicit.add("ignoreRoutes");
    }
    if (o.maskIp != null) {
      explicit.add("maskIp");
    }

    HttpClient h = null;
    URI ingest = null;
    URI config = null;
    if (enabled && ingestKey.isEmpty()) {
      debug("no ingest key configured (TRAILSCOPE_INGEST_KEY); events will not be sent");
    } else if (enabled) {
      try {
        ingest = URI.create(endpoint + "/v1/ingest");
        config = URI.create(endpoint + "/v1/config");
        h =
            HttpClient.newBuilder()
                .version(HttpClient.Version.HTTP_1_1)
                .connectTimeout(requestTimeout)
                .build();
      } catch (RuntimeException e) {
        h = null;
        LOG.log(System.Logger.Level.WARNING, "trailscope: disabled, bad endpoint: " + e);
      }
    }
    this.http = h;
    this.ingestUri = ingest;
    this.configUri = config;
    if (h != null) {
      // Started now so /v1/config is loaded before the first request.
      Thread t = new Thread(this::run, "trailscope-flush");
      t.setDaemon(true);
      this.worker = t;
      t.start();
    } else {
      this.worker = null;
    }
  }

  // ---- read-only accessors -------------------------------------------------------------------

  /**
   * Whether the client records anything: it is enabled, has an ingest key and isn't shut down.
   * Integrations skip their work when it doesn't.
   */
  public boolean isEnabled() {
    return http != null && !closed;
  }

  public String service() {
    return service;
  }

  public String environment() {
    return environment;
  }

  public String endpoint() {
    return endpoint;
  }

  /** The lower-cased header that carries the request ID. */
  public String requestIdHeader() {
    return requestIdHeader;
  }

  public BodyCapture captureBodies() {
    return captureBodies;
  }

  public int maxBodyBytes() {
    return maxBodyBytes;
  }

  public double slowMs() {
    return slowMs;
  }

  public boolean capturePrincipal() {
    return capturePrincipal;
  }

  // ---- capture -------------------------------------------------------------------------------

  /** Queues a request in the wire format (camelCase keys, see docs/ingest-protocol.md). */
  public void captureRequest(Map<String, ?> item) {
    try {
      Map<String, Object> copy = new LinkedHashMap<>(item);
      copy.put("type", "request");
      if (copy.get("service") == null) {
        copy.put("service", service);
      }
      enqueue(copy);
    } catch (RuntimeException e) {
      debug("captureRequest failed: " + e);
    }
  }

  /** Records an event; IDs, user and correlation come from the event's or the current context. */
  public void captureEvent(Event e) {
    if (!isEnabled() || e == null) {
      return;
    }
    try {
      TrailscopeContext ctx = e.context != null ? e.context : Trailscope.currentContext();
      Map<String, Object> item = new LinkedHashMap<>();
      item.put("type", "event");
      item.put("service", service);
      item.put("kind", e.kind.wire());
      item.put("name", e.name);
      item.put("level", e.level != null ? e.level.wire() : null);
      item.put("message", e.message);
      item.put("status", e.status != null ? e.status.wire() : null);
      item.put("durationMs", e.durationMs != null ? round2(e.durationMs) : null);
      item.put("attempt", e.attempt);
      item.put("attributes", e.attributes != null ? new LinkedHashMap<>(e.attributes) : null);
      item.put("timestamp", e.timestamp);
      if (ctx != null) {
        item.put("requestId", ctx.requestId());
        item.put("traceId", ctx.traceId());
        item.put("userId", ctx.userId());
        item.put("correlation", ctx.correlationOrNull());
      }
      item.put("errorType", e.errorType);
      item.put("errorMessage", e.errorMessage);
      item.put("errorStack", e.errorStack);
      enqueue(item);
    } catch (RuntimeException ex) {
      debug("captureEvent failed: " + ex);
    }
  }

  /** A custom event with attributes. */
  public void captureEvent(String name, Map<String, ?> attributes) {
    captureEvent(Event.of(name).attributes(attributes));
  }

  /** An application log line, linked to the current request or job if there is one. */
  public void log(Level level, String message) {
    log(level, message, "log", null, null);
  }

  /**
   * An application log line.
   *
   * @param name the event name, usually the logger name
   * @param attributes extra attributes, or null
   * @param error an exception to attach, or null
   */
  public void log(
      Level level, String message, String name, Map<String, ?> attributes, Throwable error) {
    captureEvent(
        Event.of(name != null ? name : "log")
            .kind(EventKind.LOG)
            .level(level)
            .message(message)
            .attributes(attributes)
            .error(error));
  }

  /** Reports an exception as an error event (outside an HTTP request). */
  public void captureException(Throwable error) {
    captureException(error, "exception", null);
  }

  /** Reports an exception as an error event, with a name and attributes. */
  public void captureException(Throwable error, String name, Map<String, ?> attributes) {
    if (error == null) {
      return;
    }
    captureEvent(
        Event.of(name != null ? name : "exception")
            .kind(EventKind.LOG)
            .level(Level.ERROR)
            .status(EventStatus.ERROR)
            .message(Errors.message(error))
            .attributes(attributes)
            .error(error));
  }

  /** Starts a span of the current request; close it to record it. */
  public Span startSpan(SpanKind kind, String name) {
    return new Span(this, Trailscope.currentContext(), kind, name);
  }

  /** Times {@code fn} as a span of the current request and returns its result. */
  public <T, E extends Throwable> T span(SpanKind kind, String name, ThrowingSupplier<T, E> fn)
      throws E {
    Span span = startSpan(kind, name);
    try {
      return fn.get();
    } catch (Throwable t) {
      span.fail(t);
      throw t;
    } finally {
      span.end();
    }
  }

  /** Times {@code fn} as a span of the current request. */
  public <E extends Throwable> void span(SpanKind kind, String name, ThrowingRunnable<E> fn)
      throws E {
    Span span = startSpan(kind, name);
    try {
      fn.run();
    } catch (Throwable t) {
      span.fail(t);
      throw t;
    } finally {
      span.end();
    }
  }

  void recordSpan(
      TrailscopeContext ctx,
      SpanKind kind,
      String name,
      double start,
      double durationMs,
      boolean failed,
      Map<String, Object> attributes) {
    if (!isEnabled()) {
      return;
    }
    try {
      Map<String, Object> item = new LinkedHashMap<>();
      item.put("type", "span");
      item.put("service", service);
      item.put("traceId", ctx != null ? ctx.traceId() : Ids.newTraceId());
      item.put("spanId", Ids.newSpanId());
      item.put("parentSpanId", ctx != null ? ctx.spanId() : null);
      item.put("requestId", ctx != null ? ctx.requestId() : null);
      item.put("kind", (kind != null ? kind : SpanKind.FN).wire());
      item.put("name", name);
      item.put("start", Math.round(start * 1000) / 1000.0);
      item.put("durationMs", round2(durationMs));
      item.put("status", failed ? "error" : "ok");
      item.put("attributes", attributes);
      enqueue(item);
    } catch (RuntimeException e) {
      debug("span failed: " + e);
    }
  }

  /**
   * Runs a background job inside a Trailscope context and records it as a {@code kind=job} event
   * with its duration, outcome and attempt number. A job started inside a request belongs to it.
   * The job's own exceptions propagate unchanged.
   *
   * <pre>{@code
   * executor.submit(Trailscope.wrap(() ->
   *     client.trackJob("report.build", JobOptions.attempt(1), () -> build())));
   * }</pre>
   */
  public <T, E extends Throwable> T trackJob(
      String name, JobOptions options, ThrowingSupplier<T, E> fn) throws E {
    JobOptions opts = options != null ? options : JobOptions.create();
    TrailscopeContext parent = opts.context != null ? opts.context : Trailscope.currentContext();
    TrailscopeContext ctx =
        new TrailscopeContext(
            parent != null ? parent.requestId() : Ids.newRequestId(),
            parent != null ? parent.traceId() : Ids.newTraceId(),
            Ids.newSpanId(),
            parent != null ? parent.spanId() : null);
    if (parent != null) {
      ctx.setUser(parent.userId());
      Map<String, String> correlation = parent.correlationOrNull();
      if (correlation != null) {
        correlation.forEach(ctx::setCorrelation);
      }
    }
    long timestamp = System.currentTimeMillis();
    long t0 = System.nanoTime();
    T result;
    try (ContextScope scope = Trailscope.useContext(ctx)) {
      result = fn.get();
    } catch (Throwable t) {
      recordJob(name, opts, ctx, timestamp, t0, t);
      throw t;
    }
    recordJob(name, opts, ctx, timestamp, t0, null);
    return result;
  }

  /** {@link #trackJob(String, JobOptions, ThrowingSupplier)} with default options. */
  public <T, E extends Throwable> T trackJob(String name, ThrowingSupplier<T, E> fn) throws E {
    return trackJob(name, null, fn);
  }

  /** {@link #trackJob(String, JobOptions, ThrowingSupplier)} for a job without a result. */
  public <E extends Throwable> void trackJob(
      String name, JobOptions options, ThrowingRunnable<E> fn) throws E {
    trackJob(
        name,
        options,
        (ThrowingSupplier<Object, E>)
            () -> {
              fn.run();
              return null;
            });
  }

  /** {@link #trackJob(String, JobOptions, ThrowingRunnable)} with default options. */
  public <E extends Throwable> void trackJob(String name, ThrowingRunnable<E> fn) throws E {
    trackJob(name, null, fn);
  }

  private void recordJob(
      String name, JobOptions o, TrailscopeContext ctx, long timestamp, long t0, Throwable error) {
    double duration = (System.nanoTime() - t0) / 1e6;
    Event e =
        Event.of(name)
            .kind(EventKind.JOB)
            .durationMs(duration)
            .timestamp(timestamp)
            .context(ctx)
            .attributes(o.attributes);
    if (o.attempt != null) {
      e.attempt(o.attempt);
    }
    if (error == null) {
      e.level(Level.INFO).status(EventStatus.OK).message(name + " completed");
    } else {
      e.level(o.willRetry ? Level.WARN : Level.ERROR)
          .status(EventStatus.ERROR)
          .message(name + " failed: " + Errors.message(error))
          .error(error);
    }
    captureEvent(e);
  }

  // ---- helpers for framework integrations ----------------------------------------------------

  /**
   * Builds the context of an incoming request from its headers: the request ID header if it looks
   * sane, and the W3C {@code traceparent}.
   */
  public TrailscopeContext contextFromHeaders(Function<String, String> header) {
    Ids.TraceParent tp = Ids.parseTraceparent(header.apply("traceparent"));
    String requestId = Ids.acceptRequestId(header.apply(requestIdHeader));
    return new TrailscopeContext(
        requestId != null ? requestId : Ids.newRequestId(),
        tp != null ? tp.traceId() : Ids.newTraceId(),
        Ids.newSpanId(),
        tp != null ? tp.parentSpanId() : null);
  }

  /** Whether a route or path is in the ignore list (exact) or matches an ignore pattern. */
  public boolean isIgnored(String routeOrPath) {
    if (routeOrPath == null || routeOrPath.isEmpty()) {
      return false;
    }
    if (ignoreRoutes.contains(routeOrPath)) {
      return true;
    }
    for (Pattern p : ignoreRoutePatterns) {
      if (p.matcher(routeOrPath).find()) {
        return true;
      }
    }
    return false;
  }

  /** Whether bodies should be kept for a request with this outcome. */
  public boolean shouldCaptureBodies(int status, double durationMs) {
    return Bodies.shouldCapture(captureBodies, status, durationMs, slowMs);
  }

  /**
   * The sample rate to record when the request is kept, or 0 when it is dropped. Errors (status
   * 400 or more) and slow requests are always kept.
   */
  public double sample(String route, int status, double durationMs, boolean hasError) {
    if (status >= 400 || hasError || durationMs >= slowMs) {
      return 1.0;
    }
    Double perRoute = route != null ? routeSampleRates.get(route) : null;
    double rate = perRoute != null ? perRoute : sampleRate;
    if (rate >= 1) {
      return 1.0;
    }
    return ThreadLocalRandom.current().nextDouble() < rate ? rate : 0.0;
  }

  /**
   * Records a finished HTTP exchange. Applies the ignore list, sampling and the body capture
   * policy. Error details are kept only for 5xx responses; a handled 4xx is an expected outcome.
   */
  public void recordHttp(TrailscopeContext ctx, HttpExchange ex) {
    if (!isEnabled() || ctx == null || ex == null) {
      return;
    }
    try {
      String url = ex.url != null ? ex.url : "/";
      int q = url.indexOf('?');
      String path = q < 0 ? url : url.substring(0, q);
      String query = q < 0 ? "" : url.substring(q + 1);
      if (path.isEmpty()) {
        path = "/";
      }
      String route = ex.route != null ? ex.route : ctx.route();
      if (isIgnored(route) || isIgnored(path)) {
        return;
      }
      int status = ex.status;
      double durationMs = round2(ex.durationMs);
      Throwable failed = null;
      if (status >= 500) {
        Throwable raw = ex.error != null ? ex.error : ctx.error();
        if (raw != null) {
          failed = Errors.unwrap(raw);
          Trailscope.markRecorded(raw);
          Trailscope.markRecorded(failed);
        }
      }
      double rate = sample(route != null ? route : path, status, durationMs, failed != null);
      if (rate == 0) {
        return;
      }
      Map<String, String> reqHeaders = lowerCase(ex.reqHeaders);
      Map<String, String> resHeaders = lowerCase(ex.resHeaders);
      Map<String, Object> item = new LinkedHashMap<>();
      item.put("type", "request");
      item.put("service", service);
      item.put("timestamp", ex.timestamp > 0 ? ex.timestamp : System.currentTimeMillis());
      item.put("requestId", ctx.requestId());
      item.put("traceId", ctx.traceId());
      item.put("spanId", ctx.spanId());
      item.put("method", ex.method != null ? ex.method.toUpperCase(Locale.ROOT) : "GET");
      item.put("route", route);
      item.put("path", path);
      item.put("query", query.isEmpty() ? null : query);
      item.put("status", status);
      item.put("durationMs", durationMs);
      item.put("reqSize", ex.reqSize);
      item.put("resSize", ex.resSize);
      item.put("userId", ctx.userId());
      item.put("ip", ex.ip);
      item.put("userAgent", reqHeaders.get("user-agent"));
      item.put("reqHeaders", reqHeaders);
      item.put("resHeaders", resHeaders);
      item.put("attributes", ctx.attributesOrNull());
      item.put("correlation", ctx.correlationOrNull());
      item.put("sampleRate", rate < 1 ? rate : null);
      if (shouldCaptureBodies(status, durationMs)) {
        int limit = maxBodyBytes;
        Bodies.Captured rq = Bodies.serialize(ex.reqBody, reqHeaders.get("content-type"), limit);
        String encoding = resHeaders.get("content-encoding");
        Bodies.Captured rs =
            encoding != null && !encoding.isEmpty() && !encoding.equalsIgnoreCase("identity")
                ? null
                : Bodies.serialize(ex.resBody, resHeaders.get("content-type"), limit);
        item.put("reqBody", rq != null ? rq.text() : null);
        item.put("resBody", rs != null ? rs.text() : null);
        if ((rq != null && rq.truncated()) || (rs != null && rs.truncated())) {
          item.put("bodyTruncated", true);
        }
        // Said separately so a cut response alone doesn't block replaying the request.
        item.put("reqBodyTruncated", rq != null && rq.truncated());
      }
      if (failed != null) {
        item.put("errorType", Errors.type(failed));
        item.put("errorMessage", Errors.message(failed));
        item.put("errorStack", Errors.stack(failed));
      }
      enqueue(item);
    } catch (RuntimeException e) {
      debug("could not record request: " + e);
    }
  }

  private static Map<String, String> lowerCase(Map<String, String> headers) {
    Map<String, String> out = new LinkedHashMap<>();
    if (headers == null) {
      return out;
    }
    for (Map.Entry<String, String> e : headers.entrySet()) {
      if (e.getKey() == null || e.getValue() == null) {
        continue;
      }
      String name = e.getKey().toLowerCase(Locale.ROOT);
      out.merge(name, e.getValue(), (a, b) -> a + ", " + b);
    }
    return out;
  }

  // ---- lifecycle -----------------------------------------------------------------------------

  public ClientStats stats() {
    long queued;
    long drop;
    synchronized (lock) {
      queued = queue.size();
      drop = dropped;
    }
    return new ClientStats(queued, drop, sent.get(), rejected.get(), failedBatches.get());
  }

  /** Sends everything queued, in the calling thread. */
  public void flush() {
    if (enabled && http != null) {
      try {
        drain();
      } catch (RuntimeException e) {
        debug("flush failed: " + e);
      }
    }
  }

  /** Stops the background thread after a final flush, waiting at most the shutdown timeout. */
  public void shutdown() {
    shutdown(shutdownTimeout);
  }

  /** Stops the background thread after a final flush, waiting at most {@code timeout}. */
  public void shutdown(Duration timeout) {
    if (closed) {
      return;
    }
    closed = true;
    Thread t = worker;
    if (t == null || !t.isAlive()) {
      return;
    }
    backingOff = false;
    synchronized (lock) {
      stopping = true;
      lock.notifyAll();
    }
    try {
      t.join(Math.max(1, timeout.toMillis()));
    } catch (InterruptedException e) {
      Thread.currentThread().interrupt();
    }
  }

  /** Fetches the project's SDK settings from {@code /v1/config} and applies them. */
  public void refreshConfig() {
    if (http == null) {
      return;
    }
    try {
      HttpRequest req =
          HttpRequest.newBuilder(configUri)
              .timeout(requestTimeout)
              .header("authorization", "Bearer " + ingestKey)
              .header("user-agent", USER_AGENT)
              .GET()
              .build();
      HttpResponse<String> resp = http.send(req, HttpResponse.BodyHandlers.ofString());
      if (resp.statusCode() != 200) {
        debug("could not load /v1/config: HTTP " + resp.statusCode());
        return;
      }
      if (Json.parse(resp.body()) instanceof Map<?, ?> config) {
        @SuppressWarnings("unchecked")
        Map<String, ?> c = (Map<String, ?>) config;
        applyRemoteConfig(c);
      }
    } catch (IOException | RuntimeException e) {
      debug("could not load /v1/config: " + e);
    } catch (InterruptedException e) {
      Thread.currentThread().interrupt();
    }
  }

  /** Applies project settings to every option the application didn't set itself. */
  public void applyRemoteConfig(Map<String, ?> c) {
    if (!explicit.contains("sampleRate") && c.get("sampleRate") instanceof Number n) {
      sampleRate = clamp(n.doubleValue());
    }
    if (!explicit.contains("routeSampleRates")
        && c.get("routeSampleRates") instanceof Map<?, ?> m) {
      Map<String, Double> rates = new LinkedHashMap<>();
      for (Map.Entry<?, ?> e : m.entrySet()) {
        if (e.getValue() instanceof Number n) {
          rates.put(String.valueOf(e.getKey()), clamp(n.doubleValue()));
        }
      }
      routeSampleRates = Collections.unmodifiableMap(rates);
    }
    if (!explicit.contains("slowMs")
        && c.get("slowMs") instanceof Number n
        && n.doubleValue() > 0) {
      slowMs = n.doubleValue();
    }
    if (!explicit.contains("bodyCapture") && c.get("bodyCapture") instanceof String s) {
      BodyCapture b = BodyCapture.parse(s);
      if (b != null) {
        captureBodies = b;
      }
    }
    if (!explicit.contains("maxBodyBytes")
        && c.get("maxBodyBytes") instanceof Number n
        && n.longValue() >= 0) {
      maxBodyBytes = (int) Math.min(Integer.MAX_VALUE - 8, n.longValue());
    }
    if (!explicit.contains("ignoreRoutes") && c.get("ignoreRoutes") instanceof List<?> routes) {
      List<String> out = new ArrayList<>();
      for (Object r : routes) {
        if (r instanceof String s) {
          out.add(s);
        }
      }
      ignoreRoutes = List.copyOf(out);
    }
    if (!explicit.contains("maskIp")
        && c.get("maskIp") instanceof Boolean mask
        && mask != redactor.maskIp) {
      redactor = newRedactor(mask);
    }
  }

  // ---- internals -----------------------------------------------------------------------------

  private Redactor newRedactor(boolean maskIp) {
    return new Redactor(
        redactHeaders, redactKeys, redactPatterns, maskIp, disableDefaultRedaction);
  }

  void enqueue(Map<String, Object> item) {
    if (http == null || closed) {
      return;
    }
    if (!"span".equals(item.get("type"))) {
      Object env = item.get("environment");
      if (env == null || "".equals(env)) {
        item.put("environment", environment);
      }
      if (!release.isEmpty() && item.get("release") == null) {
        item.put("release", release);
      }
      if (item.get("timestamp") == null) {
        item.put("timestamp", System.currentTimeMillis());
      }
    }
    synchronized (lock) {
      if (queue.size() >= maxQueueSize) {
        queue.pollFirst();
        dropped++;
      }
      queue.addLast(item);
      if (queue.size() >= flushAt && !wake) {
        wake = true;
        lock.notifyAll();
      }
    }
  }

  private void run() {
    long nextConfig = System.nanoTime();
    while (true) {
      if (remoteConfig && System.nanoTime() - nextConfig >= 0) {
        refreshConfig();
        nextConfig = System.nanoTime() + CONFIG_REFRESH_NANOS;
      }
      synchronized (lock) {
        if (!wake && !stopping) {
          try {
            lock.wait(flushIntervalMillis);
          } catch (InterruptedException e) {
            // Interrupts don't stop the flush thread; shutdown() does.
            Thread.interrupted();
          }
        }
        wake = false;
      }
      try {
        drain();
      } catch (RuntimeException | Error e) {
        debug("flush failed: " + e);
      }
      if (stopping) {
        return;
      }
    }
  }

  private List<Map<String, Object>> take(int n) {
    synchronized (lock) {
      int count = Math.min(n, queue.size());
      List<Map<String, Object>> out = new ArrayList<>(count);
      for (int i = 0; i < count; i++) {
        out.add(queue.pollFirst());
      }
      return out;
    }
  }

  /** Puts a failed batch back at the front, as far as room allows (keeping the newest). */
  private void putBack(List<Map<String, Object>> batch) {
    synchronized (lock) {
      int room = maxQueueSize - queue.size();
      int skip = Math.max(0, batch.size() - room);
      dropped += skip;
      for (int i = batch.size() - 1; i >= skip; i--) {
        queue.addFirst(batch.get(i));
      }
    }
  }

  private enum Result {
    OK,
    RETRY,
    DROP
  }

  private void drain() {
    sendLock.lock();
    try {
      while (true) {
        if (backingOff && System.nanoTime() - backoffUntil < 0) {
          return;
        }
        backingOff = false;
        List<Map<String, Object>> batch = take(maxBatchSize);
        if (batch.isEmpty()) {
          return;
        }
        List<Map<String, Object>> prepared = prepare(batch);
        if (prepared.isEmpty()) {
          continue;
        }
        double[] retryAfter = {0};
        Result result = send(prepared, retryAfter);
        if (result == Result.OK) {
          failures = 0;
          continue;
        }
        if (result == Result.DROP) {
          continue;
        }
        failedBatches.incrementAndGet();
        failures++;
        putBack(batch);
        double backoff = Math.min(60.0, Math.pow(2, failures - 1));
        double jittered = backoff / 2 + ThreadLocalRandom.current().nextDouble() * backoff / 2;
        double pause = Math.max(retryAfter[0], jittered);
        backoffUntil = System.nanoTime() + (long) (pause * 1e9);
        backingOff = true;
        debug(String.format(Locale.ROOT, "flush failed; retrying in %.1f s", pause));
        return;
      }
    } finally {
      sendLock.unlock();
    }
  }

  /** Redaction, beforeSend and removal of empty fields, off the request path. */
  @SuppressWarnings("unchecked")
  private List<Map<String, Object>> prepare(List<Map<String, Object>> batch) {
    Redactor r = redactor;
    List<Map<String, Object>> out = new ArrayList<>(batch.size());
    for (Map<String, Object> original : batch) {
      try {
        Map<String, Object> item = new LinkedHashMap<>();
        for (Map.Entry<String, Object> e : original.entrySet()) {
          if (e.getValue() != null) {
            item.put(e.getKey(), e.getValue());
          }
        }
        Object type = item.get("type");
        if ("request".equals(type)) {
          String ct = header(item.get("reqHeaders"), "content-type");
          String resCt = header(item.get("resHeaders"), "content-type");
          if (item.get("reqHeaders") instanceof Map<?, ?> h) {
            item.put("reqHeaders", r.redactHeaders(h));
          }
          if (item.get("resHeaders") instanceof Map<?, ?> h) {
            item.put("resHeaders", r.redactHeaders(h));
          }
          redactBodyField(r, item, "reqBody", ct);
          redactBodyField(r, item, "resBody", resCt);
          if (item.get("query") instanceof String s) {
            item.put("query", r.redactQuery(s));
          }
          if (item.get("ip") instanceof String s) {
            item.put("ip", r.ip(s));
          }
          if (item.get("errorMessage") instanceof String s) {
            item.put("errorMessage", r.applyPatterns(s));
          }
        } else if ("event".equals(type)) {
          if (item.get("message") instanceof String s) {
            item.put("message", r.applyPatterns(s));
          }
          if (item.get("errorMessage") instanceof String s) {
            item.put("errorMessage", r.applyPatterns(s));
          }
        }
        Map<String, Object> result = item;
        if (beforeSend != null) {
          try {
            result = beforeSend.apply(item);
          } catch (RuntimeException e) {
            debug("beforeSend threw: " + e);
          }
        }
        if (result != null && !result.isEmpty()) {
          out.add(result);
        }
      } catch (RuntimeException | StackOverflowError e) {
        debug("could not prepare an item: " + e);
      }
    }
    return out;
  }

  private static void redactBodyField(
      Redactor r, Map<String, Object> item, String key, String contentType) {
    Object body = item.get(key);
    if (body == null) {
      return;
    }
    try {
      String text = body instanceof String s ? s : Json.write(body);
      item.put(key, r.redactBody(text, contentType != null ? contentType : ""));
    } catch (RuntimeException | StackOverflowError e) {
      // A body that can't be redacted safely isn't sent.
      item.remove(key);
    }
  }

  private static String header(Object headers, String name) {
    if (headers instanceof Map<?, ?> m && m.get(name) instanceof String s) {
      return s;
    }
    return null;
  }

  private Result send(List<Map<String, Object>> items, double[] retryAfter) {
    byte[] body;
    try {
      StringBuilder sb = new StringBuilder(items.size() * 512);
      for (int i = 0; i < items.size(); i++) {
        if (i > 0) {
          sb.append('\n');
        }
        Json.write(sb, items.get(i));
      }
      ByteArrayOutputStream bos = new ByteArrayOutputStream(Math.max(256, sb.length() / 4));
      try (GZIPOutputStream gz = new GZIPOutputStream(bos)) {
        gz.write(sb.toString().getBytes(StandardCharsets.UTF_8));
      }
      body = bos.toByteArray();
    } catch (IOException | RuntimeException e) {
      debug("could not serialize batch: " + e);
      return Result.DROP;
    }
    HttpResponse<String> resp;
    try {
      HttpRequest req =
          HttpRequest.newBuilder(ingestUri)
              .timeout(requestTimeout)
              .header("authorization", "Bearer " + ingestKey)
              .header("content-type", "application/x-ndjson")
              .header("content-encoding", "gzip")
              .header("user-agent", USER_AGENT)
              .POST(HttpRequest.BodyPublishers.ofByteArray(body))
              .build();
      resp = http.send(req, HttpResponse.BodyHandlers.ofString());
    } catch (IOException | RuntimeException e) {
      debug("network error: " + e);
      return Result.RETRY;
    } catch (InterruptedException e) {
      Thread.interrupted();
      debug("send interrupted");
      return Result.RETRY;
    }
    int status = resp.statusCode();
    if (status == 200 || status == 207) {
      long accepted = items.size();
      long rej = 0;
      String firstError = "";
      try {
        if (Json.parse(resp.body()) instanceof Map<?, ?> m) {
          if (m.get("accepted") instanceof Number n) {
            accepted = n.longValue();
          }
          if (m.get("rejected") instanceof Number n) {
            rej = n.longValue();
          }
          if (m.get("errors") instanceof List<?> errors
              && !errors.isEmpty()
              && errors.get(0) instanceof Map<?, ?> first
              && first.get("message") instanceof String msg) {
            firstError = msg;
          }
        }
      } catch (RuntimeException e) {
        // An unreadable 200 still means the batch was taken.
      }
      sent.addAndGet(accepted);
      if (rej > 0) {
        rejected.addAndGet(rej);
        debug("server rejected " + rej + " items: " + firstError);
      }
      return Result.OK;
    }
    if (status == 429 || status >= 500) {
      retryAfter[0] =
          resp.headers().firstValue("retry-after").map(TrailscopeClient::seconds).orElse(0.0);
      return Result.RETRY;
    }
    if ((status == 401 || status == 403) && !warnedAuth) {
      warnedAuth = true;
      LOG.log(
          System.Logger.Level.WARNING,
          "trailscope: ingest key rejected (HTTP " + status + "); check TRAILSCOPE_INGEST_KEY");
    }
    rejected.addAndGet(items.size());
    debug("batch dropped: HTTP " + status);
    return Result.DROP;
  }

  private static double seconds(String v) {
    try {
      double d = Double.parseDouble(v.trim());
      return d > 0 && d < 3600 ? d : 0;
    } catch (NumberFormatException e) {
      return 0;
    }
  }

  private void debug(String message) {
    if (debug) {
      LOG.log(System.Logger.Level.INFO, "trailscope: " + message);
    }
  }

  static double nowMillisPrecise() {
    Instant now = Instant.now();
    return now.getEpochSecond() * 1000.0 + now.getNano() / 1e6;
  }

  private static double round2(double v) {
    return Math.round(v * 100) / 100.0;
  }

  private static double clamp(double rate) {
    return Math.min(1.0, Math.max(0.0, rate));
  }

  private static Map<String, Double> clampRates(Map<String, Double> rates) {
    if (rates == null || rates.isEmpty()) {
      return Collections.emptyMap();
    }
    Map<String, Double> out = new LinkedHashMap<>();
    rates.forEach(
        (k, v) -> {
          if (k != null && v != null) {
            out.put(k, clamp(v));
          }
        });
    return Collections.unmodifiableMap(out);
  }

  private static long millis(Duration d, long fallback) {
    return d != null ? d.toMillis() : fallback;
  }

  private static String env(String name) {
    try {
      String v = System.getenv(name);
      return v == null || v.isEmpty() ? null : v;
    } catch (SecurityException e) {
      return null;
    }
  }

  private static String firstNonEmpty(String a, String b, String fallback) {
    if (a != null && !a.isEmpty()) {
      return a;
    }
    if (b != null && !b.isEmpty()) {
      return b;
    }
    return fallback;
  }
}
