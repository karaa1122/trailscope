package pro.trailscope.bench;

import pro.trailscope.TrailscopeClient;
import pro.trailscope.TrailscopeOptions;
import pro.trailscope.servlet.Routes;
import pro.trailscope.servlet.TrailscopeFilter;
import jakarta.servlet.FilterChain;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletResponse;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.Arrays;
import java.util.Locale;
import org.springframework.mock.web.MockHttpServletRequest;
import org.springframework.mock.web.MockHttpServletResponse;

/**
 * Measures what the servlet filter adds to the request path: context setup, the request ID header,
 * body copying and recording the request (redaction, serialization and gzip happen later in the
 * flush thread). Mock requests keep network noise out. Exits with 1 if the added p99 reaches the
 * spec's 0.2 ms budget.
 */
public final class Overhead {
  private static final int N = 100_000;
  private static final byte[] PAYLOAD =
      "[{\"id\":\"t1\",\"title\":\"Write the brief\",\"done\":false}]"
          .getBytes(StandardCharsets.UTF_8);
  private static final byte[] BODY =
      "{\"title\":\"Draft roadmap\",\"tags\":[\"a\",\"b\"]}".getBytes(StandardCharsets.UTF_8);

  private Overhead() {}

  /** The application: reads the body, answers JSON; every 50th request fails with a 500. */
  private static void app(HttpServletRequest req, HttpServletResponse res) throws IOException {
    req.getInputStream().readAllBytes();
    req.setAttribute(Routes.SPRING_BEST_MATCHING_PATTERN, "/projects/{projectId}/tasks");
    res.setStatus(req.getRequestURI().endsWith("/fail") ? 500 : 200);
    res.setContentType("application/json");
    res.getOutputStream().write(PAYLOAD);
  }

  private static MockHttpServletRequest request(int i) {
    String path = "/projects/prj_" + (i % 100) + "/tasks" + (i % 50 == 0 ? "/fail" : "");
    MockHttpServletRequest req = new MockHttpServletRequest("POST", path);
    req.setQueryString("expand=owner");
    req.addHeader("host", "api.example.com");
    req.addHeader("content-type", "application/json");
    req.addHeader("authorization", "Bearer eyJhbGciOiJIUzI1NiJ9.bench");
    req.addHeader("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605");
    req.setContentType("application/json");
    req.setContent(BODY);
    req.setRemoteAddr("198.51.100.7");
    return req;
  }

  private static long[] run(TrailscopeFilter filter) throws Exception {
    long[] samples = new long[N];
    FilterChain chain = (rq, rs) -> app((HttpServletRequest) rq, (HttpServletResponse) rs);
    for (int i = 0; i < N; i++) {
      MockHttpServletRequest req = request(i);
      MockHttpServletResponse res = new MockHttpServletResponse();
      long t = System.nanoTime();
      if (filter == null) {
        app(req, res);
      } else {
        filter.doFilter(req, res, chain);
      }
      samples[i] = System.nanoTime() - t;
    }
    Arrays.sort(samples);
    return samples;
  }

  private static double pct(long[] sorted, double p) {
    return sorted[Math.min(sorted.length - 1, (int) (sorted.length * p))] / 1e6;
  }

  private static double mean(long[] samples) {
    double sum = 0;
    for (long s : samples) {
      sum += s;
    }
    return sum / samples.length / 1e6;
  }

  public static void main(String[] args) throws Exception {
    TrailscopeClient client =
        new TrailscopeClient(
            new TrailscopeOptions()
                .ingestKey("trs_ingest_bench")
                .service("bench")
                .endpoint("http://127.0.0.1:9") // never reached: flushing is paused
                .flushInterval(Duration.ofHours(1))
                .flushAt(Integer.MAX_VALUE)
                .remoteConfig(false));
    TrailscopeFilter filter = new TrailscopeFilter(client);
    // Warm up the JIT on both paths, then measure.
    run(null);
    run(filter);
    run(null);
    run(filter);
    long[] base = run(null);
    long[] sdk = run(filter);
    double addedP99 = pct(sdk, 0.99) - pct(base, 0.99);
    System.out.printf(
        Locale.ROOT,
        "{%n  \"java\": \"%s\",%n  \"name\": \"servlet-filter\",%n  \"iterations\": %d,%n"
            + "  \"baseline_p50_ms\": %.4f,%n  \"baseline_p99_ms\": %.4f,%n"
            + "  \"with_sdk_p50_ms\": %.4f,%n  \"with_sdk_p99_ms\": %.4f,%n"
            + "  \"added_p50_ms\": %.4f,%n  \"added_p99_ms\": %.4f,%n  \"added_mean_ms\": %.4f,%n"
            + "  \"dropped\": %d%n}%n",
        System.getProperty("java.version"),
        N,
        pct(base, 0.5),
        pct(base, 0.99),
        pct(sdk, 0.5),
        pct(sdk, 0.99),
        pct(sdk, 0.5) - pct(base, 0.5),
        addedP99,
        mean(sdk) - mean(base),
        client.stats().dropped());
    client.shutdown(Duration.ofMillis(10));
    System.exit(addedP99 >= 0.2 ? 1 : 0);
  }
}
