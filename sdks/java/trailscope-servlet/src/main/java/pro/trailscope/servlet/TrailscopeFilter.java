package pro.trailscope.servlet;

import pro.trailscope.BodyCapture;
import pro.trailscope.ContextScope;
import pro.trailscope.HttpExchange;
import pro.trailscope.Trailscope;
import pro.trailscope.TrailscopeClient;
import pro.trailscope.TrailscopeContext;
import pro.trailscope.TrailscopeOptions;
import jakarta.servlet.AsyncEvent;
import jakarta.servlet.AsyncListener;
import jakarta.servlet.Filter;
import jakarta.servlet.FilterChain;
import jakarta.servlet.FilterConfig;
import jakarta.servlet.RequestDispatcher;
import jakarta.servlet.ServletException;
import jakarta.servlet.ServletRequest;
import jakarta.servlet.ServletResponse;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletResponse;
import java.io.IOException;
import java.security.Principal;
import java.util.Collection;
import java.util.Enumeration;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Function;

/**
 * Records every request to Trailscope. Register it first in the filter chain, for {@code
 * /*}:
 *
 * <pre>{@code
 * <filter>
 *   <filter-name>trailscope</filter-name>
 *   <filter-class>pro.trailscope.servlet.TrailscopeFilter</filter-class>
 *   <async-supported>true</async-supported>
 * </filter>
 * <filter-mapping>
 *   <filter-name>trailscope</filter-name>
 *   <url-pattern>/*</url-pattern>
 * </filter-mapping>
 * }</pre>
 *
 * <p>Per request it builds the context (request ID from {@code X-Request-Id}, W3C {@code
 * traceparent}), returns the request ID in the response, makes the context current while the
 * request runs, copies at most {@code maxBodyBytes + 1} bytes of each body as the application
 * reads and writes it, and records the exchange when the response is done (async requests
 * included). Without a client passed in, it uses the process-wide one from {@link
 * Trailscope#init()}, creating it from the {@code TRAILSCOPE_*} environment variables and the
 * filter's init parameters ({@code endpoint}, {@code service}, {@code environment}, {@code
 * release}) if there is none.
 */
public class TrailscopeFilter implements Filter {
  static final String STATE = TrailscopeFilter.class.getName() + ".state";

  private volatile TrailscopeClient client;
  private volatile boolean ownsClient;
  private Function<HttpServletRequest, String> routeResolver;

  /** Uses the process-wide client. */
  public TrailscopeFilter() {}

  /** Uses this client. */
  public TrailscopeFilter(TrailscopeClient client) {
    this.client = client;
  }

  /**
   * Adds a way to read the route pattern for frameworks the filter doesn't know, consulted after
   * {@link Trailscope#setRoute(String)} and before Spring MVC's attribute. Return null when
   * unknown.
   */
  public TrailscopeFilter routeResolver(Function<HttpServletRequest, String> resolver) {
    this.routeResolver = resolver;
    return this;
  }

  /** The Trailscope context of a request this filter is recording, or null. */
  public static TrailscopeContext contextOf(ServletRequest request) {
    Object state = request.getAttribute(STATE);
    return state instanceof Exchange e ? e.ctx : null;
  }

  @Override
  public void init(FilterConfig config) {
    if (client != null) {
      return;
    }
    TrailscopeClient global = Trailscope.client();
    if (global != null) {
      client = global;
      return;
    }
    TrailscopeOptions options = new TrailscopeOptions();
    if (config != null) {
      String v;
      if ((v = config.getInitParameter("endpoint")) != null) {
        options.endpoint(v);
      }
      if ((v = config.getInitParameter("service")) != null) {
        options.service(v);
      }
      if ((v = config.getInitParameter("environment")) != null) {
        options.environment(v);
      }
      if ((v = config.getInitParameter("release")) != null) {
        options.release(v);
      }
    }
    client = Trailscope.init(options);
    ownsClient = true;
  }

  private TrailscopeClient client() {
    TrailscopeClient c = client;
    return c != null ? c : Trailscope.client();
  }

