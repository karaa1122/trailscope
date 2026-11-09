package pro.trailscope.spring;

import pro.trailscope.BodyCapture;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.springframework.boot.context.properties.ConfigurationProperties;

/**
 * {@code trailscope.*} settings. Anything left unset falls back to the {@code TRAILSCOPE_*}
 * environment variables, then to the SDK defaults.
 */
@ConfigurationProperties(prefix = "trailscope")
public class TrailscopeProperties {
  /** Trailscope's base URL (env TRAILSCOPE_ENDPOINT; default http://localhost:8080). */
  private String endpoint;

  /** The project's ingest key (env TRAILSCOPE_INGEST_KEY). Without one nothing is sent. */
  private String ingestKey;

  /** Service name (env TRAILSCOPE_SERVICE; default spring.application.name). */
  private String service;

  /** Environment name (env TRAILSCOPE_ENVIRONMENT; default production). */
  private String environment;

  /** Running version (env TRAILSCOPE_RELEASE). */
  private String release;

  /** Set to false to turn Trailscope off (env TRAILSCOPE_ENABLED=0). */
  private Boolean enabled;

  /** Log SDK diagnostics on the pro.trailscope logger (env TRAILSCOPE_DEBUG=1). */
  private Boolean debug;

  /** Share of successful, fast requests to keep, in [0, 1]. Overrides the project setting. */
  private Double sampleRate;

  /** Per-route sample rates, e.g. trailscope.route-sample-rates[/search]=0.25. */
  private Map<String, Double> routeSampleRates;

  /** Requests at least this slow (ms) are always kept with their bodies. */
  private Double slowMs;

  /** When bodies are kept: errors, always or never. */
  private BodyCapture captureBodies;

  /** Bytes kept per body. */
  private Integer maxBodyBytes;

  /** Routes and paths that aren't recorded (exact). Replaces the default health routes. */
  private List<String> ignoreRoutes;

  /** Regular expressions of routes and paths that aren't recorded, on top of the list. */
  private List<String> ignoreRoutePatterns = new ArrayList<>();

  /** Don't record the actuator endpoints (management.endpoints.web.base-path). */
  private boolean ignoreActuator = true;

  /** Header carrying the request ID in and out. */
  private String requestIdHeader;

  /** Header names redacted on top of the built-in ones. */
  private List<String> redactHeaders = new ArrayList<>();

  /** Body keys redacted on top of the built-in ones (dotted paths and * work). */
  private List<String> redactKeys = new ArrayList<>();

  /** Regular expressions replaced in values, headers, the query and messages. */
  private List<String> redactPatterns = new ArrayList<>();

  /** Mask the last IPv4 octet. Overrides the project setting. */
  private Boolean maskIp;

  /** Drop the built-in redaction rules. Not recommended. */
  private boolean disableDefaultRedaction;

  /** Load sampling and capture settings from /v1/config. */
  private boolean remoteConfig = true;

  /** Record the authenticated principal's name as the user when setUser wasn't called. */
  private boolean capturePrincipal = true;

  /** Items kept in memory before the oldest are dropped. */
  private Integer maxQueueSize;

  /** How often the background thread sends. */
  private Duration flushInterval;

  /** Queue size that triggers an early flush. */
  private Integer flushAt;

  /** Items per request to /v1/ingest. */
  private Integer maxBatchSize;

  /** Timeout of each HTTP call to Trailscope. */
  private Duration requestTimeout;

  /** How long shutdown waits for the final flush. */
  private Duration shutdownTimeout;

  private final Filter filter = new Filter();

  private final Logback logback = new Logback();

  /** The request filter. */
  public static class Filter {
    /** Filter order; the default runs it before every other filter. */
    private int order = Integer.MIN_VALUE;

    public int getOrder() {
      return order;
    }

    public void setOrder(int order) {
      this.order = order;
    }
  }

  /** The Logback appender, attached to the root logger when Logback is in use. */
  public static class Logback {
    /** Attach the appender. */
    private boolean enabled = true;

    /** Lowest level sent. */
    private String level = "WARN";

    public boolean isEnabled() {
      return enabled;
    }

    public void setEnabled(boolean enabled) {
      this.enabled = enabled;
    }

    public String getLevel() {
      return level;
    }

    public void setLevel(String level) {
      this.level = level;
    }
  }

  public String getEndpoint() {
    return endpoint;
  }

  public void setEndpoint(String endpoint) {
    this.endpoint = endpoint;
  }

  public String getIngestKey() {
    return ingestKey;
  }

  public void setIngestKey(String ingestKey) {
    this.ingestKey = ingestKey;
  }

  public String getService() {
    return service;
  }

  public void setService(String service) {
    this.service = service;
  }

  public String getEnvironment() {
    return environment;
  }

  public void setEnvironment(String environment) {
    this.environment = environment;
  }

