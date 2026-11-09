package dev.trailscope;

import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.util.Collection;
import java.util.Locale;
import java.util.Map;

/** Body capture policy, serialization and truncation. */
final class Bodies {
  private static final String[] BINARY_TYPES = {
    "multipart/",
    "application/octet-stream",
    "image/",
    "audio/",
    "video/",
    "font/",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-protobuf",
    "application/grpc",
  };

  private Bodies() {}

  /** A serialized body and whether it was cut. */
  record Captured(String text, boolean truncated) {}

  /** Whether a body with this content type should never be captured. */
  static boolean isBinaryContentType(String contentType) {
    if (contentType == null || contentType.isEmpty()) {
      return false;
    }
    String ct = contentType.toLowerCase(Locale.ROOT);
    for (String t : BINARY_TYPES) {
      if (ct.startsWith(t)) {
        return true;
      }
    }
    return false;
  }

  static boolean shouldCapture(BodyCapture policy, int status, double durationMs, double slowMs) {
    if (policy == BodyCapture.NEVER) {
      return false;
    }
    if (policy == BodyCapture.ALWAYS) {
      return true;
    }
    return status >= 400 || durationMs >= slowMs;
  }

  /**
   * Serializes a body-like value and truncates it to {@code limit} UTF-8 bytes. Returns null for
   * binary or empty bodies. Raw bodies (bytes, text) with a binary or multipart content type are
   * skipped; already parsed form fields (a map) are kept.
   */
  static Captured serialize(Object value, String contentType, int limit) {
    if (value == null) {
      return null;
    }
    String text;
    if (value instanceof byte[] bytes) {
      if (isBinaryContentType(contentType) || bytes.length == 0) {
        return null;
      }
      int scan = Math.min(bytes.length, 512);
      for (int i = 0; i < scan; i++) {
        if (bytes[i] == 0) {
          return null;
        }
      }
      int n = Math.min(bytes.length, limit + 4);
      text = new String(bytes, 0, n, charsetOf(contentType));
      Captured c = truncate(text, limit);
      return bytes.length > limit ? new Captured(c.text(), true) : c;
    } else if (value instanceof CharSequence cs) {
      if (isBinaryContentType(contentType)) {
        return null;
      }
      text = cs.toString();
    } else if (value instanceof Map<?, ?> || value instanceof Collection<?>) {
      text = Json.write(value);
    } else {
      text = String.valueOf(value);
    }
    if (text.isEmpty()) {
      return null;
    }
    return truncate(text, limit);
  }

  /** Cuts {@code text} to at most {@code limit} UTF-8 bytes, at a character boundary. */
  static Captured truncate(String text, int limit) {
    int n = text.length();
    if ((long) n * 3 <= limit) {
      return new Captured(text, false);
    }
    int bytes = 0;
    int i = 0;
    while (i < n) {
      char c = text.charAt(i);
      int width;
      int step = 1;
      if (c < 0x80) {
        width = 1;
      } else if (c < 0x800) {
        width = 2;
      } else if (Character.isHighSurrogate(c)
          && i + 1 < n
          && Character.isLowSurrogate(text.charAt(i + 1))) {
        width = 4;
        step = 2;
      } else {
        width = 3;
      }
      if (bytes + width > limit) {
        return new Captured(text.substring(0, i), true);
      }
      bytes += width;
      i += step;
    }
    return new Captured(text, false);
  }

  /** The {@code charset} parameter of a content type, or UTF-8. */
  static Charset charsetOf(String contentType) {
    if (contentType != null) {
      int i = contentType.toLowerCase(Locale.ROOT).indexOf("charset=");
      if (i >= 0) {
        String name = contentType.substring(i + 8).trim();
        int end = name.indexOf(';');
        if (end >= 0) {
          name = name.substring(0, end).trim();
        }
        if (name.length() > 1 && name.startsWith("\"") && name.endsWith("\"")) {
          name = name.substring(1, name.length() - 1);
        }
        try {
          return Charset.forName(name);
        } catch (IllegalArgumentException e) {
          return StandardCharsets.UTF_8;
        }
      }
    }
    return StandardCharsets.UTF_8;
  }
}
