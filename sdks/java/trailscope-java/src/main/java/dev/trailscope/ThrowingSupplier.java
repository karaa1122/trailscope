package dev.trailscope;

/**
 * A block that returns a value and may throw. Used by {@link TrailscopeClient#trackJob} and
 * {@link TrailscopeClient#span}; checked exceptions thrown by the block propagate unchanged.
 *
 * @param <T> the result type
 * @param <E> the exception type the block may throw
 */
@FunctionalInterface
public interface ThrowingSupplier<T, E extends Throwable> {
  T get() throws E;
}
