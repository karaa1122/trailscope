package pro.trailscope;

import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/** A small JSON writer and parser, enough for the wire format, bodies and /v1/config. */
final class Json {
  private static final int MAX_DEPTH = 256;
  private static final char[] HEX = "0123456789abcdef".toCharArray();

  private Json() {}

  static String write(Object value) {
    StringBuilder sb = new StringBuilder(64);
    write(sb, value, 0);
    return sb.toString();
  }

  static void write(StringBuilder sb, Object value) {
    write(sb, value, 0);
  }

  private static void write(StringBuilder sb, Object v, int depth) {
    if (depth > MAX_DEPTH) {
      sb.append("null");
    } else if (v == null) {
      sb.append("null");
    } else if (v instanceof String s) {
      string(sb, s);
    } else if (v instanceof Boolean) {
      sb.append(v);
    } else if (v instanceof Integer
        || v instanceof Long
        || v instanceof Short
        || v instanceof Byte
        || v instanceof BigInteger
        || v instanceof AtomicInteger
        || v instanceof AtomicLong) {
      sb.append(v);
    } else if (v instanceof BigDecimal d) {
      sb.append(d.toString());
    } else if (v instanceof Number n) {
      double d = n.doubleValue();
      if (Double.isNaN(d) || Double.isInfinite(d)) {
        sb.append("null");
      } else if (d == Math.rint(d) && Math.abs(d) < 1e15) {
        sb.append((long) d);
      } else {
        sb.append(d);
      }
    } else if (v instanceof Map<?, ?> m) {
      sb.append('{');
      boolean first = true;
      for (Map.Entry<?, ?> e : m.entrySet()) {
        if (!first) {
          sb.append(',');
        }
        first = false;
        string(sb, String.valueOf(e.getKey()));
        sb.append(':');
        write(sb, e.getValue(), depth + 1);
      }
      sb.append('}');
    } else if (v instanceof Collection<?> c) {
      sb.append('[');
      boolean first = true;
      for (Object o : c) {
        if (!first) {
          sb.append(',');
        }
        first = false;
        write(sb, o, depth + 1);
      }
      sb.append(']');
    } else if (v instanceof Object[] arr) {
      sb.append('[');
      for (int i = 0; i < arr.length; i++) {
        if (i > 0) {
          sb.append(',');
        }
        write(sb, arr[i], depth + 1);
      }
      sb.append(']');
    } else {
      string(sb, String.valueOf(v));
    }
  }

