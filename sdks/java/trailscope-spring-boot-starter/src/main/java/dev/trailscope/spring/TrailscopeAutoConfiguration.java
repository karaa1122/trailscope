package dev.trailscope.spring;

import dev.trailscope.Trailscope;
import dev.trailscope.TrailscopeClient;
import dev.trailscope.TrailscopeOptions;
import dev.trailscope.logback.TrailscopeAppender;
import dev.trailscope.servlet.TrailscopeFilter;
import java.util.regex.Pattern;
import org.springframework.beans.factory.DisposableBean;
import org.springframework.beans.factory.ObjectProvider;
import org.springframework.boot.autoconfigure.AutoConfiguration;
import org.springframework.boot.autoconfigure.condition.ConditionalOnClass;
import org.springframework.boot.autoconfigure.condition.ConditionalOnMissingBean;
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty;
import org.springframework.boot.autoconfigure.condition.ConditionalOnWebApplication;
import org.springframework.boot.context.event.ApplicationStartedEvent;
import org.springframework.boot.context.properties.EnableConfigurationProperties;
import org.springframework.context.ApplicationListener;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import org.springframework.core.Ordered;
import org.springframework.core.env.Environment;
import org.springframework.core.task.TaskDecorator;
import org.springframework.web.servlet.config.annotation.InterceptorRegistry;
import org.springframework.web.servlet.config.annotation.WebMvcConfigurer;

/**
 * Sets up Trailscope in a Spring Boot application: the client (from {@code trailscope.*}
 * properties and {@code TRAILSCOPE_*} environment variables), the request filter, route patterns
 * and exceptions from Spring MVC, the authenticated user, context propagation into {@code @Async}
 * methods, and the Logback appender.
 */
@AutoConfiguration
@EnableConfigurationProperties(TrailscopeProperties.class)
public class TrailscopeAutoConfiguration {

  /** The client, also made the process-wide one ({@link Trailscope#client()}). */
  @Bean(destroyMethod = "shutdown")
  @ConditionalOnMissingBean
  public TrailscopeClient trailscopeClient(TrailscopeProperties properties, Environment env) {
    TrailscopeClient client = new TrailscopeClient(options(properties, env));
    Trailscope.setClient(client);
    return client;
  }

  /** Runs {@code @Async} methods and other tasks of Boot's executor in the caller's context. */
  @Bean
  @ConditionalOnMissingBean(TaskDecorator.class)
  public TaskDecorator trailscopeTaskDecorator() {
    return Trailscope::wrap;
  }

  static TrailscopeOptions options(TrailscopeProperties p, Environment env) {
    TrailscopeOptions o = new TrailscopeOptions().shutdownHook(false);
    o.endpoint(p.getEndpoint()).ingestKey(p.getIngestKey()).environment(p.getEnvironment());
    o.release(p.getRelease());
    String service = p.getService();
    if (isBlank(service) && isBlank(System.getenv("TRAILSCOPE_SERVICE"))) {
      service = env.getProperty("spring.application.name");
    }
    o.service(service);
    if (p.getEnabled() != null) {
      o.enabled(p.getEnabled());
    }
    if (p.getDebug() != null) {
      o.debug(p.getDebug());
    }
    if (p.getSampleRate() != null) {
      o.sampleRate(p.getSampleRate());
    }
    if (p.getRouteSampleRates() != null) {
      o.routeSampleRates(p.getRouteSampleRates());
    }
    if (p.getSlowMs() != null) {
      o.slowMs(p.getSlowMs());
    }
    if (p.getCaptureBodies() != null) {
      o.captureBodies(p.getCaptureBodies());
    }
    if (p.getMaxBodyBytes() != null) {
      o.maxBodyBytes(p.getMaxBodyBytes());
    }
    if (p.getIgnoreRoutes() != null) {
      o.ignoreRoutes(p.getIgnoreRoutes());
    }
    for (String pattern : p.getIgnoreRoutePatterns()) {
      o.ignoreRoutePattern(Pattern.compile(pattern));
    }
    if (p.isIgnoreActuator()) {
      String base = env.getProperty("management.endpoints.web.base-path", "/actuator");
      String context = env.getProperty("server.servlet.context-path", "");
      if (!base.isEmpty() && !base.equals("/")) {
        o.ignoreRoutePattern(Pattern.compile("^" + Pattern.quote(context + base) + "(/.*)?$"));
      }
    }
    if (p.getRequestIdHeader() != null) {
      o.requestIdHeader(p.getRequestIdHeader());
    }
    o.redactHeaders(p.getRedactHeaders()).redactKeys(p.getRedactKeys());
    o.redactPatterns(p.getRedactPatterns());
    if (p.getMaskIp() != null) {
      o.maskIp(p.getMaskIp());
    }
    o.disableDefaultRedaction(p.isDisableDefaultRedaction());
    o.remoteConfig(p.isRemoteConfig()).capturePrincipal(p.isCapturePrincipal());
    if (p.getMaxQueueSize() != null) {
      o.maxQueueSize(p.getMaxQueueSize());
    }
    if (p.getFlushInterval() != null) {
      o.flushInterval(p.getFlushInterval());
    }
    if (p.getFlushAt() != null) {
      o.flushAt(p.getFlushAt());
    }
    if (p.getMaxBatchSize() != null) {
      o.maxBatchSize(p.getMaxBatchSize());
    }
    if (p.getRequestTimeout() != null) {
      o.requestTimeout(p.getRequestTimeout());
    }
    if (p.getShutdownTimeout() != null) {
      o.shutdownTimeout(p.getShutdownTimeout());
    }
    return o;
  }

