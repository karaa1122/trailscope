package dev.trailscope.spring;

import dev.trailscope.Trailscope;
import dev.trailscope.TrailscopeContext;
import dev.trailscope.servlet.TrailscopeFilter;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletResponse;
import java.security.Principal;
import org.springframework.web.servlet.HandlerInterceptor;

/**
 * Records the authenticated principal's name as the user, as Spring MVC sees it (Spring Security
 * included), unless the application already called {@code Trailscope.setUser}.
 */
public class TrailscopeUserInterceptor implements HandlerInterceptor {
  @Override
  public boolean preHandle(
      HttpServletRequest request, HttpServletResponse response, Object handler) {
    try {
      TrailscopeContext ctx = TrailscopeFilter.contextOf(request);
      if (ctx == null) {
        ctx = Trailscope.currentContext();
      }
      if (ctx != null && ctx.userId() == null) {
        Principal principal = request.getUserPrincipal();
        if (principal != null) {
          ctx.setUser(principal.getName());
        }
      }
    } catch (RuntimeException e) {
      // Never fail the request because of Trailscope.
    }
    return true;
  }
}
