#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//! Differential test: compiled SQL run by the restricted reader in a real
//! ClickHouse must select exactly the rows the in-memory evaluator selects.
//! Skipped when `LOGBOX_TEST_*` is unset.

mod common;

use common::strategies::expr;
use logbox_core::ch::{self, ClickHouse};
use logbox_core::rows::RequestRow;
use logbox_core::testing;
use logbox_query::compile::{ParamValue, where_clause};
use logbox_query::{Ctx, Expr, Prepared, Table, parse};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

const PID: &str = "proj-diff";

fn rows(now_ms: i64) -> Vec<RequestRow> {
    let routes = [
        "/users/:id",
        "/projects",
        "/search",
        "/Search",
        "/auth/login",
        "/files/upload",
        "",
    ];
    let methods = ["GET", "POST", "get", "DELETE"];
    let users = ["u_1", "u_2", "", "U_1", "ü_3"];
    let texts = [
        "",
        "a",
        "abc",
        "ABC",
        "a_b",
        "a%b",
        "it's",
        "back\\slash",
        "tab\there",
        "line\nbreak",
        "éø",
        "x'y",
        "50%",
    ];
    let mut out = Vec::new();
    for i in 0..300u32 {
        let pick = |xs: &[&str], salt: u32| {
            xs[((i * 7 + salt * 13) % xs.len() as u32) as usize].to_string()
        };
        out.push(RequestRow {
            project_id: PID.into(),
            environment: if i % 5 == 0 {
                "staging".into()
            } else {
                "production".into()
            },
            service: pick(&["api", "auth", "worker"], 1),
            release: pick(&["", "v1", "v2"], 2),
            timestamp: now_ms - 3_600_000 * i64::from(i % 50) - 1_000 * i64::from(i),
            request_id: format!("r{i:04}"),
            trace_id: format!("t{}", i % 9),
            method: pick(&methods, 3),
            route: pick(&routes, 4),
            path: format!("/p/{}/{}", pick(&texts, 5), i),
            query: pick(&texts, 6),
            status: [200u16, 201, 204, 301, 400, 404, 422, 429, 500, 503][(i % 10) as usize],
            duration_ms: (i as f32) * 13.37 % 2500.0,
            req_size: i * 10,
            res_size: i * 3,
            user_id: pick(&users, 7),
            client: pick(&["web", "ios", "android", "server", ""], 8),
            client_version: pick(&["1.0", "2.0", ""], 9),
            ip: pick(&["203.0.113.0", "198.51.100.0", ""], 10),
            user_agent: pick(&texts, 11),
            error_type: pick(&["", "", "TimeoutError", "ValidationError"], 12),
            error_message: pick(&texts, 13),
            error_stack: pick(&texts, 14),
            error_fingerprint: u64::from(i % 4) * 1_000_000_007,
            attributes: if i % 3 == 0 {
                vec![]
            } else {
                vec![
                    ("tenant".into(), pick(&texts, 15)),
                    ("region".into(), pick(&["eu", "us"], 16)),
                ]
            },
            correlation: if i % 2 == 0 {
                vec![("account_id".into(), format!("acc_{}", i % 6))]
            } else {
                vec![]
            },
            sample_rate: if i % 4 == 0 { 0.5 } else { 1.0 },
            ..Default::default()
        });
    }
    out
}

const FIXED: &[&str] = &[
    "status >= 500",
    "status IN (200, 404) OR method = 'POST'",
    "status BETWEEN 400 AND 499 AND NOT (route = '/search')",
    "route LIKE '/users/%'",
    "route LIKE '/_earch'",
    "route ILIKE '/search'",
    "route NOT ILIKE '%S%'",
    "query LIKE '%\\\\%%'",
    "query LIKE 'a\\\\_b'",
    "query = 'it''s'",
    "query = 'back\\\\slash'",
    "query = 'tab\there'",
    "query LIKE '%\nbreak'",
    "lower(method) = 'get'",
    "upper(user_id) = 'U_1'",
    "upper(user_id) = 'Ü_3'",
    "length(query) = 2",
    "length(user_id) = 4",
    "position(query, 'b') = 2",
    "position(query, 'ø') = 2",
    "position(query, '') = 1",
    "startsWith(path, '/p/a')",
    "endsWith(path, '0')",
    "match(path, '^/p/[a-z]+/[0-9]+$')",
    "user_id IS NULL",
    "user_id IS NOT NULL AND client IS NULL",
    "attr['tenant'] IS NULL",
    "attr['tenant'] = 'abc'",
    "attr['region'] IN ('eu')",
    "correlation['account_id'] = 'acc_2'",
    "correlation['account_id'] IS NOT NULL",
    "duration_ms > 1000.5",
    "duration_ms <= 13.37",
    "sample_rate < 1",
    "error_fingerprint = 1000000007",
    "error_fingerprint = '2000000014'",
    "error_fingerprint IS NULL",
    "timestamp > now() - INTERVAL 1 DAY",
    "timestamp < now() - INTERVAL 30 HOUR",
    "timestamp BETWEEN now() - INTERVAL 10 HOUR AND now() - INTERVAL 5 HOUR",
    "timestamp > now() - INTERVAL 7200 SECOND AND status < 300",
    "release = '' AND environment != 'staging'",
    "route > '/p' AND route < '/t'",
    "user_id >= 'u'",
    "NOT (status = 200 OR status = 201) AND NOT method IN ('GET', 'get')",
    "error_type IN ('TimeoutError', 'ValidationError') AND status >= 400",
    "req_size + 0 = 0",
    "match(user_id, '^\\\\w+$')",
    "match(path, '\\\\d+$')",
    "match(query, '\\\\s')",
    "match(query, '\\\\bb')",
    "match(path, '(?i)^/P/A')",
    "user_id ILIKE 'ü%'",
    "user_id ILIKE 'Ü%'",
    "lower(user_id) = 'ü_3'",
    "route ILIKE '%SEARCH'",
];

