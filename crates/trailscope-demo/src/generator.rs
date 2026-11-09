//! Realistic traffic for a fictional, generic SaaS API: auth, users,
//! projects and tasks, search, file uploads, invoices and incoming webhooks,
//! plus the background jobs they trigger.
//!
//! Everything is deterministic for a given seed so screenshots and e2e tests
//! are stable.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{Value, json};

/// A route in the fictional API.
struct Route {
    service: &'static str,
    method: &'static str,
    route: &'static str,
    /// Relative traffic weight.
    weight: u32,
    /// Typical latency (median ms) and spread factor.
    p50_ms: f64,
}

const ROUTES: &[Route] = &[
    Route {
        service: "auth",
        method: "POST",
        route: "/auth/login",
        weight: 6,
        p50_ms: 85.0,
    },
    Route {
        service: "auth",
        method: "POST",
        route: "/auth/refresh",
        weight: 10,
        p50_ms: 18.0,
    },
    Route {
        service: "api",
        method: "GET",
        route: "/users/:id",
        weight: 18,
        p50_ms: 22.0,
    },
    Route {
        service: "api",
        method: "GET",
        route: "/projects",
        weight: 14,
        p50_ms: 35.0,
    },
    Route {
        service: "api",
        method: "GET",
        route: "/projects/:id/tasks",
        weight: 20,
        p50_ms: 48.0,
    },
    Route {
        service: "api",
        method: "POST",
        route: "/projects/:id/tasks",
        weight: 7,
        p50_ms: 64.0,
    },
    Route {
        service: "api",
        method: "GET",
        route: "/search",
        weight: 12,
        p50_ms: 120.0,
    },
    Route {
        service: "files",
        method: "POST",
        route: "/files/upload",
        weight: 3,
        p50_ms: 420.0,
    },
    Route {
        service: "billing",
        method: "GET",
        route: "/billing/invoices",
        weight: 4,
        p50_ms: 95.0,
    },
    Route {
        service: "webhooks",
        method: "POST",
        route: "/webhooks/incoming",
        weight: 6,
        p50_ms: 30.0,
    },
];

const CLIENTS: &[(&str, &str, &str)] = &[
    (
        "web",
        "4.12.0",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 Safari/605.1.15",
    ),
    (
        "web",
        "4.11.2",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141.0 Safari/537.36",
    ),
    ("ios", "3.8.1", "ExampleApp/3.8.1 (iPhone; iOS 19.1)"),
    ("android", "3.8.0", "ExampleApp/3.8.0 (Android 16; Pixel 9)"),
    ("server", "1.4.2", "example-cli/1.4.2"),
];

pub const RELEASE_BEFORE: &str = "v1.42.3";
pub const RELEASE_BAD: &str = "v1.43.0";
pub const RELEASE_FIX: &str = "v1.43.1";
/// The route that degrades during the scripted incident.
pub const INCIDENT_ROUTE: &str = "/projects/:id/tasks";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before the bad deploy.
    Normal,
    /// After the bad deploy: errors and latency spike on one route.
    Incident,
    /// After the fix.
    Recovered,
}

impl Phase {
    pub fn api_release(self) -> &'static str {
        match self {
            Phase::Normal => RELEASE_BEFORE,
            Phase::Incident => RELEASE_BAD,
            Phase::Recovered => RELEASE_FIX,
        }
    }
}

pub struct Generator {
    rng: StdRng,
    total_weight: u32,
    seq: u64,
    pub environment: String,
}

/// One simulated request and everything it caused: wire items ready to send.
pub struct Generated {
    pub items: Vec<Value>,
}

