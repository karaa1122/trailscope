package com.example.tasks;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.scheduling.annotation.EnableAsync;

// Minimal Spring Boot app wired to Trailscope. Start Trailscope, then:
//   TRAILSCOPE_INGEST_KEY=trs_ingest_… mvn spring-boot:run
//   curl localhost:3000/projects/p1/tasks
@SpringBootApplication
@EnableAsync
public class TasksApplication {
  public static void main(String[] args) {
    SpringApplication.run(TasksApplication.class, args);
  }
}