  private static boolean isBlank(String s) {
    return s == null || s.isBlank();
  }

  /** The filter, first in the chain. */
  @Configuration(proxyBeanMethods = false)
  @ConditionalOnWebApplication(type = ConditionalOnWebApplication.Type.SERVLET)
  @ConditionalOnClass(name = "jakarta.servlet.Filter")
  static class ServletConfiguration {
    @Bean
    @ConditionalOnMissingBean(TrailscopeFilter.class)
    OrderedTrailscopeFilter trailscopeFilter(TrailscopeClient client, TrailscopeProperties p) {
      return new OrderedTrailscopeFilter(client, p.getFilter().getOrder());
    }
  }

  /** Controller exceptions and the authenticated user, from Spring MVC. */
  @Configuration(proxyBeanMethods = false)
  @ConditionalOnWebApplication(type = ConditionalOnWebApplication.Type.SERVLET)
  @ConditionalOnClass(name = "org.springframework.web.servlet.DispatcherServlet")
  static class WebMvcConfiguration {
    @Bean
    TrailscopeExceptionResolver trailscopeExceptionResolver() {
      return new TrailscopeExceptionResolver();
    }

    @Bean
    WebMvcConfigurer trailscopeWebMvcConfigurer(TrailscopeClient client) {
      return new WebMvcConfigurer() {
        @Override
        public void addInterceptors(InterceptorRegistry registry) {
          if (client.capturePrincipal()) {
            registry
                .addInterceptor(new TrailscopeUserInterceptor())
                .order(Ordered.HIGHEST_PRECEDENCE);
          }
        }
      };
    }
  }

  /** The Logback appender on the root logger. */
  @Configuration(proxyBeanMethods = false)
  @ConditionalOnClass(name = "ch.qos.logback.classic.LoggerContext")
  @ConditionalOnProperty(prefix = "trailscope.logback", name = "enabled", matchIfMissing = true)
  static class LogbackConfiguration {
    @Bean
    LogbackInstaller trailscopeLogbackInstaller(
        ObjectProvider<TrailscopeClient> client, TrailscopeProperties p) {
      String level = p.getLogback().getLevel();
      // Until the application has started, only WARN and above: startup lines are noise.
      String startup =
          ch.qos.logback.classic.Level.toLevel(level, ch.qos.logback.classic.Level.WARN)
                  .isGreaterOrEqual(ch.qos.logback.classic.Level.WARN)
              ? level
              : "WARN";
      return new LogbackInstaller(
          TrailscopeAppender.install(client.getIfAvailable(), startup), level);
    }
  }

  /**
   * Lowers the appender's level to the configured one once the application has started, and
   * detaches it when the application stops.
   */
  static final class LogbackInstaller
      implements DisposableBean, ApplicationListener<ApplicationStartedEvent> {
    private final TrailscopeAppender appender;
    private final String level;

    LogbackInstaller(TrailscopeAppender appender, String level) {
      this.appender = appender;
      this.level = level;
    }

    @Override
    public void onApplicationEvent(ApplicationStartedEvent event) {
      if (appender != null && appender.isInstalledByCode()) {
        appender.setLevel(level);
      }
    }

    @Override
    public void destroy() {
      TrailscopeAppender.uninstall(appender);
    }
  }

  /** The servlet filter with an order, so Spring Boot registers it first. */
  static final class OrderedTrailscopeFilter extends TrailscopeFilter implements Ordered {
    private final int order;

    OrderedTrailscopeFilter(TrailscopeClient client, int order) {
      super(client);
      this.order = order;
    }

    @Override
    public int getOrder() {
      return order;
    }
  }
}
