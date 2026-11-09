package dev.trailscope;

import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.StringJoiner;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Redaction of headers, bodies, query strings and IPs. Same rules as the Node and Python SDKs. */
final class Redactor {
  static final String REDACTED = "[REDACTED]";

  static final List<String> DEFAULT_HEADERS =
      List.of("authorization", "cookie", "set-cookie", "x-api-key");

  static final List<String> DEFAULT_KEYS =
      List.of(
          "password",
          "passwd",
          "secret",
          "token",
          "access_token",
          "refresh_token",
          "otp",
          "pin",
          "cvv",
          "card",
          "card_number",
          "iban",
          "ssn");

  private static final Pattern IPV4 =
      Pattern.compile("^(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})\\.\\d{1,3}$");

  private final Set<String> headers = new HashSet<>();
  private final Set<String> keys = new HashSet<>();
  private final List<String[]> paths = new ArrayList<>();
  private final List<Pattern> patterns;
  private final Pattern rawKeys;
  final boolean maskIp;

  Redactor(
      Collection<String> extraHeaders,
      Collection<String> extraKeys,
      Collection<Pattern> patterns,
      boolean maskIp,
      boolean disableDefaults) {
    List<String> simple = new ArrayList<>(disableDefaults ? List.of() : DEFAULT_KEYS);
    if (!disableDefaults) {
      headers.addAll(DEFAULT_HEADERS);
    }
    for (String h : extraHeaders) {
      headers.add(h.toLowerCase(Locale.ROOT));
    }
    for (String k : extraKeys) {
      if (k.contains(".")) {
        String[] parts = k.split("\\.", -1);
        for (int i = 0; i < parts.length; i++) {
          parts[i] = normKey(parts[i]);
        }
        paths.add(parts);
      } else if (!k.isEmpty()) {
        simple.add(k);
      }
    }
    for (String k : simple) {
      keys.add(normKey(k));
    }
    this.patterns = List.copyOf(patterns);
    this.maskIp = maskIp;
    if (simple.isEmpty()) {
      rawKeys = null;
    } else {
      StringJoiner alts = new StringJoiner("|");
      for (String k : simple) {
        StringJoiner parts = new StringJoiner("[_-]?");
        for (String part : k.split("_", -1)) {
          parts.add(Pattern.quote(part));
        }
        alts.add(parts.toString());
      }
      // Possessive quantifiers keep long values from recursing deeply in java.util.regex.
      rawKeys =
          Pattern.compile(
              "(\"(?:" + alts + ")\"\\s*+:\\s*+)(\"(?:[^\"\\\\]++|\\\\.)*+\"?|[^,}\\]\\s]++)",
              Pattern.CASE_INSENSITIVE);
    }
  }

  /** {@code accessToken}, {@code access-token} and {@code ACCESS_TOKEN} all become one key. */
  static String normKey(String k) {
    StringBuilder sb = new StringBuilder(k.length());
    for (int i = 0; i < k.length(); i++) {
      char c = k.charAt(i);
      if (c != '_' && c != '-') {
        sb.append(Character.toLowerCase(c));
      }
    }
    return sb.toString();
  }

  Map<String, String> redactHeaders(Map<?, ?> in) {
    Map<String, String> out = new LinkedHashMap<>();
    for (Map.Entry<?, ?> e : in.entrySet()) {
      String lower = String.valueOf(e.getKey()).toLowerCase(Locale.ROOT);
      out.put(
          lower, headers.contains(lower) ? REDACTED : applyPatterns(String.valueOf(e.getValue())));
    }
    return out;
  }

  String redactBody(String body, String contentType) {
    if (body == null || body.isEmpty()) {
      return body;
    }
    String t = body.stripLeading();
    if (t.startsWith("{") || t.startsWith("[")) {
      Object parsed;
      try {
        parsed = Json.parse(body);
      } catch (IllegalArgumentException e) {
        return redactRawText(body);
      }
      return Json.write(redactValue(parsed, new ArrayList<>()));
    }
    if (contentType != null && contentType.contains("x-www-form-urlencoded")) {
      return redactUrlencoded(body);
    }
    return redactRawText(body);
  }

