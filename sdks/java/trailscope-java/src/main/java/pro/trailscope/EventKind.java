package pro.trailscope;

/** What an event records. */
public enum EventKind {
  /** A background job run. */
  JOB("job"),
  /** An application log line. */
  LOG("log"),
  /** A custom event. */
  CUSTOM("custom");

  private final String wire;

  EventKind(String wire) {
    this.wire = wire;
  }

  /** The value sent on the wire. */
  public String wire() {
    return wire;
  }
}
