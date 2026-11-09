package dev.trailscope.spring;

import dev.trailscope.Trailscope;
import dev.trailscope.TrailscopeContext;
import dev.trailscope.servlet.TrailscopeFilter;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletResponse;
import org.springframework.core.Ordered;
import org.springframework.web.servlet.HandlerExceptionResolver;
import org.springframework.web.servlet.ModelAndView;

/**
 * Notes the exception a controller threw on the request's Trailscope context, then lets Spring
 * answer as usual (it never resolves anything). The error is recorded with the request when the
 * response turns out to be a 5xx.
 */
public class TrailscopeExceptionResolver implements HandlerExceptionResolver, Ordered {
  @Override
  public ModelAndView resolveException(
      HttpServletRequest request, HttpServletResponse response, Object handler, Exception ex) {
    TrailscopeContext ctx = TrailscopeFilter.contextOf(request);
    if (ctx == null) {
      ctx = Trailscope.currentContext();
    }
    if (ctx != null) {
      ctx.recordError(ex);
    }
    return null;
  }

  @Override
  public int getOrder() {
    return Ordered.HIGHEST_PRECEDENCE;
  }
}
