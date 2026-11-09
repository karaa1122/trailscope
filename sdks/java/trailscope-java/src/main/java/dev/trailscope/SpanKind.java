package dev.trailscope;

/** What a span times. */
public enum SpanKind {
  HTTP("http"),
  DB("db"),
  CACHE("cache"),
  QUEUE("queue"),
  FN("fn"),
  EXTERNAL("external");

  private final String wire;

  SpanKind(String wire) {
    this.wire = wire;
  }

  /** The value sent on the wire. */
  public String wire() {
    return wire;
  }
}
