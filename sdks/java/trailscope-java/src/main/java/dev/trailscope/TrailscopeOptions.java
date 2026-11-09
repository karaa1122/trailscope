package dev.trailscope;

import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.UnaryOperator;
import java.util.regex.Pattern;

/**
 * Options for a {@link TrailscopeClient}. Every setter returns {@code this}:
 *
 * <pre>{@code
 * TrailscopeClient client = Trailscope.init(new TrailscopeOptions()
 *     .service("orders-api")
 *     .redactKeys("dob"));
 * }</pre>
 *
 * <p>Options left unset fall back to the {@code TRAILSCOPE_*} environment variables ({@code
 * TRAILSCOPE_ENDPOINT}, {@code TRAILSCOPE_INGEST_KEY}, {@code TRAILSCOPE_SERVICE}, {@code
 * TRAILSCOPE_ENVIRONMENT}, {@code TRAILSCOPE_RELEASE}, {@code TRAILSCOPE_ENABLED=0}, {@code
 * TRAILSCOPE_DEBUG=1}), then to the defaults. Sampling and capture options set here win over the
 * project settings the client loads from the server.
 */
public final class TrailscopeOptions {
  String endpoint;
  String ingestKey;
  String service;
  String environment;
  String release;
  Boolean enabled;
  Boolean debug;
  int maxQueueSize = 10_000;
  Duration flushInterval = Duration.ofSeconds(1);
  int flushAt = 500;
  int maxBatchSize = 1000;
  Duration requestTimeout = Duration.ofSeconds(10);
  Duration shutdownTimeout = Duration.ofSeconds(5);
  Double sampleRate;
  Map<String, Double> routeSampleRates;
  Double slowMs;
  BodyCapture captureBodies;
  Integer maxBodyBytes;
  List<String> ignoreRoutes;
  final List<Pattern> ignoreRoutePatterns = new ArrayList<>();
  String requestIdHeader = "x-request-id";
  final List<String> redactHeaders = new ArrayList<>();
  final List<String> redactKeys = new ArrayList<>();
  final List<Pattern> redactPatterns = new ArrayList<>();
  Boolean maskIp;
  boolean disableDefaultRedaction;
  boolean remoteConfig = true;
  boolean capturePrincipal = true;
  boolean shutdownHook = true;
  UnaryOperator<Map<String, Object>> beforeSend;

  /** Trailscope's base URL. Env {@code TRAILSCOPE_ENDPOINT}; default {@code localhost:8080}. */
  public TrailscopeOptions endpoint(String endpoint) {
    this.endpoint = endpoint;
    return this;
  }

  /** The project's ingest key. Env {@code TRAILSCOPE_INGEST_KEY}; without one nothing is sent. */
  public TrailscopeOptions ingestKey(String ingestKey) {
    this.ingestKey = ingestKey;
    return this;
  }

  /** The service name. Env {@code TRAILSCOPE_SERVICE}; default {@code unknown-service}. */
  public TrailscopeOptions service(String service) {
    this.service = service;
    return this;
  }

  /** Env {@code TRAILSCOPE_ENVIRONMENT}; default {@code production}. */
  public TrailscopeOptions environment(String environment) {
    this.environment = environment;
    return this;
  }

  /** The running version. Env {@code TRAILSCOPE_RELEASE}. */
  public TrailscopeOptions release(String release) {
    this.release = release;
    return this;
  }

  /** {@code false} turns every call into a no-op. Env {@code TRAILSCOPE_ENABLED=0} disables. */
  public TrailscopeOptions enabled(boolean enabled) {
    this.enabled = enabled;
    return this;
  }

  /** Logs SDK diagnostics on the {@code dev.trailscope} logger. Env {@code TRAILSCOPE_DEBUG=1}. */
  public TrailscopeOptions debug(boolean debug) {
    this.debug = debug;
    return this;
  }

  /** Items kept in memory before the oldest are dropped. Default 10,000. */
  public TrailscopeOptions maxQueueSize(int maxQueueSize) {
    this.maxQueueSize = maxQueueSize;
    return this;
  }

  /** How often the background thread sends. Default 1 second. */
  public TrailscopeOptions flushInterval(Duration flushInterval) {
    this.flushInterval = flushInterval;
    return this;
  }

  /** Queue size that triggers an early flush. Default 500. */
  public TrailscopeOptions flushAt(int flushAt) {
    this.flushAt = flushAt;
    return this;
  }

  /** Items per request to {@code /v1/ingest}. Default 1,000. */
  public TrailscopeOptions maxBatchSize(int maxBatchSize) {
    this.maxBatchSize = maxBatchSize;
    return this;
  }

  /** Timeout of each HTTP call to Trailscope. Default 10 seconds. */
  public TrailscopeOptions requestTimeout(Duration requestTimeout) {
    this.requestTimeout = requestTimeout;
    return this;
  }

  /** How long {@link TrailscopeClient#shutdown()} waits for the final flush. Default 5 seconds. */
  public TrailscopeOptions shutdownTimeout(Duration shutdownTimeout) {
    this.shutdownTimeout = shutdownTimeout;
    return this;
  }

  /** Share of successful, fast requests to keep, in [0, 1]. Default 1. */
  public TrailscopeOptions sampleRate(double sampleRate) {
    this.sampleRate = sampleRate;
    return this;
  }

