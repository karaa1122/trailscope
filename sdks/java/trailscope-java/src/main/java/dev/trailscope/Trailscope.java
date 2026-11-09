package dev.trailscope;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.concurrent.Callable;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;

/**
 * Entry point: the process-wide client and the current thread's context.
 *
 * <pre>{@code
 * TrailscopeClient client = Trailscope.init(new TrailscopeOptions().service("orders-api"));
 *
 * // inside a request
 * Trailscope.setUser(userId);
 * Trailscope.setCorrelation("order_id", orderId);
 * }</pre>
 *
 * <p>The context lives in a {@link ThreadLocal}. Work handed to another thread takes it along
 * with {@link #wrap(Runnable)} or a wrapped executor ({@link #wrap(ExecutorService)}). None of
 * these methods throw because of Trailscope; outside a request they do nothing.
 */
public final class Trailscope {
  /** The SDK version, from the jar manifest. */
  public static final String VERSION = version();

  private static final ThreadLocal<TrailscopeContext> CURRENT = new ThreadLocal<>();
  private static final Map<Throwable, Boolean> RECORDED =
      Collections.synchronizedMap(new WeakHashMap<>());
  private static final Object INIT_LOCK = new Object();
  private static volatile TrailscopeClient global;

  private Trailscope() {}

  private static String version() {
    String v = Trailscope.class.getPackage().getImplementationVersion();
    return v != null ? v : "dev";
  }

  // ---- the process-wide client -------------------------------------------------------------

  /** Creates the process-wide client from the {@code TRAILSCOPE_*} environment variables. */
  public static TrailscopeClient init() {
    return init(new TrailscopeOptions());
  }

  /**
   * Creates the process-wide client used by the integrations. A previous one is shut down. Unless
   * {@link TrailscopeOptions#shutdownHook(boolean)} is off, a JVM shutdown hook flushes what is
   * left when the process exits.
   */
  public static TrailscopeClient init(TrailscopeOptions options) {
    TrailscopeClient client = new TrailscopeClient(options);
    TrailscopeClient previous;
    synchronized (INIT_LOCK) {
      previous = global;
      global = client;
    }
    if (previous != null) {
      previous.shutdown();
    }
    if (options.shutdownHook) {
      try {
        Runtime.getRuntime().addShutdownHook(new Thread(client::shutdown, "trailscope-shutdown"));
      } catch (IllegalStateException | SecurityException e) {
        // Already shutting down, or not allowed: nothing to register.
      }
    }
    return client;
  }

  /** The process-wide client, or null before {@link #init()}. */
  public static TrailscopeClient client() {
    return global;
  }

  /**
   * Makes an existing client the process-wide one (used by framework integrations that manage the
   * client's lifecycle themselves). Doesn't shut down the previous one.
   */
  public static void setClient(TrailscopeClient client) {
    synchronized (INIT_LOCK) {
      global = client;
    }
  }

  // ---- context -----------------------------------------------------------------------------

  /** The current thread's context, or null outside a request or job. */
  public static TrailscopeContext currentContext() {
    return CURRENT.get();
  }

  /**
   * Makes {@code ctx} current until the returned scope is closed, which restores the previous one:
   *
   * <pre>{@code
   * try (ContextScope scope = Trailscope.useContext(ctx)) { ... }
   * }</pre>
   */
  public static ContextScope useContext(TrailscopeContext ctx) {
    TrailscopeContext previous = CURRENT.get();
    CURRENT.set(ctx);
    return new ContextScope(previous);
  }

  static void restore(TrailscopeContext previous) {
    if (previous == null) {
      CURRENT.remove();
    } else {
      CURRENT.set(previous);
    }
  }

