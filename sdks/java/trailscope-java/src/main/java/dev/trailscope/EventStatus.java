package dev.trailscope;

/** Outcome of an event. When it isn't set, the server derives it from the level. */
public enum EventStatus {
  OK("ok"),
  WARN("warn"),
  ERROR("error");

  private final String wire;

  EventStatus(String wire) {
    this.wire = wire;
  }

  /** The value sent on the wire. */
  public String wire() {
    return wire;
  }
}
