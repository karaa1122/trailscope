#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//! End-to-end API tests against real Postgres and ClickHouse (skipped when
//! `TRAILSCOPE_TEST_*` is unset).

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use futures::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use trailscope_api::AppState;
use trailscope_core::ch::{self, ClickHouse};
use trailscope_core::live::LiveBus;
use trailscope_core::rows::{LiveRequest, RequestRow, SpanRow};
use trailscope_core::{pg, testing};

struct App {
    router: Router,
    bus: LiveBus,
    ch: ClickHouse,
}

#[derive(Clone, Default)]
struct Client {
    cookie: Option<String>,
    csrf: Option<String>,
    bearer: Option<String>,
}

impl App {
    async fn call(
        &self,
        c: &Client,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, Option<String>) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(cookie) = &c.cookie {
            req = req.header(header::COOKIE, cookie);
        }
        if let Some(csrf) = &c.csrf {
            req = req.header("x-csrf-token", csrf);
        }
        if let Some(b) = &c.bearer {
            req = req.header(header::AUTHORIZATION, format!("Bearer {b}"));
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let set_cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .map(str::to_string);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json, set_cookie)
    }

    async fn get(&self, c: &Client, uri: &str) -> (StatusCode, Value) {
        let (s, v, _) = self.call(c, Method::GET, uri, None).await;
        (s, v)
    }

    async fn post(&self, c: &Client, uri: &str, body: Value) -> (StatusCode, Value) {
        let (s, v, _) = self.call(c, Method::POST, uri, Some(body)).await;
        (s, v)
    }
}

/// The management API's token in tests.
const ADMIN_TOKEN: &str = "test-admin-token-0123456789abcdef0123";

async fn app() -> Option<App> {
    let env = testing::env().await?;
    let mut cfg = env.config;
    cfg.admin_token = ADMIN_TOKEN.to_string();
    ch::migrate(&cfg).await.unwrap();
    let pool = pg::connect(&cfg.postgres).await.unwrap();
    pg::migrate(&pool).await.unwrap();
    let chc = ClickHouse::new(&cfg);
    let bus = LiveBus::local(&cfg.live);
    let state = AppState::new(
        cfg,
        chc.clone(),
        pool,
        bus.clone(),
        None,
        trailscope_alerts::Keys::from_secret("test", true),
        CancellationToken::new(),
    )
    .unwrap();
    Some(App {
        router: trailscope_api::router(state),
        bus,
        ch: chc,
    })
}

fn now_ms() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

