package dev.trailscope;

import java.util.Locale;
import java.util.concurrent.ThreadLocalRandom;

/** Request, trace and span IDs, and the W3C {@code traceparent} header. */
public final class Ids {
  private static final char[] HEX = "0123456789abcdef".toCharArray();

  private Ids() {}

  /** 32 random hex characters, like the server's generated request IDs. */
  public static String newRequestId() {
    return hex128();
  }

  /** A W3C trace ID: 32 hex characters, never all zeros. */
  public static String newTraceId() {
    return hex128();
  }

  /** A W3C span ID: 16 hex characters, never all zeros. */
  public static String newSpanId() {
    long v;
    do {
      v = ThreadLocalRandom.current().nextLong();
    } while (v == 0);
    char[] out = new char[16];
    put(out, 0, v);
    return new String(out);
  }

  private static String hex128() {
    ThreadLocalRandom r = ThreadLocalRandom.current();
    long hi;
    long lo;
    do {
      hi = r.nextLong();
      lo = r.nextLong();
    } while (hi == 0 && lo == 0);
    char[] out = new char[32];
    put(out, 0, hi);
    put(out, 16, lo);
    return new String(out);
  }

  private static void put(char[] out, int at, long v) {
    for (int i = 15; i >= 0; i--) {
      out[at + i] = HEX[(int) (v & 0xf)];
      v >>>= 4;
    }
  }

  /**
   * A parsed {@code traceparent} header.
   *
   * @param traceId the caller's trace ID
   * @param parentSpanId the caller's span ID, the parent of this request's span
   * @param sampled the sampled flag
   */
  public record TraceParent(String traceId, String parentSpanId, boolean sampled) {}

  /** Parses a W3C {@code traceparent} header; null when it is missing or invalid. */
  public static TraceParent parseTraceparent(String header) {
    if (header == null) {
      return null;
    }
    String h = header.trim().toLowerCase(Locale.ROOT);
    // version "-" trace-id "-" parent-id "-" flags
    if (h.length() != 55 || h.charAt(2) != '-' || h.charAt(35) != '-' || h.charAt(52) != '-') {
      return null;
    }
    if (!isHex(h, 0, 2) || !isHex(h, 3, 35) || !isHex(h, 36, 52) || !isHex(h, 53, 55)) {
      return null;
    }
    if (h.startsWith("ff") || allZeros(h, 3, 35) || allZeros(h, 36, 52)) {
      return null;
    }
    int flags = Integer.parseInt(h.substring(53, 55), 16);
    return new TraceParent(h.substring(3, 35), h.substring(36, 52), (flags & 1) == 1);
  }

  /** Formats a {@code traceparent} header for an outgoing call. */
  public static String formatTraceparent(String traceId, String spanId, boolean sampled) {
    return "00-" + traceId + "-" + spanId + (sampled ? "-01" : "-00");
  }

  /**
   * A client-provided request ID, only if it matches {@code [A-Za-z0-9._:-]{1,128}}; null
   * otherwise.
   */
  public static String acceptRequestId(String value) {
    if (value == null) {
      return null;
    }
    String v = value.trim();
    int n = v.length();
    if (n == 0 || n > 128) {
      return null;
    }
    for (int i = 0; i < n; i++) {
      char c = v.charAt(i);
      boolean ok =
          (c >= 'a' && c <= 'z')
              || (c >= 'A' && c <= 'Z')
              || (c >= '0' && c <= '9')
              || c == '.'
              || c == '_'
              || c == ':'
              || c == '-';
      if (!ok) {
        return null;
      }
    }
    return v;
  }

  private static boolean isHex(String s, int from, int to) {
    for (int i = from; i < to; i++) {
      char c = s.charAt(i);
      if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) {
        return false;
      }
    }
    return true;
  }

  private static boolean allZeros(String s, int from, int to) {
    for (int i = from; i < to; i++) {
      if (s.charAt(i) != '0') {
        return false;
      }
    }
    return true;
  }
}
