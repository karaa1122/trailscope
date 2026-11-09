package pro.trailscope;

/** Severity of an event or log line. */
public enum Level {
  DEBUG("debug"),
  INFO("info"),
  WARN("warn"),
  ERROR("error");

  private final String wire;

  Level(String wire) {
    this.wire = wire;
  }

  /** The value sent on the wire ({@code debug}, {@code info}, {@code warn}, {@code error}). */
  public String wire() {
    return wire;
  }
}