  @Override
  public void doFilter(ServletRequest req, ServletResponse res, FilterChain chain)
      throws IOException, ServletException {
    TrailscopeClient c = client();
    if (c == null
        || !c.isEnabled()
        || !(req instanceof HttpServletRequest request)
        || !(res instanceof HttpServletResponse response)) {
      chain.doFilter(req, res);
      return;
    }
    if (req.getAttribute(STATE) instanceof Exchange existing) {
      // A forward, include, async or error dispatch of a request already being recorded.
      try (ContextScope scope = Trailscope.useContext(existing.ctx)) {
        chain.doFilter(req, res);
      }
      return;
    }

    Exchange ex;
    try {
      ex = start(c, request, response);
    } catch (RuntimeException e) {
      chain.doFilter(req, res);
      return;
    }
    Throwable failure = null;
    try (ContextScope scope = Trailscope.useContext(ex.ctx)) {
      chain.doFilter(ex.request, ex.response);
    } catch (IOException | ServletException | RuntimeException | Error e) {
      failure = e;
      throw e;
    } finally {
      ex.afterChain(failure);
    }
  }

  private Exchange start(
      TrailscopeClient c, HttpServletRequest request, HttpServletResponse response) {
    TrailscopeContext ctx = c.contextFromHeaders(request::getHeader);
    String header = c.requestIdHeader();
    if (!response.isCommitted() && !response.containsHeader(header)) {
      response.setHeader(header, ctx.requestId());
    }
    int cap = c.captureBodies() == BodyCapture.NEVER ? 0 : c.maxBodyBytes() + 1;
    CapturingRequest wreq = new CapturingRequest(request, cap);
    CapturingResponse wres = new CapturingResponse(response, cap);
    wreq.setResponse(wres);
    Exchange ex = new Exchange(this, c, ctx, request, wreq, wres);
    request.setAttribute(STATE, ex);
    return ex;
  }

  /** One request being recorded. */
  static final class Exchange implements AsyncListener {
    final TrailscopeFilter filter;
    final TrailscopeClient client;
    final TrailscopeContext ctx;
    final HttpServletRequest original;
    final CapturingRequest request;
    final CapturingResponse response;
    final long timestamp = System.currentTimeMillis();
    final long t0 = System.nanoTime();
    private final AtomicBoolean done = new AtomicBoolean();
    private volatile Throwable asyncError;
    private volatile Object matchedPattern;

    Exchange(
        TrailscopeFilter filter,
        TrailscopeClient client,
        TrailscopeContext ctx,
        HttpServletRequest original,
        CapturingRequest request,
        CapturingResponse response) {
      this.filter = filter;
      this.client = client;
      this.ctx = ctx;
      this.original = original;
      this.request = request;
      this.response = response;
    }

    void afterChain(Throwable failure) {
      try {
        // Read now: an error dispatch later replaces Spring's matched pattern with /error.
        matchedPattern = original.getAttribute(Routes.SPRING_BEST_MATCHING_PATTERN);
        if (failure == null && original.isAsyncStarted()) {
          original.getAsyncContext().addListener(this);
          return;
        }
      } catch (RuntimeException e) {
        // The async context is gone already: record now.
      }
      finish(failure, false);
    }

    @Override
    public void onComplete(AsyncEvent event) {
      finish(asyncError, true);
    }

    @Override
    public void onTimeout(AsyncEvent event) {
      // The container (or the framework's timeout handler) sets the status.
    }

    @Override
    public void onError(AsyncEvent event) {
      if (event.getThrowable() != null) {
        asyncError = event.getThrowable();
      }
    }

    @Override
    public void onStartAsync(AsyncEvent event) {
      if (event.getAsyncContext() != null) {
        event.getAsyncContext().addListener(this);
      }
    }

    void finish(Throwable failure, boolean async) {
      if (!done.compareAndSet(false, true)) {
        return;
      }
      try {
        record(failure, async);
      } catch (RuntimeException e) {
        // Never fail the request because of Trailscope.
      }
    }