async fn seed(app: &App, project: &str) {
    let now = now_ms();
    let mut ins = app
        .ch
        .writer
        .insert::<RequestRow>("requests")
        .await
        .unwrap();
    for i in 0..60u32 {
        ins.write(&RequestRow {
            project_id: project.into(),
            environment: if i % 10 == 0 {
                "staging".into()
            } else {
                "production".into()
            },
            service: if i % 2 == 0 {
                "api".into()
            } else {
                "auth".into()
            },
            timestamp: now - 1000 * i64::from(i) - 5_000,
            request_id: format!("req{i:03}"),
            trace_id: format!("trace{i:03}"),
            method: "POST".into(),
            route: "/projects/:id/tasks".into(),
            path: format!("/projects/prj_{i}/tasks"),
            status: if i % 5 == 0 { 500 } else { 200 },
            duration_ms: i as f32 * 10.0,
            user_id: format!("u_{}", i % 3),
            client: "web".into(),
            req_headers: vec![
                ("authorization".into(), "[REDACTED]".into()),
                ("content-type".into(), "application/json".into()),
                ("x-tenant".into(), "t1".into()),
            ],
            req_body: r#"{"title":"hello","password":"[REDACTED]"}"#.into(),
            res_body: r#"{"ok":false}"#.into(),
            error_type: if i % 5 == 0 {
                "TimeoutError".into()
            } else {
                String::new()
            },
            error_message: if i % 5 == 0 {
                "upstream timed out".into()
            } else {
                String::new()
            },
            error_fingerprint: if i % 5 == 0 { 42 } else { 0 },
            attributes: vec![("tenant".into(), format!("t{}", i % 2))],
            correlation: vec![("account_id".into(), "acc_1".into())],
            sample_rate: 1.0,
            ..Default::default()
        })
        .await
        .unwrap();
    }
    ins.end().await.unwrap();
    let mut spans = app.ch.writer.insert::<SpanRow>("spans").await.unwrap();
    spans
        .write(&SpanRow {
            project_id: project.into(),
            trace_id: "trace000".into(),
            span_id: "s1".into(),
            request_id: "req000".into(),
            service: "api".into(),
            kind: "db".into(),
            name: "SELECT tasks".into(),
            start: (now - 5_000) * 1000,
            duration_ms: 3.0,
            status: "ok".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    spans.end().await.unwrap();
}

#[tokio::test]
async fn full_api_flow() {
    let Some(app) = app().await else {
        eprintln!("skipping: TRAILSCOPE_TEST_* not set");
        return;
    };
    let anon = Client::default();

    // ---- setup ----
    let (s, v) = app.get(&anon, "/api/v1/setup").await;
    assert_eq!((s, v["needsSetup"].clone()), (StatusCode::OK, json!(true)));
    let (s, _) = app.get(&anon, "/api/v1/auth/me").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v, cookie) = app
        .call(&anon, Method::POST, "/api/v1/setup", Some(json!({
            "email": "Owner@Example.com", "name": "Owner", "password": "correct horse battery", "projectName": "Main API"
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["ingestKey"].as_str().unwrap().starts_with("trs_ingest_"));
    let project = v["projectId"].as_str().unwrap().to_string();
    let owner = Client {
        cookie,
        csrf: v["csrfToken"].as_str().map(str::to_string),
        bearer: None,
    };
    let (s, _) = app
        .post(
            &anon,
            "/api/v1/setup",
            json!({"email": "x@example.com", "password": "another long one", "projectName": "x"}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // ---- auth & CSRF ----
    let (s, v) = app.get(&owner, "/api/v1/auth/me").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["user"]["email"], "owner@example.com");
    let no_csrf = Client {
        csrf: None,
        ..owner.clone()
    };
    let (s, v) = app
        .post(&no_csrf, "/api/v1/projects", json!({"name": "Other"}))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, v) = app
        .post(&owner, "/api/v1/projects", json!({"name": "Other"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    seed(&app, &project).await;
    let range = json!({"project": project, "from": now_ms() - 3_600_000, "to": now_ms()});
    let with = |extra: Value| {
        let mut r = range.clone();
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };

    // ---- query ----
    let (s, v) = app
        .post(&owner, "/api/v1/query/validate", json!({"query": {"mode": "sql", "text": "status >= 500 AND attr['tenant'] = 't0'"}, "project": project}))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["conditions"], 2);
    assert_eq!(v["search"], "status:>=500 attr.tenant:t0");
    assert_eq!(v["chips"].as_array().unwrap().len(), 2);
    let (_, v) = app
        .post(
            &owner,
            "/api/v1/query/validate",
            json!({"query": {"mode": "sql", "text": "stauts = 1"}}),
        )
        .await;
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["col"], 1);

    let (s, v) = app
        .post(
            &owner,
            "/api/v1/query/requests",
            with(json!({"query": {"mode": "sql", "text": "status >= 500"}, "limit": 5})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rows = v["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5, "{v}");
    assert!(rows.iter().all(|r| r["status"] == 500));
    assert!(rows[0]["timestamp"].as_i64().unwrap() >= rows[1]["timestamp"].as_i64().unwrap());
    // Page through everything with the cursor.
    let mut seen = 0;
    let mut cursor: Option<String> = None;
    loop {
        let mut body =
            with(json!({"limit": 7, "query": {"mode": "search", "text": "env:production"}}));
        if let Some(c) = &cursor {
            body["cursor"] = json!(c);
        }
        let (_, v) = app.post(&owner, "/api/v1/query/requests", body).await;
        seen += v["rows"].as_array().unwrap().len();
        match v["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(seen, 54);
    let (s, v) = app
        .post(
            &owner,
            "/api/v1/query/requests",
            with(json!({"query": {"mode": "sql", "text": "1=1; DROP TABLE requests"}})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v["error"]["code"], "invalid_query");

    let (s, v) = app
        .post(
            &owner,
            "/api/v1/query/histogram",
            with(json!({"buckets": 60})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 60.0);
    assert_eq!(v["series"], json!(["2xx", "3xx", "4xx", "5xx"]));

    let (s, v) = app
        .post(&owner, "/api/v1/query/facets", with(json!({"fields": [{"column": "service"}, {"column": "attributes", "key": "tenant"}]})))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["facets"][0]["values"][0]["count"], 30);
    assert_eq!(v["facets"][1]["field"], "attributes.tenant");

    let (s, v) = app
        .post(
            &owner,
            "/api/v1/query/group",
            with(json!({"by": {"column": "service"}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let api = v["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["value"] == "api")
        .unwrap()
        .clone();
    assert_eq!(api["count"], 30.0);
    assert_eq!(api["errorRate"], 0.2);

    let (s, v) = app.get(&owner, "/api/v1/schema").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        v["tables"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "status")
    );
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/values?project={project}&column=user_id&prefix=u_"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["values"].as_array().unwrap().len(), 3);
    let (_, v) = app
        .get(
            &owner,
            &format!("/api/v1/keys?project={project}&map=attributes"),
        )
        .await;
    assert_eq!(v["values"][0]["value"], "tenant");

    // ---- inspector ----
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/requests/req000?project={project}"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["request"]["route"], "/projects/:id/tasks");
    assert_eq!(v["spans"][0]["name"], "SELECT tasks");
    // Scoped to the request's environment (staging): 6 of the 12 errors.
    assert_eq!(v["errorGroup"]["count"].to_string().trim_matches('"'), "6");
    assert_eq!(v["bodiesHidden"], false);
    assert!(v["request"]["req_body"].as_str().unwrap().contains("hello"));
    let (_, v) = app
        .get(
            &owner,
            &format!(
                "/api/v1/requests/req000/curl?project={project}&target=https://staging.example.com"
            ),
        )
        .await;
    let cmd = v["command"].as_str().unwrap();
    assert!(
        cmd.contains("'https://staging.example.com/projects/prj_0/tasks'"),
        "{cmd}"
    );
    assert!(cmd.contains("authorization: <authorization>"));

    // ---- replay against a local echo server ----
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo.local_addr().unwrap();
    tokio::spawn(async move {
        let app = Router::new().fallback(|req: Request<Body>| async move {
            let auth = req
                .headers()
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_string());
            let tenant = req
                .headers()
                .get("x-tenant")
                .map(|v| v.to_str().unwrap().to_string());
            let body = req.into_body().collect().await.unwrap().to_bytes();
            axum::Json(
                json!({"auth": auth, "tenant": tenant, "body": String::from_utf8_lossy(&body)}),
            )
        });
        axum::serve(echo, app).await.unwrap();
    });
    let (s, v) = app
        .post(
            &owner,
            &format!("/api/v1/projects/{project}/replay-targets"),
            json!({"name": "local", "baseUrl": format!("http://{echo_addr}")}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let target = v["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &owner,
            "/api/v1/requests/req000/replay",
            json!({"project": project, "targetId": target}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], 200);
    let echoed: Value = serde_json::from_str(v["body"].as_str().unwrap()).unwrap();
    assert_eq!(
        echoed["auth"],
        Value::Null,
        "auth headers must not be replayed"
    );
    assert_eq!(echoed["tenant"], "t1");

    // ---- members, roles and the bodies permission ----
    let (s, v) = app
        .post(
            &owner,
            "/api/v1/invites",
            json!({"email": "viewer@example.com", "role": "viewer"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let (s, v, cookie) = app
        .call(
            &anon,
            Method::POST,
            "/api/v1/invites/accept",
            Some(json!({"token": token, "password": "viewer password 1"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let viewer = Client {
        cookie,
        csrf: v["csrfToken"].as_str().map(str::to_string),
        bearer: None,
    };
    let (s, _) = app
        .post(&viewer, "/api/v1/projects", json!({"name": "Nope"}))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, v) = app
        .get(
            &viewer,
            &format!("/api/v1/requests/req000?project={project}"),
        )
        .await;
    assert_eq!(v["bodiesHidden"], true);
    assert!(v["request"].get("req_body").is_none());
    assert!(v["request"].get("req_headers").is_none());
    let (s, v) = app
        .post(
            &viewer,
            "/api/v1/query/requests",
            with(json!({"query": {"mode": "sql", "text": "req_body LIKE '%hello%'"}})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, _) = app
        .post(
            &viewer,
            "/api/v1/query/requests",
            with(json!({"columns": ["timestamp", "res_body"]})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app
        .post(
            &viewer,
            "/api/v1/requests/req000/replay",
            json!({"project": project, "targetId": target}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // ---- API tokens and scopes ----
    let (s, v) = app
        .post(
            &owner,
            "/api/v1/tokens",
            json!({"name": "ci", "scopes": ["read"]}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let bearer = Client {
        bearer: v["token"].as_str().map(str::to_string),
        ..Client::default()
    };
    let (s, _) = app
        .post(&bearer, "/api/v1/query/histogram", range.clone())
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .post(&bearer, "/api/v1/projects", json!({"name": "via token"}))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // ---- saved views ----
    let (s, v) = app
        .post(&owner, "/api/v1/views", json!({"project": project, "name": "Errors", "query": {"mode": "search", "text": "status:5xx"}, "shared": true, "timeRange": {"relative": "1h"}}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v) = app
        .get(&viewer, &format!("/api/v1/views?project={project}"))
        .await;
    assert_eq!(v[0]["name"], "Errors");
    assert_eq!(v[0]["query"]["text"], "status:5xx");
    let (s, _) = app
        .post(&owner, "/api/v1/views", json!({"project": project, "name": "Bad", "query": {"mode": "sql", "text": "nope = 1"}}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // ---- projects metadata ----
    let (s, v) = app
        .get(&owner, &format!("/api/v1/projects/{project}/services"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .get(&owner, &format!("/api/v1/projects/{project}/environments"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!(["production", "staging"]));
    let (s, v) = app.get(&owner, "/api/openapi.json").await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["paths"]["/api/v1/query/requests"].is_object());

    // ---- overview, heatmap and errors (from the rollups) ----
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/overview?project={project}&minutes=30"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["times"].as_array().unwrap().len(), 30);
    assert!(v["requestsPerMin"]["value"].as_f64().unwrap() > 0.0, "{v}");
    assert!(v["errorRate"]["value"].as_f64().unwrap() > 0.0, "{v}");
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/routes/heatmap?project={project}&minutes=30"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["routes"][0]["route"], "/projects/:id/tasks");
    assert_eq!(v["routes"][0]["cells"].as_array().unwrap().len(), 30);
    let (s, v) = app
        .get(&owner, &format!("/api/v1/errors?project={project}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let groups = v["groups"].as_array().unwrap();
    assert!(
        groups.iter().any(|g| g["fingerprint"] == "42"
            && g["errorType"] == "TimeoutError"
            && g["isNew"] == true),
        "{v}"
    );
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/errors/42?project={project}&environment=production"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["samples"].as_array().unwrap().len(), 6);
    assert_eq!(v["group"]["last24h"], 6);

    // ---- login and rate limiting ----
    let (s, _) = app
        .post(
            &anon,
            "/api/v1/auth/login",
            json!({"email": "owner@example.com", "password": "correct horse battery"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    for _ in 0..8 {
        let (s, _) = app
            .post(
                &anon,
                "/api/v1/auth/login",
                json!({"email": "owner@example.com", "password": "wrong"}),
            )
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, _) = app
        .post(
            &anon,
            "/api/v1/auth/login",
            json!({"email": "owner@example.com", "password": "correct horse battery"}),
        )
        .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);

    // ---- live tail ----
    let mut req = Request::get(format!(
        "/api/v1/live?project={project}&filter=status%20%3E%3D%20500"
    ))
    .header(header::COOKIE, owner.cookie.clone().unwrap())
    .body(Body::empty())
    .unwrap();
    req.headers_mut()
        .insert(header::ACCEPT, "text/event-stream".parse().unwrap());
    let resp = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body().into_data_stream();
    let first = String::from_utf8(body.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.contains("event: ready"), "{first}");
    let live = |status: u16, project: &str| LiveRequest {
        project_id: project.into(),
        status,
        route: format!("/s{status}"),
        ..Default::default()
    };
    app.bus.publish_requests(vec![
        live(200, &project),
        live(503, "other"),
        live(502, &project),
    ]);
    let chunk = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(chunk.to_vec()).unwrap();
    assert!(
        text.contains("event: request") && text.contains("/s502"),
        "{text}"
    );
    assert!(!text.contains("/s200") && !text.contains("/s503"));
    let (s, v) = app
        .get(
            &owner,
            &format!("/api/v1/live?project={project}&filter=req_body%20%3D%20%27x%27"),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

/// Organizations (decision 070): a member of one organization can't read,
/// list, change or query anything of another, route by route. The route
/// table is checked against the OpenAPI document, so a new project-scoped
/// route fails this test until it is added here.
#[tokio::test]
async fn cross_tenant_isolation() {
    let Some(app) = app().await else {
        eprintln!("skipping: TRAILSCOPE_TEST_* not set");
        return;
    };
    let anon = Client::default();
    let session = |cookie: Option<String>, v: &Value| Client {
        cookie,
        csrf: v["csrfToken"].as_str().map(str::to_string),
        bearer: None,
    };

    // Alpha: set up by A, who is an instance admin.
    let (s, v, cookie) = app
        .call(&anon, Method::POST, "/api/v1/setup", Some(json!({
            "email": "a@example.com", "password": "correct horse battery", "projectName": "Alpha API",
            "organizationName": "Alpha"
        })))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let a = session(cookie, &v);
    let alpha = v["org"]["id"].as_str().unwrap().to_string();
    let pa = v["projectId"].as_str().unwrap().to_string();
    assert_eq!(v["org"]["name"], "Alpha");

    // Beta: created by A, with its own project and an admin B who belongs only to Beta.
    let (s, v) = app.post(&a, "/api/v1/orgs", json!({"name": "Beta"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let beta = v["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(&a, "/api/v1/projects", json!({"name": "Beta API"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pb = v["project"]["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &a,
            "/api/v1/invites",
            json!({"email": "b@example.com", "role": "admin", "canViewBodies": true}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = app
        .get(
            &anon,
            &format!(
                "/api/v1/invites/lookup?token={}",
                v["token"].as_str().unwrap()
            ),
        )
        .await;
    assert_eq!(
        (s, v["organization"].clone(), v["existingAccount"].clone()),
        (StatusCode::OK, json!("Beta"), json!(false))
    );
    let (_, v) = app
        .post(
            &a,
            "/api/v1/invites",
            json!({"email": "b2@example.com", "role": "viewer"}),
        )
        .await;
    let beta_invite_token = v["token"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &a,
            "/api/v1/invites",
            json!({"email": "b@example.com", "role": "admin", "canViewBodies": true}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, cookie) = app
        .call(
            &anon,
            Method::POST,
            "/api/v1/invites/accept",
            Some(json!({"token": v["token"], "password": "b password long"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let b = session(cookie, &v);
    assert_eq!(v["org"]["id"], json!(beta));
    assert!(!beta_invite_token.is_empty());

    // Back in Alpha, A invites C, who belongs only to Alpha.
    let (s, _) = app
        .post(&a, &format!("/api/v1/orgs/{alpha}/switch"), json!({}))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = app
        .post(
            &a,
            "/api/v1/invites",
            json!({"email": "c@example.com", "role": "member"}),
        )
        .await;
    let (s, v, _) = app
        .call(
            &anon,
            Method::POST,
            "/api/v1/invites/accept",
            Some(json!({"token": v["token"], "password": "c password long"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let c_id = v["user"]["id"].as_str().unwrap().to_string();

    // Things in Alpha for B to go after.
    seed(&app, &pa).await;
    let (_, v) = app.get(&a, &format!("/api/v1/projects/{pa}/keys")).await;
    let key_a = v[0]["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &a,
            &format!("/api/v1/projects/{pa}/redaction-rules"),
            json!({"kind": "body_key", "pattern": "ssn"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let redaction_a = v["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &a,
            &format!("/api/v1/projects/{pa}/replay-targets"),
            json!({"name": "staging", "baseUrl": "http://127.0.0.1:9"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let target_a = v["id"].as_str().unwrap().to_string();
    let rule = |p: &str| {
        json!({
            "project": p, "name": "5xx", "kind": "count",
            "query": {"table": "requests", "input": {"mode": "sql", "text": "status >= 500"}},
            "threshold": 10, "windowMinutes": 5
        })
    };
    let (s, v) = app.post(&a, "/api/v1/alerts/rules", rule(&pa)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let rule_a = v["id"].as_str().unwrap().to_string();
    let channel = |p: &str| json!({"project": p, "name": "hook", "kind": "webhook", "url": "http://127.0.0.1:9/hook"});
    let (s, v) = app.post(&a, "/api/v1/alerts/channels", channel(&pa)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let channel_a = v["channel"]["id"].as_str().unwrap().to_string();
    let view = |p: &str| json!({"project": p, "name": "Errors", "query": {"mode": "search", "text": "status:5xx"}, "shared": true, "timeRange": {"relative": "1h"}});
    let (s, v) = app.post(&a, "/api/v1/views", view(&pa)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let view_a = v["id"].as_str().unwrap().to_string();
    let (s, v) = app
        .post(
            &a,
            "/api/v1/tokens",
            json!({"name": "alpha ci", "scopes": ["read"]}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token_a_id = v["id"].as_str().unwrap().to_string();
    let a_token = Client {
        bearer: v["token"].as_str().map(str::to_string),
        ..Client::default()
    };

    let (from, to) = (now_ms() - 3_600_000, now_ms());
    let q = |p: &str, extra: Value| {
        let mut body = json!({"project": p, "from": from, "to": to, "query": {"mode": "sql", "text": "status >= 500"}});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    let email_rule = |p: &str| json!({"project": p, "enabled": false, "filter": "status >= 500", "memberIds": [], "emails": []});

    // (method, OpenAPI path, URI aimed at Alpha, body). Every project-scoped route is listed.
    let foreign: Vec<(Method, &str, String, Option<Value>)> = vec![
        (
            Method::POST,
            "/api/v1/query/validate",
            "/api/v1/query/validate".into(),
            Some(json!({"project": pa, "query": {"mode": "sql", "text": "status = 1"}})),
        ),
        (
            Method::POST,
            "/api/v1/query/requests",
            "/api/v1/query/requests".into(),
            Some(q(&pa, json!({"limit": 5}))),
        ),
        (
            Method::POST,
            "/api/v1/query/histogram",
            "/api/v1/query/histogram".into(),
            Some(q(&pa, json!({"buckets": 10}))),
        ),
        (
            Method::POST,
            "/api/v1/query/facets",
            "/api/v1/query/facets".into(),
            Some(q(&pa, json!({"fields": [{"column": "service"}]}))),
        ),
        (
            Method::POST,
            "/api/v1/query/group",
            "/api/v1/query/group".into(),
            Some(q(&pa, json!({"by": {"column": "service"}}))),
        ),
        (
            Method::GET,
            "/api/v1/values",
            format!("/api/v1/values?project={pa}&table=requests&column=service"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/keys",
            format!("/api/v1/keys?project={pa}&table=requests&map=attributes"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/requests/{request_id}",
            format!("/api/v1/requests/req000?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/requests/{request_id}/curl",
            format!("/api/v1/requests/req000/curl?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/requests/{request_id}/replay",
            "/api/v1/requests/req000/replay".into(),
            Some(json!({"project": pa, "targetId": target_a})),
        ),
        (
            Method::GET,
            "/api/v1/live",
            format!("/api/v1/live?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/views",
            format!("/api/v1/views?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/views",
            "/api/v1/views".into(),
            Some(view(&pa)),
        ),
        (
            Method::PUT,
            "/api/v1/views/{id}",
            format!("/api/v1/views/{view_a}"),
            Some(view(&pa)),
        ),
        (
            Method::DELETE,
            "/api/v1/views/{id}",
            format!("/api/v1/views/{view_a}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/overview",
            format!("/api/v1/overview?project={pa}&minutes=30"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/routes/heatmap",
            format!("/api/v1/routes/heatmap?project={pa}&minutes=30"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/routes/series",
            format!("/api/v1/routes/series?project={pa}&route=/x&minutes=30"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/errors",
            format!("/api/v1/errors?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/errors/{fingerprint}",
            format!("/api/v1/errors/1?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/journeys",
            format!("/api/v1/journeys?project={pa}&key=user_id&value=u1"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/alerts/rules",
            format!("/api/v1/alerts/rules?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/alerts/rules",
            "/api/v1/alerts/rules".into(),
            Some(rule(&pa)),
        ),
        (
            Method::PUT,
            "/api/v1/alerts/rules/{id}",
            format!("/api/v1/alerts/rules/{rule_a}"),
            Some(rule(&pa)),
        ),
        (
            Method::DELETE,
            "/api/v1/alerts/rules/{id}",
            format!("/api/v1/alerts/rules/{rule_a}?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/alerts/rules/preview",
            "/api/v1/alerts/rules/preview".into(),
            Some(rule(&pa)),
        ),
        (
            Method::GET,
            "/api/v1/alerts/channels",
            format!("/api/v1/alerts/channels?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/alerts/channels",
            "/api/v1/alerts/channels".into(),
            Some(channel(&pa)),
        ),
        (
            Method::DELETE,
            "/api/v1/alerts/channels/{id}",
            format!("/api/v1/alerts/channels/{channel_a}?project={pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/alerts/channels/{id}/test",
            format!("/api/v1/alerts/channels/{channel_a}/test?project={pa}"),
            Some(json!({})),
        ),
        (
            Method::GET,
            "/api/v1/alerts/email-rule",
            format!("/api/v1/alerts/email-rule?project={pa}"),
            None,
        ),
        (
            Method::PUT,
            "/api/v1/alerts/email-rule",
            "/api/v1/alerts/email-rule".into(),
            Some(email_rule(&pa)),
        ),
        (
            Method::POST,
            "/api/v1/alerts/email-rule/test",
            format!("/api/v1/alerts/email-rule/test?project={pa}"),
            Some(json!({})),
        ),
        (
            Method::GET,
            "/api/v1/alerts/history",
            format!("/api/v1/alerts/history?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/alerts/mutes",
            format!("/api/v1/alerts/mutes?project={pa}"),
            None,
        ),
        (
            Method::DELETE,
            "/api/v1/alerts/mutes/{fingerprint}",
            format!("/api/v1/alerts/mutes/1?project={pa}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}",
            format!("/api/v1/projects/{pa}"),
            None,
        ),
        (
            Method::PATCH,
            "/api/v1/projects/{id}",
            format!("/api/v1/projects/{pa}"),
            Some(json!({"name": "taken"})),
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/environments",
            format!("/api/v1/projects/{pa}/environments"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/services",
            format!("/api/v1/projects/{pa}/services"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/releases",
            format!("/api/v1/projects/{pa}/releases"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/keys",
            format!("/api/v1/projects/{pa}/keys"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/projects/{id}/keys",
            format!("/api/v1/projects/{pa}/keys"),
            Some(json!({"name": "stolen"})),
        ),
        (
            Method::DELETE,
            "/api/v1/projects/{id}/keys/{key_id}",
            format!("/api/v1/projects/{pa}/keys/{key_a}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/redaction-rules",
            format!("/api/v1/projects/{pa}/redaction-rules"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/projects/{id}/redaction-rules",
            format!("/api/v1/projects/{pa}/redaction-rules"),
            Some(json!({"kind": "body_key", "pattern": "x"})),
        ),
        (
            Method::DELETE,
            "/api/v1/projects/{id}/redaction-rules/{rule_id}",
            format!("/api/v1/projects/{pa}/redaction-rules/{redaction_a}"),
            None,
        ),
        (
            Method::GET,
            "/api/v1/projects/{id}/replay-targets",
            format!("/api/v1/projects/{pa}/replay-targets"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/projects/{id}/replay-targets",
            format!("/api/v1/projects/{pa}/replay-targets"),
            Some(json!({"name": "x", "baseUrl": "http://127.0.0.1:9"})),
        ),
        (
            Method::DELETE,
            "/api/v1/projects/{id}/replay-targets/{target_id}",
            format!("/api/v1/projects/{pa}/replay-targets/{target_a}"),
            None,
        ),
        (
            Method::DELETE,
            "/api/v1/projects/{id}",
            format!("/api/v1/projects/{pa}"),
            None,
        ),
        (
            Method::POST,
            "/api/v1/tokens",
            "/api/v1/tokens".into(),
            Some(json!({"name": "x", "scopes": ["read"], "projectId": pa})),
        ),
    ];

    // Coverage: every operation with a `project` parameter, a request body
    // with a `project` field, or a /projects/{id} path is in the table.
    let (_, doc) = app.get(&a, "/api/openapi.json").await;
    let listed: std::collections::HashSet<(String, String)> = foreign
        .iter()
        .map(|(m, p, _, _)| (m.as_str().to_ascii_lowercase(), (*p).to_string()))
        .collect();
    let mut missing = Vec::new();
    for (path, ops) in doc["paths"].as_object().unwrap() {
        for (method, op) in ops.as_object().unwrap() {
            let param = op["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["name"] == "project");
            let body = op["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .and_then(|r| r.rsplit('/').next())
                .is_some_and(|name| {
                    doc["components"]["schemas"][name]["properties"]["project"].is_object()
                });
            if (param || body || path.starts_with("/api/v1/projects/{id}"))
                && !listed.contains(&(method.clone(), path.clone()))
            {
                missing.push(format!("{} {path}", method.to_uppercase()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "project-scoped routes missing from the isolation table: {missing:?}"
    );

    // The management API (decisions 071, 072) answers only the admin token:
    // no session (not even an instance admin's), no API token, no wrong
    // token. Bodies are valid, so a refusal can't hide behind a 422.
    let nil = "00000000-0000-0000-0000-000000000000";
    let admin_body = |method: &str, path: &str| -> Option<Value> {
        match (method, path) {
            ("get", _) => None,
            ("post", "/api/v1/admin/orgs") => Some(json!({"name": "Gamma"})),
            ("post", "/api/v1/admin/orgs/{id}/invites") => {
                Some(json!({"email": "x@example.com", "role": "viewer"}))
            }
            ("post", "/api/v1/admin/plans") => Some(json!({"id": "x", "name": "X"})),
            ("put", "/api/v1/admin/plans/{id}") => Some(json!({"name": "X"})),
            _ => Some(json!({})),
        }
    };
    let wrong = Client {
        bearer: Some("w".repeat(48)),
        ..Client::default()
    };
    let mut admin_ops = 0;
    for (path, ops) in doc["paths"].as_object().unwrap() {
        if !path.starts_with("/api/v1/admin/") {
            continue;
        }
        for method in ops.as_object().unwrap().keys() {
            let m = Method::from_bytes(method.to_uppercase().as_bytes()).unwrap();
            let uri = path.replace("{id}", nil);
            for (who, c) in [
                ("Beta's admin", &b),
                ("an instance admin's session", &a),
                ("an instance admin's API token", &a_token),
                ("a wrong admin token", &wrong),
            ] {
                let (s, v, _) = app.call(c, m.clone(), &uri, admin_body(method, path)).await;
                assert!(
                    s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN,
                    "{method} {path} as {who} answered {s}: {v}"
                );
            }
            admin_ops += 1;
        }
    }
    assert!(admin_ops >= 8, "only {admin_ops} admin operations found");
    // With the admin token: every organization, and one made for a customer
    // with an owner invite, without making anyone else a member.
    let operator = Client {
        bearer: Some(ADMIN_TOKEN.to_string()),
        ..Client::default()
    };
    let (s, v) = app.get(&operator, "/api/v1/admin/orgs").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["name"].as_str())
        .collect();
    assert!(
        names.contains(&"Alpha") && names.contains(&"Beta"),
        "{names:?}"
    );
    let (s, v) = app
        .post(
            &operator,
            "/api/v1/admin/orgs",
            json!({"name": "Gamma", "planId": "team", "ownerEmail": "owner@gamma.example"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["org"]["planId"], "team");
    assert_eq!(v["org"]["members"], 0);
    assert!(v["invite"]["url"].as_str().unwrap().contains("/invite/"));
    let (_, mine) = app.get(&a, "/api/v1/orgs").await;
    assert!(
        !mine
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["name"] == "Gamma"),
        "{mine}"
    );

    // B (Beta admin) gets nothing of Alpha: 403 or 404, never success or a
    // validation error that would hide a missing check.
    for (method, path, uri, body) in &foreign {
        let (s, v, _) = app.call(&b, method.clone(), uri, body.clone()).await;
        assert!(
            s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN,
            "{method} {path} as Beta's admin on Alpha's project answered {s}: {v}"
        );
    }
    // The same reads on B's own project work, so the requests above are valid.
    for (method, path, uri, _) in foreign.iter().filter(|(m, p, _, _)| {
        *m == Method::GET
            && !p.contains("{request_id}")
            && !p.ends_with("/live")
            && !p.contains("{fingerprint}")
    }) {
        let own = uri.replace(&pa, &pb);
        let (s, v, _) = app.call(&b, method.clone(), &own, None).await;
        assert_eq!(
            s,
            StatusCode::OK,
            "{method} {path} on Beta's own project: {v}"
        );
    }
    // Alpha's rows don't leak into Beta's queries either.
    let (s, v) = app
        .post(&b, "/api/v1/query/requests", q(&pb, json!({"limit": 5})))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["rows"].as_array().unwrap().len(), 0, "{v}");

    // Organization-level data.
    let (_, v) = app.get(&b, "/api/v1/projects").await;
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(ids, vec![pb.as_str()]);
    let (_, v) = app.get(&b, "/api/v1/members").await;
    let emails: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["email"].as_str())
        .collect();
    assert!(!emails.contains(&"c@example.com"), "{emails:?}");
    assert!(emails.contains(&"b@example.com"), "{emails:?}");
    let (s, _, _) = app
        .call(
            &b,
            Method::PATCH,
            &format!("/api/v1/members/{c_id}"),
            Some(json!({"role": "viewer"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = app
        .call(&b, Method::DELETE, &format!("/api/v1/members/{c_id}"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = app.get(&b, "/api/v1/invites").await;
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .all(|i| i["email"] != "c@example.com"),
        "{v}"
    );
    let (s, _, _) = app
        .call(
            &b,
            Method::DELETE,
            &format!("/api/v1/tokens/{token_a_id}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = app
        .post(
            &b,
            "/api/v1/alerts/channels",
            json!({"project": pb, "name": "mail", "kind": "email", "memberIds": [c_id]}),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an Alpha member as a Beta recipient: {v}"
    );
    let mut with_c = email_rule(&pb);
    with_c["memberIds"] = json!([c_id]);
    let (s, _, _) = app
        .call(&b, Method::PUT, "/api/v1/alerts/email-rule", Some(with_c))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = app
        .post(&b, &format!("/api/v1/orgs/{alpha}/switch"), json!({}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = app.get(&b, "/api/v1/orgs").await;
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");

    // Server-wide settings and new organizations are for instance admins.
    let (s, _) = app.get(&b, "/api/v1/settings/smtp").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = app.post(&b, "/api/v1/orgs", json!({"name": "Gamma"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A belongs to both, but a session works in one at a time.
    let (_, v) = app.get(&a, "/api/v1/projects").await;
    assert!(
        v.as_array().unwrap().iter().all(|p| p["id"] != json!(pb)),
        "{v}"
    );
    let (s, _) = app
        .get(&a, &format!("/api/v1/overview?project={pb}&minutes=30"))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // A token created in Alpha stays in Alpha.
    let (s, _) = app.get(&a_token, &format!("/api/v1/projects/{pb}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app.get(&a_token, &format!("/api/v1/projects/{pa}")).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .post(&a, &format!("/api/v1/orgs/{beta}/switch"), json!({}))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app
        .get(&a, &format!("/api/v1/overview?project={pb}&minutes=30"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .get(&a, &format!("/api/v1/overview?project={pa}&minutes=30"))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Alpha's things are all still there after B's attempts.
    let (s, _) = app
        .post(&a, &format!("/api/v1/orgs/{alpha}/switch"), json!({}))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&a, &format!("/api/v1/projects/{pa}")).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = app.get(&a, &format!("/api/v1/projects/{pa}/keys")).await;
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .any(|k| k["id"] == json!(key_a) && k["revokedAt"].is_null()),
        "{v}"
    );
    let (_, v) = app
        .get(&a, &format!("/api/v1/alerts/rules?project={pa}"))
        .await;
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
    let (_, v) = app.get(&a, &format!("/api/v1/views?project={pa}")).await;
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
}
