package pro.trailscope;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/** Runs each task inside the context of the thread that submitted it. */
final class ContextExecutorService implements ExecutorService {
  private final ExecutorService delegate;

  ContextExecutorService(ExecutorService delegate) {
    this.delegate = delegate;
  }

  private static <T> List<Callable<T>> wrapAll(Collection<? extends Callable<T>> tasks) {
    List<Callable<T>> out = new ArrayList<>(tasks.size());
    for (Callable<T> t : tasks) {
      out.add(Trailscope.wrap(t));
    }
    return out;
  }

  @Override
  public void execute(Runnable command) {
    delegate.execute(Trailscope.wrap(command));
  }

  @Override
  public <T> Future<T> submit(Callable<T> task) {
    return delegate.submit(Trailscope.wrap(task));
  }

  @Override
  public <T> Future<T> submit(Runnable task, T result) {
    return delegate.submit(Trailscope.wrap(task), result);
  }

  @Override
  public Future<?> submit(Runnable task) {
    return delegate.submit(Trailscope.wrap(task));
  }

  @Override
  public <T> List<Future<T>> invokeAll(Collection<? extends Callable<T>> tasks)
      throws InterruptedException {
    return delegate.invokeAll(wrapAll(tasks));
  }

  @Override
  public <T> List<Future<T>> invokeAll(
      Collection<? extends Callable<T>> tasks, long timeout, TimeUnit unit)
      throws InterruptedException {
    return delegate.invokeAll(wrapAll(tasks), timeout, unit);
  }

  @Override
  public <T> T invokeAny(Collection<? extends Callable<T>> tasks)
      throws InterruptedException, ExecutionException {
    return delegate.invokeAny(wrapAll(tasks));
  }

  @Override
  public <T> T invokeAny(Collection<? extends Callable<T>> tasks, long timeout, TimeUnit unit)
      throws InterruptedException, ExecutionException, TimeoutException {
    return delegate.invokeAny(wrapAll(tasks), timeout, unit);
  }

  @Override
  public void shutdown() {
    delegate.shutdown();
  }

  @Override
  public List<Runnable> shutdownNow() {
    return delegate.shutdownNow();
  }

  @Override
  public boolean isShutdown() {
    return delegate.isShutdown();
  }

  @Override
  public boolean isTerminated() {
    return delegate.isTerminated();
  }

  @Override
  public boolean awaitTermination(long timeout, TimeUnit unit) throws InterruptedException {
    return delegate.awaitTermination(timeout, unit);
  }
}
