package dev.trailscope;

/**
 * A block without a result that may throw. Used by {@link TrailscopeClient#trackJob} and
 * {@link TrailscopeClient#span}; checked exceptions thrown by the block propagate unchanged.
 *
 * @param <E> the exception type the block may throw
 */
@FunctionalInterface
public interface ThrowingRunnable<E extends Throwable> {
  void run() throws E;
}
