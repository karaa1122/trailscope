package pro.trailscope.logback;

import ch.qos.logback.classic.Level;
import ch.qos.logback.classic.Logger;
import ch.qos.logback.classic.LoggerContext;
import ch.qos.logback.classic.spi.ILoggingEvent;
import ch.qos.logback.classic.spi.IThrowableProxy;
import ch.qos.logback.classic.spi.ThrowableProxy;
import ch.qos.logback.classic.spi.ThrowableProxyUtil;
import ch.qos.logback.core.Appender;
import ch.qos.logback.core.UnsynchronizedAppenderBase;
import pro.trailscope.Event;
import pro.trailscope.EventKind;
import pro.trailscope.Trailscope;
import pro.trailscope.TrailscopeClient;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import org.slf4j.ILoggerFactory;
import org.slf4j.LoggerFactory;

/**
 * Sends log lines to Trailscope as {@code log} events, linked to the current request or job (its
 * request ID, trace ID, user and correlation keys). The logger name becomes the event name and an
 * attached exception becomes the error.
 *
 * <pre>{@code
 * <appender name="TRAILSCOPE" class="pro.trailscope.logback.TrailscopeAppender">
 *   <level>WARN</level>
 * </appender>
 * <root level="INFO">
 *   <appender-ref ref="TRAILSCOPE" />
 * </root>
 * }</pre>
 *
 * <p>It sends {@code WARN} and above by default. It skips Trailscope's own logs, the servlet
 * container's report of an exception that failed a request (the request already carries it), and,
 * below {@code WARN}, framework loggers such as {@code org.springframework.} and {@code
 * org.apache.}. Don't put it behind
 * an {@code AsyncAppender}: the context lives on the logging thread, and the client never blocks.
 */
public class TrailscopeAppender extends UnsynchronizedAppenderBase<ILoggingEvent> {
  /** Logger name prefixes treated as framework noise below {@code WARN}. */
  public static final List<String> DEFAULT_FRAMEWORK_PREFIXES =
      List.of(
          "org.springframework.",
          "org.apache.",
          "org.hibernate.",
          "org.eclipse.jetty.",
          "io.undertow.",
          "org.xnio.",
          "io.netty.",
          "reactor.",
          "com.zaxxer.hikari.",
          "org.flywaydb.",
          "liquibase.",
          "org.jboss.",
          "jdk.",
          "sun.",
          "java.",
          "javax.",
          "jakarta.");

  private volatile TrailscopeClient client;
  private boolean installed;
  private volatile Level threshold = Level.WARN;
  private boolean skipFrameworkInfo = true;
  private final List<String> frameworkPrefixes = new ArrayList<>(DEFAULT_FRAMEWORK_PREFIXES);

  /** Uses this client instead of the process-wide one. */
  public void setClient(TrailscopeClient client) {
    this.client = client;
  }

  /** The lowest level sent, e.g. {@code INFO}; default {@code WARN}. */
  public void setLevel(String level) {
    this.threshold = Level.toLevel(level, Level.WARN);
  }

  public String getLevel() {
    return threshold.toString();
  }

  /** Whether {@link #install} attached this appender (rather than {@code logback.xml}). */
  public boolean isInstalledByCode() {
    return installed;
  }

  /** Whether framework loggers below {@code WARN} are skipped. Default true. */
  public void setSkipFrameworkInfo(boolean skip) {
    this.skipFrameworkInfo = skip;
  }

  /** Adds a logger name prefix treated as framework noise below {@code WARN}. */
  public void addFrameworkPrefix(String prefix) {
    if (prefix != null && !prefix.isEmpty()) {
      frameworkPrefixes.add(prefix);
    }
  }

