package dev.trailscope.servlet;

import java.util.Arrays;

/** Keeps the first {@code cap} bytes or characters written to a body and counts the rest. */
final class BoundedBuffer {
  private final int cap;
  private byte[] bytes;
  private int byteCount;
  private StringBuilder chars;
  private long total;

  BoundedBuffer(int cap) {
    this.cap = Math.max(0, cap);
  }

  void write(int b) {
    total++;
    if (byteCount < cap) {
      ensure(byteCount + 1);
      bytes[byteCount++] = (byte) b;
    }
  }

  void write(byte[] b, int off, int len) {
    if (len <= 0) {
      return;
    }
    total += len;
    int n = Math.min(len, cap - byteCount);
    if (n > 0) {
      ensure(byteCount + n);
      System.arraycopy(b, off, bytes, byteCount, n);
      byteCount += n;
    }
  }

  void write(char c) {
    total += c < 0x80 ? 1 : c < 0x800 ? 2 : 3;
    if (chars == null) {
      chars = new StringBuilder(Math.min(cap, 256));
    }
    if (chars.length() < cap) {
      chars.append(c);
    }
  }

  void write(char[] c, int off, int len) {
    if (len <= 0) {
      return;
    }
    total += utf8Length(c, off, len);
    if (chars == null) {
      chars = new StringBuilder(Math.min(cap, Math.max(len, 256)));
    }
    int n = Math.min(len, cap - chars.length());
    if (n > 0) {
      chars.append(c, off, n);
    }
  }

  void write(String s, int off, int len) {
    if (len <= 0) {
      return;
    }
    total += utf8Length(s, off, len);
    if (chars == null) {
      chars = new StringBuilder(Math.min(cap, Math.max(len, 256)));
    }
    int n = Math.min(len, cap - chars.length());
    if (n > 0) {
      chars.append(s, off, off + n);
    }
  }

  /** The kept bytes or text, or null when nothing was written. */
  Object content() {
    if (byteCount > 0) {
      return Arrays.copyOf(bytes, byteCount);
    }
    if (chars != null && chars.length() > 0) {
      return chars.toString();
    }
    return null;
  }

  boolean isEmpty() {
    return total == 0;
  }

  /** Bytes written in total (characters counted as UTF-8). */
  long total() {
    return total;
  }

  void reset() {
    byteCount = 0;
    if (chars != null) {
      chars.setLength(0);
    }
    total = 0;
  }

  private void ensure(int size) {
    if (bytes == null) {
      bytes = new byte[Math.min(cap, Math.max(size, 512))];
    } else if (size > bytes.length) {
      bytes = Arrays.copyOf(bytes, Math.min(cap, Math.max(size, bytes.length * 2)));
    }
  }

  private static long utf8Length(char[] c, int off, int len) {
    long n = 0;
    for (int i = off; i < off + len; i++) {
      char ch = c[i];
      n += ch < 0x80 ? 1 : ch < 0x800 ? 2 : Character.isSurrogate(ch) ? 2 : 3;
    }
    return n;
  }

  private static long utf8Length(String s, int off, int len) {
    long n = 0;
    for (int i = off; i < off + len; i++) {
      char ch = s.charAt(i);
      n += ch < 0x80 ? 1 : ch < 0x800 ? 2 : Character.isSurrogate(ch) ? 2 : 3;
    }
    return n;
  }
}