  public String getRelease() {
    return release;
  }

  public void setRelease(String release) {
    this.release = release;
  }

  public Boolean getEnabled() {
    return enabled;
  }

  public void setEnabled(Boolean enabled) {
    this.enabled = enabled;
  }

  public Boolean getDebug() {
    return debug;
  }

  public void setDebug(Boolean debug) {
    this.debug = debug;
  }

  public Double getSampleRate() {
    return sampleRate;
  }

  public void setSampleRate(Double sampleRate) {
    this.sampleRate = sampleRate;
  }

  public Map<String, Double> getRouteSampleRates() {
    return routeSampleRates;
  }

  public void setRouteSampleRates(Map<String, Double> routeSampleRates) {
    this.routeSampleRates = routeSampleRates == null ? null : new LinkedHashMap<>(routeSampleRates);
  }

  public Double getSlowMs() {
    return slowMs;
  }

  public void setSlowMs(Double slowMs) {
    this.slowMs = slowMs;
  }

  public BodyCapture getCaptureBodies() {
    return captureBodies;
  }

  public void setCaptureBodies(BodyCapture captureBodies) {
    this.captureBodies = captureBodies;
  }

  public Integer getMaxBodyBytes() {
    return maxBodyBytes;
  }

  public void setMaxBodyBytes(Integer maxBodyBytes) {
    this.maxBodyBytes = maxBodyBytes;
  }

  public List<String> getIgnoreRoutes() {
    return ignoreRoutes;
  }

  public void setIgnoreRoutes(List<String> ignoreRoutes) {
    this.ignoreRoutes = ignoreRoutes;
  }

  public List<String> getIgnoreRoutePatterns() {
    return ignoreRoutePatterns;
  }

  public void setIgnoreRoutePatterns(List<String> ignoreRoutePatterns) {
    this.ignoreRoutePatterns = ignoreRoutePatterns;
  }

  public boolean isIgnoreActuator() {
    return ignoreActuator;
  }

  public void setIgnoreActuator(boolean ignoreActuator) {
    this.ignoreActuator = ignoreActuator;
  }

  public String getRequestIdHeader() {
    return requestIdHeader;
  }

  public void setRequestIdHeader(String requestIdHeader) {
    this.requestIdHeader = requestIdHeader;
  }

  public List<String> getRedactHeaders() {
    return redactHeaders;
  }

  public void setRedactHeaders(List<String> redactHeaders) {
    this.redactHeaders = redactHeaders;
  }

  public List<String> getRedactKeys() {
    return redactKeys;
  }

  public void setRedactKeys(List<String> redactKeys) {
    this.redactKeys = redactKeys;
  }

  public List<String> getRedactPatterns() {
    return redactPatterns;
  }

  public void setRedactPatterns(List<String> redactPatterns) {
    this.redactPatterns = redactPatterns;
  }

  public Boolean getMaskIp() {
    return maskIp;
  }

  public void setMaskIp(Boolean maskIp) {
    this.maskIp = maskIp;
  }

  public boolean isDisableDefaultRedaction() {
    return disableDefaultRedaction;
  }

  public void setDisableDefaultRedaction(boolean disableDefaultRedaction) {
    this.disableDefaultRedaction = disableDefaultRedaction;
  }

  public boolean isRemoteConfig() {
    return remoteConfig;
  }

  public void setRemoteConfig(boolean remoteConfig) {
    this.remoteConfig = remoteConfig;
  }

  public boolean isCapturePrincipal() {
    return capturePrincipal;
  }

  public void setCapturePrincipal(boolean capturePrincipal) {
    this.capturePrincipal = capturePrincipal;
  }

  public Integer getMaxQueueSize() {
    return maxQueueSize;
  }

  public void setMaxQueueSize(Integer maxQueueSize) {
    this.maxQueueSize = maxQueueSize;
  }

  public Duration getFlushInterval() {
    return flushInterval;
  }

  public void setFlushInterval(Duration flushInterval) {
    this.flushInterval = flushInterval;
  }

  public Integer getFlushAt() {
    return flushAt;
  }

  public void setFlushAt(Integer flushAt) {
    this.flushAt = flushAt;
  }

  public Integer getMaxBatchSize() {
    return maxBatchSize;
  }

  public void setMaxBatchSize(Integer maxBatchSize) {
    this.maxBatchSize = maxBatchSize;
  }

  public Duration getRequestTimeout() {
    return requestTimeout;
  }

  public void setRequestTimeout(Duration requestTimeout) {
    this.requestTimeout = requestTimeout;
  }

  public Duration getShutdownTimeout() {
    return shutdownTimeout;
  }

  public void setShutdownTimeout(Duration shutdownTimeout) {
    this.shutdownTimeout = shutdownTimeout;
  }

  public Filter getFilter() {
    return filter;
  }

  public Logback getLogback() {
    return logback;
  }
}