  @Override
  protected void append(ILoggingEvent event) {
    try {
      Level level = event.getLevel();
      if (!level.isGreaterOrEqual(threshold)) {
        return;
      }
      String name = event.getLoggerName() != null ? event.getLoggerName() : "log";
      if (name.equals("pro.trailscope") || name.startsWith("pro.trailscope.")) {
        return;
      }
      if (skipFrameworkInfo && !level.isGreaterOrEqual(Level.WARN) && isFramework(name)) {
        return;
      }
      TrailscopeClient c = client != null ? client : Trailscope.client();
      if (c == null || !c.isEnabled()) {
        return;
      }
      IThrowableProxy proxy = event.getThrowableProxy();
      Throwable thrown = proxy instanceof ThrowableProxy tp ? tp.getThrowable() : null;
      if (proxy != null && (Trailscope.isRecorded(thrown) || isContainerReport(name, event))) {
        // The request already carries this error; the container is just reporting it.
        return;
      }
      Event e =
          Event.of(name)
              .kind(EventKind.LOG)
              .level(map(level))
              .message(event.getFormattedMessage())
              .timestamp(event.getTimeStamp())
              .attribute("thread", event.getThreadName());
      if (thrown != null) {
        e.error(thrown);
      } else if (proxy != null) {
        e.error(proxy.getClassName(), proxy.getMessage(), ThrowableProxyUtil.asString(proxy));
      }
      c.captureEvent(e);
    } catch (RuntimeException ex) {
      // Logging must never fail because of Trailscope.
    }
  }

  /** Tomcat's and Undertow's report of an exception that escaped the servlet (a 500). */
  private static boolean isContainerReport(String name, ILoggingEvent event) {
    String message = event.getMessage();
    if (message == null) {
      return false;
    }
    if (name.startsWith("org.apache.catalina.core.ContainerBase.")) {
      return message.startsWith("Servlet.service()");
    }
    return name.equals("io.undertow.request") && message.startsWith("UT005023");
  }

  private boolean isFramework(String name) {
    for (String p : frameworkPrefixes) {
      if (name.startsWith(p)) {
        return true;
      }
    }
    return false;
  }

  private static pro.trailscope.Level map(Level level) {
    if (level.isGreaterOrEqual(Level.ERROR)) {
      return pro.trailscope.Level.ERROR;
    }
    if (level.isGreaterOrEqual(Level.WARN)) {
      return pro.trailscope.Level.WARN;
    }
    if (level.isGreaterOrEqual(Level.INFO)) {
      return pro.trailscope.Level.INFO;
    }
    return pro.trailscope.Level.DEBUG;
  }

  /**
   * Attaches an appender to the root logger when Logback is the SLF4J backend. If one is already
   * attached (from {@code logback.xml}), that one is returned and given the client if it has none.
   *
   * @param client the client to send with, or null for the process-wide one
   * @param level the lowest level sent, e.g. {@code WARN}
   * @return the attached appender, or null when Logback isn't in use
   */
  public static TrailscopeAppender install(TrailscopeClient client, String level) {
    ILoggerFactory factory = LoggerFactory.getILoggerFactory();
    if (!(factory instanceof LoggerContext context)) {
      return null;
    }
    Logger root = context.getLogger(Logger.ROOT_LOGGER_NAME);
    for (Iterator<Appender<ILoggingEvent>> it = root.iteratorForAppenders(); it.hasNext(); ) {
      if (it.next() instanceof TrailscopeAppender existing) {
        if (existing.client == null && client != null) {
          existing.setClient(client);
        }
        return existing;
      }
    }
    TrailscopeAppender appender = new TrailscopeAppender();
    appender.setContext(context);
    appender.setName("TRAILSCOPE");
    appender.setClient(client);
    if (level != null) {
      appender.setLevel(level);
    }
    appender.installed = true;
    appender.start();
    root.addAppender(appender);
    return appender;
  }

  /**
   * Detaches and stops an appender {@link #install} attached. Appenders configured in {@code
   * logback.xml} are left alone.
   */
  public static void uninstall(TrailscopeAppender appender) {
    if (appender == null || !appender.installed) {
      return;
    }
    if (appender.getContext() instanceof LoggerContext context) {
      context.getLogger(Logger.ROOT_LOGGER_NAME).detachAppender(appender);
    }
    appender.stop();
  }
}
