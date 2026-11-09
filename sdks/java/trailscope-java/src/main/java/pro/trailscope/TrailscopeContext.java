package pro.trailscope;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * What Trailscope knows about the request (or job) running on the current thread: its IDs, the
 * user, correlation keys, attributes, the matched route and the error that failed it.
 *
 * <p>Integrations create one per request and make it current with {@link
 * Trailscope#useContext(TrailscopeContext)}; application code changes it through {@link
 * Trailscope#setUser(Object)}, {@link Trailscope#setCorrelation(String, Object)} and friends. It
 * is safe to share between threads.
 */
public final class TrailscopeContext {
  private final String requestId;
  private final String traceId;
  private final String spanId;
  private final String parentSpanId;
  private volatile String userId;
  private volatile String route;
  private volatile Throwable error;
  private Map<String, String> correlation;
  private Map<String, Object> attributes;

  /**
   * Creates a context. Use {@link TrailscopeClient#contextFromHeaders} for an incoming request.
   *
   * @param requestId the request ID
   * @param traceId the W3C trace ID (32 hex characters)
   * @param spanId this request's span ID (16 hex characters)
   * @param parentSpanId the caller's span ID, or null
   */
  public TrailscopeContext(String requestId, String traceId, String spanId, String parentSpanId) {
    this.requestId = requestId;
    this.traceId = traceId;
    this.spanId = spanId;
    this.parentSpanId = parentSpanId;
  }

  /** A new context with fresh IDs. */
  public static TrailscopeContext create() {
    return new TrailscopeContext(Ids.newRequestId(), Ids.newTraceId(), Ids.newSpanId(), null);
  }

  public String requestId() {
    return requestId;
  }

  public String traceId() {
    return traceId;
  }

  public String spanId() {
    return spanId;
  }

  public String parentSpanId() {
    return parentSpanId;
  }

  public String userId() {
    return userId;
  }

  /** The matched route pattern, e.g. {@code /projects/:projectId/tasks}; null until known. */
  public String route() {
    return route;
  }

  /** The exception that failed the request; null if none was recorded. */
  public Throwable error() {
    return error;
  }

  /** Sets the user ID; null or empty values are ignored. */
  public void setUser(Object userId) {
    if (userId != null) {
      String s = String.valueOf(userId);
      if (!s.isEmpty()) {
        this.userId = s;
      }
    }
  }

  /** Sets the route pattern, in {@code :name} form. */
  public void setRoute(String route) {
    this.route = route;
  }

  /** Records the exception that failed the request. The first one wins. */
  public void recordError(Throwable error) {
    if (error != null && this.error == null) {
      this.error = error;
    }
  }

  /** Adds a correlation (journey) key, e.g. {@code account_id}. */
  public synchronized void setCorrelation(String key, Object value) {
    if (key == null || key.isEmpty() || value == null) {
      return;
    }
    if (correlation == null) {
      correlation = new LinkedHashMap<>(4);
    }
    correlation.put(key, String.valueOf(value));
  }

  /** Adds a free-form attribute (a string, number or boolean). */
  public synchronized void setAttribute(String key, Object value) {
    if (key == null || key.isEmpty() || value == null) {
      return;
    }
    if (attributes == null) {
      attributes = new LinkedHashMap<>(4);
    }
    attributes.put(key, value);
  }

  /** A copy of the correlation keys. */
  public synchronized Map<String, String> correlation() {
    return correlation == null ? Collections.emptyMap() : new LinkedHashMap<>(correlation);
  }

  /** A copy of the attributes. */
  public synchronized Map<String, Object> attributes() {
    return attributes == null ? Collections.emptyMap() : new LinkedHashMap<>(attributes);
  }

  synchronized Map<String, String> correlationOrNull() {
    return correlation == null || correlation.isEmpty() ? null : new LinkedHashMap<>(correlation);
  }

  synchronized Map<String, Object> attributesOrNull() {
    return attributes == null || attributes.isEmpty() ? null : new LinkedHashMap<>(attributes);
  }

  @Override
  public String toString() {
    return "TrailscopeContext{requestId=" + requestId + ", traceId=" + traceId + "}";
  }
}
