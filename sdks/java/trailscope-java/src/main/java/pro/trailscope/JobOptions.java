package pro.trailscope;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Options of a job run recorded by {@link TrailscopeClient#trackJob}.
 *
 * <pre>{@code
 * client.trackJob("report.build", JobOptions.attempt(2).willRetry(true), () -> build());
 * }</pre>
 */
public final class JobOptions {
  Integer attempt;
  TrailscopeContext context;
  Map<String, Object> attributes;
  boolean willRetry;

  /** Options with nothing set. */
  public static JobOptions create() {
    return new JobOptions();
  }

  /** Options with the 1-based attempt number set. */
  public static JobOptions attempt(int attempt) {
    return new JobOptions().setAttempt(attempt);
  }

  /** The 1-based attempt number. */
  public JobOptions setAttempt(int attempt) {
    this.attempt = attempt;
    return this;
  }

  /**
   * The context the job belongs to, e.g. one restored with {@link
   * Trailscope#deserializeContext(Object)}. Default: the current one.
   */
  public JobOptions context(TrailscopeContext context) {
    this.context = context;
    return this;
  }

  /** Adds an attribute to the job event. */
  public JobOptions attribute(String key, Object value) {
    if (key != null && value != null) {
      if (attributes == null) {
        attributes = new LinkedHashMap<>();
      }
      attributes.put(key, value);
    }
    return this;
  }

  /**
   * Whether a failure of this run will be retried. A failed run that will be retried is recorded
   * with level {@code warn} instead of {@code error} (its status is still {@code error}).
   */
  public JobOptions willRetry(boolean willRetry) {
    this.willRetry = willRetry;
    return this;
  }
}