  /** Per-route sample rates, keyed by route pattern ({@code /search}). */
  public TrailscopeOptions routeSampleRates(Map<String, Double> routeSampleRates) {
    this.routeSampleRates = routeSampleRates == null ? null : new LinkedHashMap<>(routeSampleRates);
    return this;
  }

  /** Requests at least this slow are always kept, with their bodies. Default 1,000 ms. */
  public TrailscopeOptions slowMs(double slowMs) {
    this.slowMs = slowMs;
    return this;
  }

  /** When bodies are kept. Default {@link BodyCapture#ERRORS}. */
  public TrailscopeOptions captureBodies(BodyCapture captureBodies) {
    this.captureBodies = captureBodies;
    return this;
  }

  /** Bytes kept per body. Default 8,192. */
  public TrailscopeOptions maxBodyBytes(int maxBodyBytes) {
    this.maxBodyBytes = maxBodyBytes;
    return this;
  }

  /**
   * Routes and paths that aren't recorded, matched exactly. Replaces the default list ({@code
   * /health}, {@code /healthz}, {@code /ready}, {@code /readyz}, {@code /livez}).
   */
  public TrailscopeOptions ignoreRoutes(Collection<String> ignoreRoutes) {
    this.ignoreRoutes = ignoreRoutes == null ? null : new ArrayList<>(ignoreRoutes);
    return this;
  }

  /** Same as {@link #ignoreRoutes(Collection)}. */
  public TrailscopeOptions ignoreRoutes(String... ignoreRoutes) {
    return ignoreRoutes(Arrays.asList(ignoreRoutes));
  }

  /**
   * Adds a regular expression of routes and paths that aren't recorded (searched, not fully
   * matched). Applies on top of {@link #ignoreRoutes(Collection)} and the server's list.
   */
  public TrailscopeOptions ignoreRoutePattern(Pattern pattern) {
    if (pattern != null) {
      ignoreRoutePatterns.add(pattern);
    }
    return this;
  }

  /** The header carrying the request ID in and out. Default {@code x-request-id}. */
  public TrailscopeOptions requestIdHeader(String requestIdHeader) {
    this.requestIdHeader = requestIdHeader;
    return this;
  }

  /** Header names redacted on top of the built-in ones. */
  public TrailscopeOptions redactHeaders(String... names) {
    return redactHeaders(Arrays.asList(names));
  }

  /** Header names redacted on top of the built-in ones. */
  public TrailscopeOptions redactHeaders(Collection<String> names) {
    if (names != null) {
      redactHeaders.addAll(names);
    }
    return this;
  }

  /** Body keys redacted on top of the built-in ones; dotted paths with {@code *} work too. */
  public TrailscopeOptions redactKeys(String... keys) {
    return redactKeys(Arrays.asList(keys));
  }

  /** Body keys redacted on top of the built-in ones; dotted paths with {@code *} work too. */
  public TrailscopeOptions redactKeys(Collection<String> keys) {
    if (keys != null) {
      redactKeys.addAll(keys);
    }
    return this;
  }

  /** Regular expressions replaced in values, headers, the query, error and log messages. */
  public TrailscopeOptions redactPatterns(String... patterns) {
    return redactPatterns(Arrays.asList(patterns));
  }

  /** Regular expressions replaced in values, headers, the query, error and log messages. */
  public TrailscopeOptions redactPatterns(Collection<String> patterns) {
    if (patterns != null) {
      for (String p : patterns) {
        redactPatterns.add(Pattern.compile(p));
      }
    }
    return this;
  }

  /** A compiled redaction pattern. */
  public TrailscopeOptions redactPattern(Pattern pattern) {
    if (pattern != null) {
      redactPatterns.add(pattern);
    }
    return this;
  }

  /** Masks the last IPv4 octet (or all but 48 bits of IPv6). Default on. */
  public TrailscopeOptions maskIp(boolean maskIp) {
    this.maskIp = maskIp;
    return this;
  }

  /** Drops the built-in redaction rules. Not recommended. */
  public TrailscopeOptions disableDefaultRedaction(boolean disable) {
    this.disableDefaultRedaction = disable;
    return this;
  }

  /** Loads sampling and capture settings from {@code /v1/config}. Default on. */
  public TrailscopeOptions remoteConfig(boolean remoteConfig) {
    this.remoteConfig = remoteConfig;
    return this;
  }

  /**
   * Records the authenticated principal's name ({@code request.getUserPrincipal()}) as the user
   * when the application didn't call {@code setUser}. Default on.
   */
  public TrailscopeOptions capturePrincipal(boolean capturePrincipal) {
    this.capturePrincipal = capturePrincipal;
    return this;
  }

  /** Whether {@link Trailscope#init} registers a JVM shutdown hook that flushes. Default on. */
  public TrailscopeOptions shutdownHook(boolean shutdownHook) {
    this.shutdownHook = shutdownHook;
    return this;
  }

  /**
   * Edits each item (a wire-format map) before it is sent, or returns null to drop it. Runs in
   * the background thread, after redaction.
   */
  public TrailscopeOptions beforeSend(UnaryOperator<Map<String, Object>> beforeSend) {
    this.beforeSend = beforeSend;
    return this;
  }
}