  /** Attaches a user ID to the current request and what it triggers. */
  public static void setUser(Object userId) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx != null) {
      ctx.setUser(userId);
    }
  }

  /** Adds a correlation (journey) key to the current request and everything it triggers. */
  public static void setCorrelation(String key, Object value) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx != null) {
      ctx.setCorrelation(key, value);
    }
  }

  /** Adds a free-form attribute to the current request. */
  public static void setAttribute(String key, Object value) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx != null) {
      ctx.setAttribute(key, value);
    }
  }

  /** Sets the current request's route pattern, when the integration can't read it. */
  public static void setRoute(String route) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx != null) {
      ctx.setRoute(route);
    }
  }

  /**
   * Attaches the exception that failed the current request. Its type, message and stack are
   * recorded with the request when the response is a 5xx.
   */
  public static void recordError(Throwable error) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx != null) {
      ctx.recordError(error);
    }
  }

  // ---- carrying the context to other threads and processes -----------------------------------

  /** Wraps a task so it runs inside the current context, on whichever thread runs it. */
  public static Runnable wrap(Runnable task) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx == null || task == null) {
      return task;
    }
    return () -> {
      try (ContextScope scope = useContext(ctx)) {
        task.run();
      }
    };
  }

  /** Wraps a task so it runs inside the current context, on whichever thread runs it. */
  public static <T> Callable<T> wrap(Callable<T> task) {
    TrailscopeContext ctx = CURRENT.get();
    if (ctx == null || task == null) {
      return task;
    }
    return () -> {
      try (ContextScope scope = useContext(ctx)) {
        return task.call();
      }
    };
  }

  /** An executor whose tasks run inside the context of the thread that submitted them. */
  public static Executor wrap(Executor executor) {
    if (executor instanceof ExecutorService service) {
      return wrap(service);
    }
    return command -> executor.execute(wrap(command));
  }

  /** An executor service whose tasks run inside the context of the thread that submitted them. */
  public static ExecutorService wrap(ExecutorService executor) {
    if (executor instanceof ContextExecutorService) {
      return executor;
    }
    return new ContextExecutorService(executor);
  }

  /**
   * The part of the current context that travels with a background job: {@code requestId},
   * {@code traceId}, {@code spanId}, {@code userId} and {@code correlation}. Store it in the job's
   * payload (it serializes to JSON as is) and restore it in the worker with {@link
   * #deserializeContext(Object)}. Null outside a request.
   */
  public static Map<String, Object> serializeContext() {
    return serializeContext(CURRENT.get());
  }

  /** Same as {@link #serializeContext()} for a given context. */
  public static Map<String, Object> serializeContext(TrailscopeContext ctx) {
    if (ctx == null) {
      return null;
    }
    Map<String, Object> out = new LinkedHashMap<>();
    out.put("requestId", ctx.requestId());
    out.put("traceId", ctx.traceId());
    out.put("spanId", ctx.spanId());
    if (ctx.userId() != null) {
      out.put("userId", ctx.userId());
    }
    Map<String, String> correlation = ctx.correlationOrNull();
    if (correlation != null) {
      out.put("correlation", correlation);
    }
    return out;
  }

  /** {@link #serializeContext()} as a JSON string, for message headers. Null outside a request. */
  public static String serializeContextJson() {
    Map<String, Object> m = serializeContext();
    return m == null ? null : Json.write(m);
  }

  /**
   * Restores a context serialized by {@link #serializeContext()}: a map, or its JSON text. Pass
   * the result to {@link JobOptions#context(TrailscopeContext)}. Returns null when the value
   * isn't a serialized context.
   */
  public static TrailscopeContext deserializeContext(Object value) {
    Object v = value;
    if (v instanceof CharSequence text) {
      try {
        v = Json.parse(text.toString());
      } catch (IllegalArgumentException e) {
        return null;
      }
    }
    if (!(v instanceof Map<?, ?> map)) {
      return null;
    }
    String requestId = str(map.get("requestId"));
    String traceId = str(map.get("traceId"));
    String spanId = str(map.get("spanId"));
    TrailscopeContext ctx =
        new TrailscopeContext(
            requestId != null ? requestId : Ids.newRequestId(),
            traceId != null ? traceId : Ids.newTraceId(),
            spanId != null ? spanId : Ids.newSpanId(),
            null);
    ctx.setUser(str(map.get("userId")));
    if (map.get("correlation") instanceof Map<?, ?> correlation) {
      for (Map.Entry<?, ?> e : correlation.entrySet()) {
        Object val = e.getValue();
        if (e.getKey() instanceof String k && (val instanceof String || val instanceof Number)) {
          ctx.setCorrelation(k, val);
        }
      }
    }
    return ctx;
  }

  private static String str(Object v) {
    return v instanceof String s && !s.isEmpty() && s.length() <= 128 ? s : null;
  }

  // ---- for integrations ----------------------------------------------------------------------

  /**
   * Notes that an exception was already recorded with a request, so logging bridges don't report
   * it a second time when the container logs it.
   */
  public static void markRecorded(Throwable error) {
    if (error != null) {
      RECORDED.put(error, Boolean.TRUE);
    }
  }

  /** Whether {@link #markRecorded(Throwable)} was called for this exception. */
  public static boolean isRecorded(Throwable error) {
    return error != null && !RECORDED.isEmpty() && RECORDED.containsKey(error);
  }
}
