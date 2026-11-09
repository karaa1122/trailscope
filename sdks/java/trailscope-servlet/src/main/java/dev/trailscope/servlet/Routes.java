package dev.trailscope.servlet;

import jakarta.servlet.http.HttpServletMapping;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.MappingMatch;

/** Route patterns from what the framework left on the request. */
public final class Routes {
  /** Spring MVC's {@code HandlerMapping.BEST_MATCHING_PATTERN_ATTRIBUTE}. */
  public static final String SPRING_BEST_MATCHING_PATTERN =
      "org.springframework.web.servlet.HandlerMapping.bestMatchingPattern";

  private Routes() {}

  /**
   * Converts a URI template to Trailscope's {@code :name} form: {@code /projects/{projectId}} and
   * {@code /files/{*path}} become {@code /projects/:projectId} and {@code /files/:path}; regular
   * expressions ({@code {id:\d+}}) are dropped.
   */
  public static String fromTemplate(String pattern) {
    if (pattern == null || pattern.indexOf('{') < 0) {
      return pattern;
    }
    StringBuilder sb = new StringBuilder(pattern.length());
    int i = 0;
    int n = pattern.length();
    while (i < n) {
      char c = pattern.charAt(i);
      if (c != '{') {
        sb.append(c);
        i++;
        continue;
      }
      int depth = 0;
      int j = i;
      for (; j < n; j++) {
        char d = pattern.charAt(j);
        if (d == '{') {
          depth++;
        } else if (d == '}' && --depth == 0) {
          break;
        }
      }
      if (j >= n) {
        sb.append(pattern, i, n);
        break;
      }
      String inner = pattern.substring(i + 1, j);
      if (inner.startsWith("*")) {
        inner = inner.substring(1);
      }
      int colon = inner.indexOf(':');
      String name = (colon >= 0 ? inner.substring(0, colon) : inner).trim();
      sb.append(':').append(name);
      i = j + 1;
    }
    return sb.toString();
  }

  /**
   * The route of a finished request: the pattern Spring MVC matched (with the context path and a
   * prefix servlet mapping in front), or the servlet's own exact mapping. Null when unknown, and
   * for 404s answered by a catch-all pattern, so the server derives a route from the path.
   */
  public static String resolve(HttpServletRequest request, int status) {
    return resolve(request, request.getAttribute(SPRING_BEST_MATCHING_PATTERN), status);
  }

  /** Same as {@link #resolve(HttpServletRequest, int)}, with Spring's pattern read earlier. */
  static String resolve(HttpServletRequest request, Object best, int status) {
    if (best != null) {
      String pattern = best.toString();
      if (status == 404 && pattern.contains("**")) {
        return null;
      }
      return prefix(request) + fromTemplate(pattern);
    }
    HttpServletMapping mapping = mapping(request);
    if (mapping != null && mapping.getMappingMatch() == MappingMatch.EXACT) {
      return request.getContextPath() + mapping.getPattern();
    }
    return null;
  }

  private static String prefix(HttpServletRequest request) {
    String context = request.getContextPath();
    HttpServletMapping mapping = mapping(request);
    if (mapping != null && mapping.getMappingMatch() == MappingMatch.PATH) {
      String servletPath = request.getServletPath();
      return context + (servletPath != null ? servletPath : "");
    }
    return context;
  }

  private static HttpServletMapping mapping(HttpServletRequest request) {
    try {
      return request.getHttpServletMapping();
    } catch (RuntimeException | AbstractMethodError e) {
      return null;
    }
  }
}