impl Generator {
    pub fn new(seed: u64, environment: &str) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            total_weight: ROUTES.iter().map(|r| r.weight).sum(),
            seq: 0,
            environment: environment.to_string(),
        }
    }

    fn pick_route(&mut self) -> &'static Route {
        let mut n = self.rng.random_range(0..self.total_weight);
        for r in ROUTES {
            if n < r.weight {
                return r;
            }
            n -= r.weight;
        }
        &ROUTES[0]
    }

    fn id(&mut self, prefix: &str, len: usize) -> String {
        const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let s: String = (0..len)
            .map(|_| ALPHABET[self.rng.random_range(0..ALPHABET.len())] as char)
            .collect();
        format!("{prefix}{s}")
    }

    fn hex(&mut self, len: usize) -> String {
        (0..len)
            .map(|_| char::from_digit(self.rng.random_range(0..16u32), 16).unwrap_or('0'))
            .collect()
    }

    /// Log-normal-ish latency around `p50` with a long tail.
    fn latency(&mut self, p50: f64) -> f64 {
        let u: f64 = self.rng.random_range(0.0001..1.0);
        let z = (-2.0 * u.ln()).sqrt()
            * (2.0 * std::f64::consts::PI * self.rng.random_range(0.0..1.0)).cos();
        (p50 * (0.45 * z).exp()).max(0.4)
    }

    /// Generates one request (with its spans, logs and jobs) at `ts_ms`.
    pub fn request(&mut self, ts_ms: i64, phase: Phase) -> Generated {
        self.seq += 1;
        let r = self.pick_route();
        let user_n = self.rng.random_range(1001..1400);
        let user_id = format!("u_{user_n}");
        let account_id = format!("acc_{}", 100 + user_n % 37);
        let project_n = 4800 + user_n % 53;
        let project_ref = format!("prj_{project_n}");
        let (client, client_version, ua) = CLIENTS[self.rng.random_range(0..CLIENTS.len())];
        let request_id = self.id("req_", 16);
        let trace_id = self.hex(32);
        let span_id = self.hex(16);
        let release = if r.service == "api" {
            phase.api_release()
        } else {
            "v2.7.0"
        };

        let mut duration = self.latency(r.p50_ms);
        let mut status: u16 = match r.method {
            "POST" if r.route != "/auth/login" => 201,
            _ => 200,
        };
        if r.route == "/webhooks/incoming" {
            status = 202;
        }
        let mut error: Option<(&str, String, String)> = None;
        let roll: f64 = self.rng.random_range(0.0..1.0);

        // Baseline error mix.
        match r.route {
            "/auth/refresh" if roll < 0.04 => {
                status = 401;
                error = Some((
                    "Unauthorized",
                    "refresh token expired".into(),
                    String::new(),
                ));
            }
            "/auth/login" if roll < 0.03 => {
                status = 429;
                error = Some((
                    "RateLimited",
                    "too many login attempts, retry in 30s".into(),
                    String::new(),
                ));
            }
            "/search" if roll < 0.015 => {
                status = 504;
                duration = 3000.0 + self.rng.random_range(0.0..40.0);
                error = Some((
                    "TimeoutError",
                    "upstream search-index timed out after 3000ms".into(),
                    stack(
                        &[
                            "at SearchClient.query (/app/src/clients/search.ts:88:15)",
                            "at withRetry (/app/src/lib/retry.ts:21:12)",
                            "at SearchController.find (/app/src/routes/search.ts:34:22)",
                            "at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)",
                        ],
                        "TimeoutError: upstream search-index timed out after 3000ms",
                    ),
                ));
            }
            "/search" if roll < 0.03 => {
                status = 429;
                error = Some((
                    "RateLimited",
                    "search quota exceeded for account".into(),
                    String::new(),
                ));
            }
            "/projects/:id/tasks" if r.method == "POST" && roll < 0.06 => {
                status = 422;
                error = Some((
                    "ValidationError",
                    "title: must not be empty; due_date: must be in the future".into(),
                    String::new(),
                ));
            }
            "/files/upload" if roll < 0.01 => {
                status = 413;
            }
            _ => {}
        }

        // The scripted incident: the bad release breaks task listing.
        if phase == Phase::Incident && r.route == INCIDENT_ROUTE && r.method == "GET" {
            duration *= 6.0 + self.rng.random_range(0.0..4.0);
            if roll < 0.28 {
                status = 500;
                error = Some((
                    "TypeError",
                    "Cannot read properties of undefined (reading 'assignee')".into(),
                    stack(
                        &[
                            "at serializeTask (/app/src/serializers/task.ts:42:31)",
                            "at Array.map (<anonymous>)",
                            "at TasksController.list (/app/src/routes/tasks.ts:58:24)",
                            "at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)",
                        ],
                        "TypeError: Cannot read properties of undefined (reading 'assignee')",
                    ),
                ));
            }
        }

        let path = r.route.replace(
            ":id",
            &if r.route.starts_with("/users") {
                user_id.clone()
            } else {
                project_ref.clone()
            },
        );
        let query = match r.route {
            "/search" => format!(
                "q={}&limit=20",
                ["roadmap", "invoice", "onboarding", "release notes", "bug"]
                    [self.rng.random_range(0..5)]
                .replace(' ', "+")
            ),
            "/projects/:id/tasks" if r.method == "GET" => "status=open&page=1".into(),
            _ => String::new(),
        };
        let slow = duration >= 1000.0;
        let capture_body = status >= 400 || slow;
        let req_body = match r.route {
            "/auth/login" => {
                json!({"email": format!("user{user_n}@example.com"), "password": "correct-horse-battery", "remember": true})
            }
            "/projects/:id/tasks" if r.method == "POST" => {
                json!({"title": if status == 422 { "" } else { "Draft quarterly roadmap" }, "due_date": "2026-01-15", "assignee_id": user_id})
            }
            "/webhooks/incoming" => {
                json!({"event": "subscription.updated", "id": self.id("evt_", 12), "data": {"account_id": account_id, "plan": "team"}})
            }
            _ => Value::Null,
        };
        let res_body = match (status, &error) {
            (s, Some((ty, msg, _))) if s >= 400 => {
                json!({"error": {"type": ty, "message": msg, "request_id": request_id}})
            }
            (200, _) if r.route == "/users/:id" => {
                json!({"id": user_id, "name": format!("User {user_n}"), "plan": "team", "account_id": account_id})
            }
            (200, _) if r.route == "/projects/:id/tasks" => {
                json!({"items": [{"id": self.id("tsk_", 8), "title": "Review pull request", "status": "open"}], "next": null})
            }
            (201, _) => json!({"id": self.id("obj_", 10), "created": true}),
            _ => Value::Null,
        };

        let region = ["eu-west-1", "us-east-1", "ap-south-1"][user_n as usize % 3];
        let plan = ["free", "team", "enterprise"][user_n as usize % 3];
        let mut items = Vec::with_capacity(6);
        let mut req = json!({
            "type": "request",
            "timestamp": ts_ms,
            "environment": self.environment,
            "service": r.service,
            "release": release,
            "requestId": request_id,
            "traceId": trace_id,
            "spanId": span_id,
            "method": r.method,
            "route": r.route,
            "path": path,
            "query": query,
            "status": status,
            "durationMs": round2(duration),
            "reqSize": if req_body.is_null() { 0 } else { req_body.to_string().len() },
            "resSize": if res_body.is_null() { 120 } else { res_body.to_string().len() },
            "userId": user_id,
            "client": client,
            "clientVersion": client_version,
            "ip": format!("198.51.100.{}", user_n % 250),
            "userAgent": ua,
            "reqHeaders": {
                "content-type": "application/json",
                "authorization": "Bearer eyJhbGciOiJIUzI1NiJ9.demo",
                "user-agent": ua,
                "x-request-id": request_id,
            },
            "resHeaders": {"content-type": "application/json", "x-request-id": request_id},
            "attributes": {"region": region, "plan": plan},
            "correlation": {"project_id": project_ref, "account_id": account_id},
        });
        if capture_body {
            if !req_body.is_null() {
                req["reqBody"] = req_body;
            }
            if !res_body.is_null() {
                req["resBody"] = res_body;
            }
        }
        if let Some((ty, msg, stk)) = &error {
            req["errorType"] = json!(ty);
            req["errorMessage"] = json!(msg);
            if !stk.is_empty() {
                req["errorStack"] = json!(stk);
            }
        }
        items.push(req);

        // Spans: handler, then a DB query (and cache / upstream for some routes).
        let start_us = ts_ms * 1000;
        let child = |items: &mut Vec<Value>,
                     g: &mut Self,
                     kind: &str,
                     name: &str,
                     offset_frac: f64,
                     frac: f64,
                     failed: bool| {
            items.push(json!({
                "type": "span",
                "traceId": trace_id,
                "spanId": g.hex(16),
                "parentSpanId": span_id,
                "requestId": request_id,
                "service": r.service,
                "kind": kind,
                "name": name,
                "start": (start_us as f64 + duration * 1000.0 * offset_frac) / 1000.0,
                "durationMs": round2(duration * frac),
                "status": if failed { "error" } else { "ok" },
            }));
        };
        child(
            &mut items,
            self,
            "cache",
            "GET session:*",
            0.02,
            0.03,
            false,
        );
        match r.route {
            "/search" => child(
                &mut items,
                self,
                "external",
                "POST search-index/query",
                0.08,
                0.86,
                status == 504,
            ),
            "/files/upload" => child(
                &mut items,
                self,
                "external",
                "PUT object-storage/uploads",
                0.1,
                0.8,
                false,
            ),
            _ => child(
                &mut items,
                self,
                "db",
                &format!("SELECT {}", table_for(r.route)),
                0.08,
                0.55,
                status == 500,
            ),
        }

        // Jobs triggered by requests share the request's IDs and correlation keys.
        let job = match (r.route, r.method, status) {
            ("/projects/:id/tasks", "POST", 201) => Some("email.send"),
            ("/billing/invoices", _, 200) if self.rng.random_range(0..4) == 0 => {
                Some("report.generate")
            }
            ("/webhooks/incoming", _, 202) => Some("webhook.deliver"),
            _ => None,
        };
        if let Some(name) = job {
            let attempts = if name == "webhook.deliver" && self.rng.random_range(0..10) == 0 {
                3
            } else {
                1
            };
            let mut t = ts_ms + duration as i64 + self.rng.random_range(30..400);
            for attempt in 1..=attempts {
                let failed = attempt < attempts;
                let jd = if failed {
                    5000.0
                } else {
                    self.latency(if name == "report.generate" {
                        1800.0
                    } else {
                        140.0
                    })
                };
                let mut ev = json!({
                    "type": "event",
                    "timestamp": t,
                    "environment": self.environment,
                    "service": "worker",
                    "release": "v2.7.0",
                    "kind": "job",
                    "name": name,
                    "level": if failed { "error" } else { "info" },
                    "status": if failed { "error" } else { "ok" },
                    "message": if failed { format!("{name} attempt {attempt} failed; retrying") } else { format!("{name} completed") },
                    "durationMs": round2(jd),
                    "requestId": request_id,
                    "traceId": trace_id,
                    "userId": user_id,
                    "attempt": attempt,
                    "attributes": {"queue": name.split('.').next().unwrap_or("default")},
                    "correlation": {"project_id": project_ref, "account_id": account_id},
                });
                if failed {
                    ev["errorType"] = json!("TimeoutError");
                    ev["errorMessage"] = json!("receiver did not respond within 5000ms");
                    ev["errorStack"] = json!(stack(
                        &[
                            "at deliver (/app/src/jobs/webhook-deliver.ts:27:11)",
                            "at Worker.process (/app/node_modules/bullmq/dist/cjs/classes/worker.js:412:20)",
                        ],
                        "TimeoutError: receiver did not respond within 5000ms"
                    ));
                }
                items.push(ev);
                t += jd as i64 + 1000 * (1 << attempt);
            }
        }

        // An occasional app log line inside the request.
        if self.rng.random_range(0..12) == 0 || slow {
            items.push(json!({
                "type": "event",
                "timestamp": ts_ms + (duration * 0.5) as i64,
                "environment": self.environment,
                "service": r.service,
                "release": release,
                "kind": "log",
                "name": if slow { "slow_request" } else { "app" },
                "level": if slow { "warn" } else { "info" },
                "message": if slow { format!("{} {} took {:.0}ms", r.method, r.route, duration) } else { format!("handled {} {}", r.method, r.route) },
                "requestId": request_id,
                "traceId": trace_id,
                "userId": user_id,
                "correlation": {"project_id": project_ref, "account_id": account_id},
            }));
        }
        Generated { items }
    }
}