    private void record(Throwable failure, boolean async) {
      double durationMs = (System.nanoTime() - t0) / 1e6;
      if (failure != null) {
        response.noteFailure(failure);
      }
      int status;
      if (response.clientGone()) {
        status = 499;
      } else if (failure != null && !async && !response.isCommitted()) {
        status = 500; // what the container answers for an unhandled exception
      } else {
        status = response.getStatus();
      }
      Throwable error = failure;
      if (error == null
          && original.getAttribute(RequestDispatcher.ERROR_EXCEPTION) instanceof Throwable t) {
        error = t;
      }
      if (error != null) {
        ctx.recordError(error);
      }

      String route = ctx.route();
      if (route == null && filter.routeResolver != null) {
        route = filter.routeResolver.apply(original);
      }
      if (route == null) {
        route = Routes.resolve(original, matchedPattern, status);
      }

      if (ctx.userId() == null && client.capturePrincipal()) {
        try {
          Principal p = original.getUserPrincipal();
          if (p != null) {
            ctx.setUser(p.getName());
          }
        } catch (RuntimeException e) {
          // No principal available.
        }
      }

      Map<String, String> reqHeaders = requestHeaders(original);
      Map<String, String> resHeaders = responseHeaders(response);
      String query = original.getQueryString();
      String url = original.getRequestURI();
      if (query != null && !query.isEmpty()) {
        url = url + "?" + query;
      }
      long reqLength = original.getContentLengthLong();
      Long reqSize = null;
      if (reqLength >= 0) {
        reqSize = reqLength;
      } else if (request.bytesRead() > 0) {
        reqSize = request.bytesRead();
      }
      long written = response.bytesWritten();
      Long resSize = null;
      String length = resHeaders.get("content-length");
      if (length != null) {
        try {
          resSize = Long.parseLong(length.trim());
        } catch (NumberFormatException e) {
          resSize = null;
        }
      }
      if (resSize == null && written > 0) {
        resSize = written;
      }

      HttpExchange exchange =
          new HttpExchange()
              .timestamp(timestamp)
              .durationMs(durationMs)
              .method(original.getMethod())
              .url(url)
              .status(status)
              .route(route)
              .reqHeaders(reqHeaders)
              .resHeaders(resHeaders)
              .ip(clientIp(original))
              .reqSize(reqSize)
              .resSize(resSize)
              .error(error);
      // Bodies are looked at only when the capture policy keeps them.
      if (client.shouldCaptureBodies(status, durationMs)) {
        int limit = client.maxBodyBytes();
        exchange.reqBody(request.capturedBody(limit)).resBody(response.capturedBody());
      }
      client.recordHttp(ctx, exchange);
    }
  }

  private static Map<String, String> requestHeaders(HttpServletRequest request) {
    Map<String, String> out = new LinkedHashMap<>();
    Enumeration<String> names = request.getHeaderNames();
    if (names == null) {
      return out;
    }
    while (names.hasMoreElements()) {
      String name = names.nextElement();
      Enumeration<String> values = request.getHeaders(name);
      if (values == null) {
        continue;
      }
      StringBuilder sb = new StringBuilder();
      while (values.hasMoreElements()) {
        if (sb.length() > 0) {
          sb.append(", ");
        }
        sb.append(values.nextElement());
      }
      out.put(name.toLowerCase(Locale.ROOT), sb.toString());
    }
    return out;
  }

  private static Map<String, String> responseHeaders(HttpServletResponse response) {
    Map<String, String> out = new LinkedHashMap<>();
    Collection<String> names = response.getHeaderNames();
    if (names != null) {
      for (String name : names) {
        Collection<String> values = response.getHeaders(name);
        if (values == null || values.isEmpty()) {
          continue;
        }
        out.put(name.toLowerCase(Locale.ROOT), String.join(", ", values));
      }
    }
    String ct = response.getContentType();
    if (ct != null && !out.containsKey("content-type")) {
      out.put("content-type", ct);
    }
    return out;
  }

  /** The first {@code X-Forwarded-For} hop, {@code X-Real-IP}, then the socket address. */
  static String clientIp(HttpServletRequest request) {
    String xff = request.getHeader("x-forwarded-for");
    if (xff != null && !xff.isBlank()) {
      int comma = xff.indexOf(',');
      String first = (comma < 0 ? xff : xff.substring(0, comma)).trim();
      if (!first.isEmpty()) {
        return first;
      }
    }
    String real = request.getHeader("x-real-ip");
    if (real != null && !real.isBlank()) {
      return real.trim();
    }
    return request.getRemoteAddr();
  }

  /** Flushes and stops the client if {@link #init} created it (the application is undeployed). */
  @Override
  public void destroy() {
    TrailscopeClient c = client;
    if (ownsClient && c != null) {
      c.shutdown();
    }
  }
}
