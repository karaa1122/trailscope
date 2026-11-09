package com.example.tasks;

import dev.trailscope.JobOptions;
import dev.trailscope.TrailscopeClient;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.scheduling.annotation.Async;
import org.springframework.stereotype.Service;

@Service
class Notifier {
  private static final Logger log = LoggerFactory.getLogger("tasks");
  private static final int MAX_ATTEMPTS = 3;

  private final TrailscopeClient trailscope;

  Notifier(TrailscopeClient trailscope) {
    this.trailscope = trailscope;
  }

  // Runs on Spring's task executor; the starter carries the request's context over, so every
  // attempt is linked to the request that created the task.
  @Async
  public void notifyWatchers(String taskId, boolean flaky) {
    for (int attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      int current = attempt;
      try {
        trailscope.trackJob(
            "task.notify",
            JobOptions.attempt(current).willRetry(current < MAX_ATTEMPTS),
            () -> {
              Thread.sleep(50);
              // A task titled "flaky" fails its first attempt, to show retries.
              if (flaky && current == 1) {
                throw new IllegalStateException("mail server busy");
              }
              log.info("notified watchers of {}", taskId);
            });
        return;
      } catch (InterruptedException e) {
        Thread.currentThread().interrupt();
        return;
      } catch (RuntimeException e) {
        try {
          Thread.sleep(1000L * current);
        } catch (InterruptedException ie) {
          Thread.currentThread().interrupt();
          return;
        }
      }
    }
  }
}
