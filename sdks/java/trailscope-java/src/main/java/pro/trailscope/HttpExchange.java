package pro.trailscope;

import java.util.Map;

/**
 * One finished HTTP exchange, as framework integrations report it to {@link
 * TrailscopeClient#recordHttp(TrailscopeContext, HttpExchange)}.
 */
public final class HttpExchange {
  long timestamp;
  double durationMs;
  String method = "GET";
  String url = "/";
  int status;
  String route;
  Map<String, String> reqHeaders;
  Map<String, String> resHeaders;
  Object reqBody;
  Object resBody;
  String ip;
  Long reqSize;
  Long resSize;
  Throwable error;

  /** When the request started, in epoch milliseconds. */
  public HttpExchange timestamp(long epochMillis) {
    this.timestamp = epochMillis;
    return this;
  }

  /** Duration, measured with a monotonic clock. */
  public HttpExchange durationMs(double durationMs) {
    this.durationMs = durationMs;
    return this;
  }

  public HttpExchange method(String method) {
    this.method = method;
    return this;
  }

  /** The path with the query string, e.g. {@code /orders/42?expand=items}. */
  public HttpExchange url(String url) {
    this.url = url;
    return this;
  }

  public HttpExchange status(int status) {
    this.status = status;
    return this;
  }

  /** The route pattern, e.g. {@code /orders/:id}. Falls back to the context's route. */
  public HttpExchange route(String route) {
    this.route = route;
    return this;
  }

  /** Request headers, names in any case; repeated values joined with {@code ", "}. */
  public HttpExchange reqHeaders(Map<String, String> reqHeaders) {
    this.reqHeaders = reqHeaders;
    return this;
  }

  /** Response headers, names in any case; repeated values joined with {@code ", "}. */
  public HttpExchange resHeaders(Map<String, String> resHeaders) {
    this.resHeaders = resHeaders;
    return this;
  }

  /**
   * The request body: the first {@code maxBodyBytes + 1} bytes ({@code byte[]}), text, or parsed
   * fields (a {@code Map}). One byte past the limit marks it as truncated.
   */
  public HttpExchange reqBody(Object reqBody) {
    this.reqBody = reqBody;
    return this;
  }

  /** The response body, like {@link #reqBody(Object)}. */
  public HttpExchange resBody(Object resBody) {
    this.resBody = resBody;
    return this;
  }

  /** The client IP. */
  public HttpExchange ip(String ip) {
    this.ip = ip;
    return this;
  }

  public HttpExchange reqSize(Long reqSize) {
    this.reqSize = reqSize;
    return this;
  }

  public HttpExchange resSize(Long resSize) {
    this.resSize = resSize;
    return this;
  }

  /** The exception that failed the request. Falls back to the context's error. */
  public HttpExchange error(Throwable error) {
    this.error = error;
    return this;
  }
}
