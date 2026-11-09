package pro.trailscope;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * An event to send with {@link TrailscopeClient#captureEvent(Event)}: a custom event, a log line
 * or a job run. The current context supplies the request ID, trace ID, user and correlation keys.
 *
 * <pre>{@code
 * client.captureEvent(Event.of("export.created").attribute("rows", 1200));
 * }</pre>
 */
public final class Event {
  final String name;
  EventKind kind = EventKind.CUSTOM;
  Level level;
  String message;
  EventStatus status;
  Double durationMs;
  Integer attempt;
  Map<String, Object> attributes;
  String errorType;
  String errorMessage;
  String errorStack;
  Long timestamp;
  TrailscopeContext context;

  private Event(String name) {
    this.name = name;
  }

  /** A custom event with this name, e.g. {@code email.send}. */
  public static Event of(String name) {
    return new Event(name);
  }

  public Event kind(EventKind kind) {
    this.kind = kind;
    return this;
  }

  public Event level(Level level) {
    this.level = level;
    return this;
  }

  public Event message(String message) {
    this.message = message;
    return this;
  }

  public Event status(EventStatus status) {
    this.status = status;
    return this;
  }

  public Event durationMs(double durationMs) {
    this.durationMs = durationMs;
    return this;
  }

  /** The 1-based attempt number of a job run. */
  public Event attempt(int attempt) {
    this.attempt = attempt;
    return this;
  }

  /** Adds an attribute (a string, number or boolean). */
  public Event attribute(String key, Object value) {
    if (key != null && value != null) {
      if (attributes == null) {
        attributes = new LinkedHashMap<>();
      }
      attributes.put(key, value);
    }
    return this;
  }

  /** Adds attributes. */
  public Event attributes(Map<String, ?> attributes) {
    if (attributes != null) {
      attributes.forEach(this::attribute);
    }
    return this;
  }

  /** Attaches an exception: its class name, message and full stack. */
  public Event error(Throwable error) {
    if (error != null) {
      this.errorType = Errors.type(error);
      this.errorMessage = Errors.message(error);
      this.errorStack = Errors.stack(error);
    }
    return this;
  }

  /** Attaches error details already rendered as text (for logging bridges). */
  public Event error(String type, String message, String stack) {
    this.errorType = type;
    this.errorMessage = message;
    this.errorStack = stack;
    return this;
  }

  /** When the event happened, in epoch milliseconds. Default: when it is captured. */
  public Event timestamp(long epochMillis) {
    this.timestamp = epochMillis;
    return this;
  }

  /** The context to link the event to. Default: the current one. */
  public Event context(TrailscopeContext context) {
    this.context = context;
    return this;
  }
}
