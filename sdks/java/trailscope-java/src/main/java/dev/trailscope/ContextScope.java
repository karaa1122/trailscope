package dev.trailscope;

/**
 * Returned by {@link Trailscope#useContext(TrailscopeContext)}; closing it restores the context
 * that was current before.
 */
public final class ContextScope implements AutoCloseable {
  private final TrailscopeContext previous;
  private boolean closed;

  ContextScope(TrailscopeContext previous) {
    this.previous = previous;
  }

  @Override
  public void close() {
    if (!closed) {
      closed = true;
      Trailscope.restore(previous);
    }
  }
}