fn table_for(route: &str) -> &'static str {
    match route {
        "/users/:id" => "users",
        "/projects" => "projects",
        "/projects/:id/tasks" => "tasks",
        "/billing/invoices" => "invoices",
        "/webhooks/incoming" => "webhook_events",
        r if r.starts_with("/auth") => "sessions",
        _ => "records",
    }
}

fn stack(frames: &[&str], head: &str) -> String {
    let mut s = head.to_string();
    for f in frames {
        s.push_str("\n    ");
        s.push_str(f);
    }
    s
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use trailscope_core::wire::WireItem;

    #[test]
    fn generated_items_are_valid_wire_items() {
        let mut g = Generator::new(7, "production");
        for i in 0..500 {
            let phase = if i % 3 == 0 {
                Phase::Incident
            } else {
                Phase::Normal
            };
            for item in g.request(1_800_000_000_000 + i, phase).items {
                let parsed: Result<WireItem, _> = serde_json::from_value(item.clone());
                assert!(parsed.is_ok(), "{item}");
            }
        }
    }

    #[test]
    fn incident_raises_errors_on_one_route() {
        let mut g = Generator::new(1, "production");
        let mut errors = 0;
        let mut total = 0;
        for i in 0..4000 {
            let generated = g.request(1_800_000_000_000 + i, Phase::Incident);
            let req = &generated.items[0];
            if req["route"] == INCIDENT_ROUTE && req["method"] == "GET" {
                total += 1;
                if req["status"] == 500 {
                    errors += 1;
                }
            }
        }
        let rate = f64::from(errors) / f64::from(total);
        assert!((0.15..0.4).contains(&rate), "{rate}");
    }

    #[test]
    fn is_deterministic() {
        let a = Generator::new(42, "staging")
            .request(1, Phase::Normal)
            .items;
        let b = Generator::new(42, "staging")
            .request(1, Phase::Normal)
            .items;
        assert_eq!(a, b);
    }
}
