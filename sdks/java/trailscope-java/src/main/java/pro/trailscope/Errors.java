package pro.trailscope;

import java.io.PrintWriter;
import java.io.StringWriter;

/** {@code errorType}, {@code errorMessage} and {@code errorStack} of an exception. */
final class Errors {
  private Errors() {}

  /** The fully qualified class name, e.g. {@code java.lang.IllegalStateException}. */
  static String type(Throwable t) {
    return t.getClass().getName();
  }

  static String message(Throwable t) {
    String m = t.getMessage();
    return m != null ? m : "";
  }

  /** The stack as {@link Throwable#printStackTrace()} prints it, causes included. */
  static String stack(Throwable t) {
    StringWriter sw = new StringWriter(1024);
    try (PrintWriter pw = new PrintWriter(sw)) {
      t.printStackTrace(pw);
    } catch (RuntimeException e) {
      return t.toString();
    }
    return sw.toString();
  }

  /**
   * The exception a servlet container or framework wrapped, e.g. the cause of a {@code
   * ServletException("Request processing failed")}.
   */
  static Throwable unwrap(Throwable t) {
    Throwable cur = t;
    for (int i = 0; i < 8 && cur.getCause() != null && cur.getCause() != cur; i++) {
      String name = cur.getClass().getName();
      if (!name.equals("jakarta.servlet.ServletException")
          && !name.equals("java.util.concurrent.CompletionException")
          && !name.equals("java.util.concurrent.ExecutionException")
          && !name.equals("java.lang.reflect.UndeclaredThrowableException")
          && !name.equals("java.lang.reflect.InvocationTargetException")) {
        break;
      }
      cur = cur.getCause();
    }
    return cur;
  }
}