  static void string(StringBuilder sb, String s) {
    sb.append('"');
    int n = s.length();
    int start = 0;
    for (int i = 0; i < n; i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c != '"' && c != '\\') {
        continue;
      }
      sb.append(s, start, i);
      start = i + 1;
      switch (c) {
        case '"':
          sb.append("\\\"");
          break;
        case '\\':
          sb.append("\\\\");
          break;
        case '\n':
          sb.append("\\n");
          break;
        case '\r':
          sb.append("\\r");
          break;
        case '\t':
          sb.append("\\t");
          break;
        case '\b':
          sb.append("\\b");
          break;
        case '\f':
          sb.append("\\f");
          break;
        default:
          sb.append("\\u00").append(HEX[(c >> 4) & 0xf]).append(HEX[c & 0xf]);
      }
    }
    sb.append(s, start, n);
    sb.append('"');
  }

  /**
   * Parses a JSON document into maps ({@link LinkedHashMap}), lists, strings, numbers ({@link
   * Long}, {@link BigInteger} or {@link BigDecimal}), booleans and null.
   *
   * @throws IllegalArgumentException when the text isn't valid JSON
   */
  static Object parse(String text) {
    Parser p = new Parser(text);
    p.skipWs();
    Object v = p.value(0);
    p.skipWs();
    if (p.pos != text.length()) {
      throw p.error("trailing characters");
    }
    return v;
  }

  private static final class Parser {
    private final String s;
    private int pos;

    Parser(String s) {
      this.s = s;
    }

    IllegalArgumentException error(String what) {
      return new IllegalArgumentException("invalid JSON at " + pos + ": " + what);
    }

    void skipWs() {
      while (pos < s.length()) {
        char c = s.charAt(pos);
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
          pos++;
        } else {
          break;
        }
      }
    }

    Object value(int depth) {
      if (depth > MAX_DEPTH) {
        throw error("too deep");
      }
      if (pos >= s.length()) {
        throw error("unexpected end");
      }
      char c = s.charAt(pos);
      switch (c) {
        case '{':
          return object(depth);
        case '[':
          return array(depth);
        case '"':
          return string();
        case 't':
          literal("true");
          return Boolean.TRUE;
        case 'f':
          literal("false");
          return Boolean.FALSE;
        case 'n':
          literal("null");
          return null;
        default:
          if (c == '-' || (c >= '0' && c <= '9')) {
            return number();
          }
          throw error("unexpected character");
      }
    }

    private void literal(String word) {
      if (!s.startsWith(word, pos)) {
        throw error("expected " + word);
      }
      pos += word.length();
    }

    private Map<String, Object> object(int depth) {
      pos++;
      Map<String, Object> out = new LinkedHashMap<>();
      skipWs();
      if (pos < s.length() && s.charAt(pos) == '}') {
        pos++;
        return out;
      }
      while (true) {
        skipWs();
        if (pos >= s.length() || s.charAt(pos) != '"') {
          throw error("expected a key");
        }
        String key = string();
        skipWs();
        if (pos >= s.length() || s.charAt(pos) != ':') {
          throw error("expected ':'");
        }
        pos++;
        skipWs();
        out.put(key, value(depth + 1));
        skipWs();
        if (pos >= s.length()) {
          throw error("unexpected end");
        }
        char c = s.charAt(pos++);
        if (c == '}') {
          return out;
        }
        if (c != ',') {
          throw error("expected ',' or '}'");
        }
      }
    }

    private List<Object> array(int depth) {
      pos++;
      List<Object> out = new ArrayList<>();
      skipWs();
      if (pos < s.length() && s.charAt(pos) == ']') {
        pos++;
        return out;
      }
      while (true) {
        skipWs();
        out.add(value(depth + 1));
        skipWs();
        if (pos >= s.length()) {
          throw error("unexpected end");
        }
        char c = s.charAt(pos++);
        if (c == ']') {
          return out;
        }
        if (c != ',') {
          throw error("expected ',' or ']'");
        }
      }
    }

    private String string() {
      pos++;
      StringBuilder sb = null;
      int start = pos;
      while (pos < s.length()) {
        char c = s.charAt(pos);
        if (c == '"') {
          String out = sb == null ? s.substring(start, pos) : sb.append(s, start, pos).toString();
          pos++;
          return out;
        }
        if (c < 0x20) {
          throw error("control character in string");
        }
        if (c == '\\') {
          if (sb == null) {
            sb = new StringBuilder();
          }
          sb.append(s, start, pos);
          pos++;
          if (pos >= s.length()) {
            break;
          }
          char e = s.charAt(pos++);
          switch (e) {
            case '"':
            case '\\':
            case '/':
              sb.append(e);
              break;
            case 'n':
              sb.append('\n');
              break;
            case 'r':
              sb.append('\r');
              break;
            case 't':
              sb.append('\t');
              break;
            case 'b':
              sb.append('\b');
              break;
            case 'f':
              sb.append('\f');
              break;
            case 'u':
              if (pos + 4 > s.length()) {
                throw error("bad unicode escape");
              }
              try {
                sb.append((char) Integer.parseInt(s.substring(pos, pos + 4), 16));
              } catch (NumberFormatException ex) {
                throw error("bad unicode escape");
              }
              pos += 4;
              break;
            default:
              throw error("bad escape");
          }
          start = pos;
        } else {
          pos++;
        }
      }
      throw error("unterminated string");
    }

    private Object number() {
      int start = pos;
      if (s.charAt(pos) == '-') {
        pos++;
      }
      if (pos >= s.length()) {
        throw error("bad number");
      }
      if (s.charAt(pos) == '0') {
        pos++;
      } else if (s.charAt(pos) >= '1' && s.charAt(pos) <= '9') {
        digits();
      } else {
        throw error("bad number");
      }
      boolean integral = true;
      if (pos < s.length() && s.charAt(pos) == '.') {
        integral = false;
        pos++;
        if (digits() == 0) {
          throw error("bad number");
        }
      }
      if (pos < s.length() && (s.charAt(pos) == 'e' || s.charAt(pos) == 'E')) {
        integral = false;
        pos++;
        if (pos < s.length() && (s.charAt(pos) == '+' || s.charAt(pos) == '-')) {
          pos++;
        }
        if (digits() == 0) {
          throw error("bad number");
        }
      }
      String text = s.substring(start, pos);
      if (integral) {
        if (text.length() < 19) {
          return Long.parseLong(text);
        }
        BigInteger big = new BigInteger(text);
        return big.bitLength() < 64 ? (Object) big.longValue() : big;
      }
      return new BigDecimal(text);
    }

    private int digits() {
      int start = pos;
      while (pos < s.length() && s.charAt(pos) >= '0' && s.charAt(pos) <= '9') {
        pos++;
      }
      return pos - start;
    }
  }
}
