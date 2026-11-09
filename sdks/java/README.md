# Trailscope Java SDK

The [Trailscope](https://github.com/karaa1122/trailscope) SDK for Java 17+: logs every request with its route, timing, user, journey keys and (on errors) bodies and stack, plus background jobs and log lines, without blocking your request path. The core has no dependencies.

## Spring Boot 3

```xml
<dependency>
  <groupId>dev.trailscope</groupId>
  <artifactId>trailscope-spring-boot-starter</artifactId>
  <version>0.1.0</version>
</dependency>
```

Set `TRAILSCOPE_ENDPOINT` and `TRAILSCOPE_INGEST_KEY` (or `trailscope.endpoint` and `trailscope.ingest-key`), and every request is recorded.

## Any Jakarta Servlet 6 container

Add `dev.trailscope:trailscope-servlet` and register `dev.trailscope.servlet.TrailscopeFilter` first, for `/*`, with `async-supported`. Logback users add `dev.trailscope:trailscope-logback`.

Documentation: [docs/sdk-java.md](https://github.com/karaa1122/trailscope/blob/main/docs/sdk-java.md).

## Building

```bash
mvn -B verify                                       # compiler warnings are errors
mvn -q install && mvn -q -P bench -pl bench exec:java   # overhead benchmark
```