async fn run(
    reader: &clickhouse::Client,
    expr: &Expr,
    rows: &[RequestRow],
    now_ms: i64,
) -> Option<(Vec<String>, Vec<String>)> {
    let ch_now = || async {
        reader
            .query("SELECT toUnixTimestamp64Milli(now64(3))")
            .fetch_one::<i64>()
            .await
            .unwrap()
    };
    let before = ch_now().await;
    let sql = where_clause(
        Table::Requests,
        PID,
        now_ms - 100 * 86_400_000,
        now_ms + 86_400_000,
        Some(expr),
    );
    let mut q = reader.query(&format!(
        "SELECT request_id FROM requests WHERE {} ORDER BY request_id",
        sql.text
    ));
    for p in &sql.params {
        q = match &p.value {
            ParamValue::Str(s) => q.param(&p.name, s),
            ParamValue::Int(i) => q.param(&p.name, i),
            ParamValue::UInt(u) => q.param(&p.name, u),
            ParamValue::Float(f) => q.param(&p.name, f),
            ParamValue::Strs(v) => q.param(&p.name, v),
        };
    }
    let got: Vec<String> = q
        .fetch_all()
        .await
        .unwrap_or_else(|e| panic!("{expr}\n{}\n{e}", sql.text));
    let after = ch_now().await;
    let prepared = Prepared::new(expr).unwrap();
    let eval_at = |at: i64| {
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|r| prepared.matches(*r, at))
            .map(|r| r.request_id.clone())
            .collect();
        ids.sort();
        ids
    };
    let want = eval_at(before);
    // A row within milliseconds of a now()-relative boundary may fall either
    // way while the query runs; such cases are ambiguous, not wrong.
    if want != eval_at(after) {
        return None;
    }
    Some((got, want))
}

#[tokio::test]
async fn clickhouse_and_evaluator_agree() {
    let Some(env) = testing::env().await else {
        eprintln!("skipping: LOGBOX_TEST_* not set");
        return;
    };
    let cfg = env.config;
    ch::migrate(&cfg).await.unwrap();
    let clients = ClickHouse::new(&cfg);
    let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
    let rows = rows(now_ms);
    let mut ins = clients
        .writer
        .insert::<RequestRow>("requests")
        .await
        .unwrap();
    for r in &rows {
        ins.write(r).await.unwrap();
    }
    ins.end().await.unwrap();

    let mut ctx = Ctx::new(Table::Requests);
    ctx.allow_sensitive = true;
    let mut checked = 0;
    for text in FIXED {
        let Ok(Some(expr)) = parse(text, &ctx) else {
            // Rejected expressions (e.g. arithmetic) are fine; they never reach SQL.
            continue;
        };
        if let Some((got, want)) = run(&clients.reader, &expr, &rows, now_ms).await {
            assert_eq!(got, want, "{text}");
            checked += 1;
        }
    }
    assert!(checked >= FIXED.len() - 1);

    // Random well-typed expressions.
    // LOGBOX_DIFF_CASES widens the sweep; LOGBOX_DIFF_RANDOM=1 uses a fresh seed.
    let cases: usize = std::env::var("LOGBOX_DIFF_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let mut runner = if std::env::var("LOGBOX_DIFF_RANDOM").is_ok() {
        TestRunner::default()
    } else {
        TestRunner::deterministic()
    };
    let strategy = expr();
    for _ in 0..cases {
        let e = strategy.new_tree(&mut runner).unwrap().current();
        if let Some((got, want)) = run(&clients.reader, &e, &rows, now_ms).await {
            assert_eq!(got, want, "{e}");
        }
    }
}
