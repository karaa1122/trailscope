package com.example.tasks;

import dev.trailscope.SpanKind;
import dev.trailscope.Trailscope;
import dev.trailscope.TrailscopeClient;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.http.HttpStatus;
import org.springframework.http.ResponseEntity;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

@RestController
class TasksController {
  private static final Logger log = LoggerFactory.getLogger("tasks");

  private final TrailscopeClient trailscope;
  private final Notifier notifier;

  TasksController(TrailscopeClient trailscope, Notifier notifier) {
    this.trailscope = trailscope;
    this.notifier = notifier;
  }

  @GetMapping("/health") // ignored by default
  String health() {
    return "ok";
  }

  @GetMapping("/projects/{projectId}/tasks")
  List<Map<String, Object>> list(@PathVariable String projectId) {
    Trailscope.setCorrelation("project_id", projectId);
    log.info("listing tasks of {}", projectId);
    return List.of(Map.of("id", "t1", "title", "Write the brief", "done", false));
  }

  @PostMapping("/projects/{projectId}/tasks")
  ResponseEntity<Map<String, Object>> create(
      @PathVariable String projectId, @RequestBody Map<String, Object> body) {
    Trailscope.setCorrelation("project_id", projectId);
    if (!(body.get("title") instanceof String title) || title.isBlank()) {
      return ResponseEntity.badRequest().body(Map.of("error", "title is required"));
    }
    Map<String, Object> task = new LinkedHashMap<>();
    task.put("id", "t" + System.currentTimeMillis());
    task.putAll(body);
    // A background job linked to this request (same request ID, user and project).
    notifier.notifyWatchers((String) task.get("id"), title.equals("flaky"));
    return ResponseEntity.status(HttpStatus.CREATED).body(task);
  }

  @GetMapping("/projects/{projectId}/report")
  Map<String, Object> report(@PathVariable String projectId) throws InterruptedException {
    Trailscope.setCorrelation("project_id", projectId);
    int rows =
        trailscope.span(
            SpanKind.DB,
            "SELECT tasks",
            () -> {
              Thread.sleep(30);
              return 42;
            });
    return Map.of("rows", rows);
  }

  @GetMapping("/projects/{projectId}/export")
  Map<String, Object> export(@PathVariable String projectId) {
    Trailscope.setCorrelation("project_id", projectId);
    throw new IllegalStateException("Cannot read the columns of an empty export");
  }
}
