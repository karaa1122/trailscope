package dev.trailscope;

import java.util.Locale;

/** When request and response bodies are kept. */
public enum BodyCapture {
  /** Only for requests with status 400 or more, or slow ones (the default). */
  ERRORS,
  /** For every recorded request. */
  ALWAYS,
  /** Never. */
  NEVER;

  /** Parses {@code errors}, {@code always} or {@code never} (any case); null for anything else. */
  public static BodyCapture parse(String value) {
    if (value == null) {
      return null;
    }
    switch (value.trim().toLowerCase(Locale.ROOT)) {
      case "errors":
        return ERRORS;
      case "always":
        return ALWAYS;
      case "never":
        return NEVER;
      default:
        return null;
    }
  }
}
