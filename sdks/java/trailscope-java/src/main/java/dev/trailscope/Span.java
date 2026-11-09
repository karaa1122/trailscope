package dev.trailscope;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A span started with {@link TrailscopeClient#startSpan}; close it (try-with-resources) to record
 * it.
 *
 * <pre>{@code
 * try (Span span = client.startSpan(SpanKind.DB, "SELECT tasks")) {
 *   rows = query();
 * } catch (SQLException e) {
 *   ... // call span.fail(e) inside the block to mark it as failed
 * }
 * }</pre>
 */
public final class Span implements AutoCloseable {
  private final TrailscopeClient client;
  private final TrailscopeContext context;
  private final SpanKind kind;
  private final String name;
  private final double start;
  private final long t0;
  private Map<String, Object> attributes;
  private boolean failed;
  private boolean ended;

  Span(TrailscopeClient client, TrailscopeContext context, SpanKind kind, String name) {
    this.client = client;
    this.context = context;
    this.kind = kind;
    this.name = name;
    this.start = TrailscopeClient.nowMillisPrecise();
    this.t0 = System.nanoTime();
  }

  /** Adds an attribute. */
  public Span attribute(String key, Object value) {
    if (key != null && value != null) {
      if (attributes == null) {
        attributes = new LinkedHashMap<>();
      }
      attributes.put(key, value);
    }
    return this;
  }

  /** Marks the span as failed (spans carry a status, not the error itself). */
  public Span fail(Throwable error) {
    failed = true;
    return this;
  }

  /** Ends and records the span. Later calls do nothing. */
  public void end() {
    if (ended) {
      return;
    }
    ended = true;
    double durationMs = (System.nanoTime() - t0) / 1e6;
    client.recordSpan(context, kind, name, start, durationMs, failed, attributes);
  }

  @Override
  public void close() {
    end();
  }
}