  String redactUrlencoded(String s) {
    StringJoiner out = new StringJoiner("&");
    for (String pair : s.split("&", -1)) {
      int eq = pair.indexOf('=');
      if (eq < 0) {
        out.add(pair);
        continue;
      }
      String k = pair.substring(0, eq);
      String decoded;
      try {
        decoded = URLDecoder.decode(k, StandardCharsets.UTF_8);
      } catch (IllegalArgumentException e) {
        decoded = k;
      }
      if (keys.contains(normKey(decoded))) {
        out.add(k + "=" + REDACTED);
      } else {
        out.add(k + "=" + applyPatterns(pair.substring(eq + 1)));
      }
    }
    return out.toString();
  }

  String redactQuery(String q) {
    if (q == null || q.isEmpty()) {
      return q;
    }
    return redactUrlencoded(q.startsWith("?") ? q.substring(1) : q);
  }

  /** Redacts a parsed JSON value: keys at any depth, dotted paths, and patterns in strings. */
  Object redactValue(Object v, List<String> path) {
    if (v instanceof List<?> list) {
      List<Object> out = new ArrayList<>(list.size());
      for (Object item : list) {
        out.add(redactValue(item, append(path, "*")));
      }
      return out;
    }
    if (v instanceof Map<?, ?> map) {
      Map<String, Object> out = new LinkedHashMap<>();
      for (Map.Entry<?, ?> e : map.entrySet()) {
        String key = String.valueOf(e.getKey());
        String nk = normKey(key);
        List<String> p = append(path, nk);
        Object child = e.getValue();
        if ((keys.contains(nk) || pathMatches(p)) && child != null) {
          out.put(key, REDACTED);
        } else {
          out.put(key, redactValue(child, p));
        }
      }
      return out;
    }
    if (v instanceof String s) {
      return applyPatterns(s);
    }
    return v;
  }

  String applyPatterns(String s) {
    if (s == null) {
      return null;
    }
    for (Pattern p : patterns) {
      s = p.matcher(s).replaceAll(Matcher.quoteReplacement(REDACTED));
    }
    return s;
  }

  String ip(String ip) {
    if (ip == null || ip.isEmpty() || !maskIp) {
      return ip;
    }
    return maskIp(ip);
  }

  private String redactRawText(String s) {
    if (rawKeys != null) {
      s = rawKeys.matcher(s).replaceAll("$1\"" + Matcher.quoteReplacement(REDACTED) + "\"");
    }
    return applyPatterns(s);
  }

  private boolean pathMatches(List<String> path) {
    for (String[] p : paths) {
      if (p.length != path.size()) {
        continue;
      }
      boolean all = true;
      for (int i = 0; i < p.length; i++) {
        String want = p[i];
        String got = path.get(i);
        if (!(want.equals("*") || got.equals("*") || want.equals(got))) {
          all = false;
          break;
        }
      }
      if (all) {
        return true;
      }
    }
    return false;
  }

  private static List<String> append(List<String> path, String part) {
    List<String> out = new ArrayList<>(path.size() + 1);
    out.addAll(path);
    out.add(part);
    return out;
  }

  /** Zeroes the last IPv4 octet, or keeps the first 48 bits of an IPv6 address. */
  static String maskIp(String raw) {
    String ip = raw.trim();
    if (ip.startsWith("::ffff:") && ip.contains(".")) {
      ip = ip.substring(7);
    }
    Matcher m = IPV4.matcher(ip);
    if (m.matches()) {
      return m.group(1) + "." + m.group(2) + "." + m.group(3) + ".0";
    }
    if (ip.contains(":")) {
      String head = ip.split("::", -1)[0];
      List<String> parts = new ArrayList<>();
      for (String p : head.split(":")) {
        if (!p.isEmpty()) {
          parts.add(p);
        }
      }
      return String.join(":", parts.subList(0, Math.min(3, parts.size()))) + "::";
    }
    return "";
  }
}
