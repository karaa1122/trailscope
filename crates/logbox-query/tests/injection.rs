#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//! Hostile inputs must fail safely: either a clear error, or a valid filter
//! whose compiled SQL contains the hostile text only inside parameters.

mod common;

use common::assert_safe;
use logbox_query::compile::{ParamValue, compile, where_clause};
use logbox_query::{Ctx, Table, parse, parse_search};

fn ctx() -> Ctx {
    Ctx::new(Table::Requests)
}

/// Inputs that must be rejected outright.
const REJECTED: &[&str] = &[
    "1=1; DROP TABLE requests",
    "status = 1; DROP TABLE requests",
    "status = 1;DROP TABLE requests;",
    "status = 1 UNION SELECT * FROM system.users",
    "status = 1 UNION ALL SELECT name FROM system.tables",
    "status = 1 -- AND route = '/x'",
    "status = 1 --",
    "status = 1 /* comment */",
    "/* leading */ status = 1",
    "status = 1 #comment",
    "status IN (SELECT status FROM requests)",
    "status IN (SELECT status FROM (SELECT * FROM system.users))",
    "status = (SELECT max(status) FROM requests)",
    "EXISTS (SELECT 1)",
    "attr['x'']) OR 1=1 --",
    "attr['x']) OR 1=1 --",
    "attr['x') OR 1=1 --']",
    "route = ‘x’ OR 1=1",
    "route = ’x’",
    "route = “x”",
    "route = ＇x＇",
    "route = `x`",
    "system.users.name = 'default'",
    "name IN (SELECT name FROM system.users)",
    "sleep(3) = 0",
    "status = 1 OR sleep(3) = 0",
    "file('/etc/passwd') = ''",
    "url('http://example.com') = ''",
    "remote('host', system.one) = 1",
    "currentUser() = 'default'",
    "toString(status) = '1'",
    "CAST(status AS String) = '1'",
    "status::String = '1'",
    "status = 1 SETTINGS max_execution_time = 1000",
    "status = 1 FORMAT JSON",
    "status = 1 INTO OUTFILE '/tmp/x'",
    "status = 1 LIMIT 1000000",
    "status = 1 ORDER BY 1",
    "status = 1 GROUP BY status",
    "status = 1) OR (1=1",
    "(status = 1",
    "status = 1)",
    "status = {p1:String}",
    "status = $1",
    "status = ?",
    "status = 1 AND",
    "SELECT 1",
    "DROP TABLE requests",
    "1",
    "'x'",
    "1 = 1",
    "'a' = 'a'",
    "now() > now() - INTERVAL 1 DAY",
    "requests.status = 1",
    "logbox.requests.status = 1",
    "status = x'41'",
    "route = e'\\x27'",
    "route = $$x$$",
    "status = 1 AND route = \"x\"",
    "arrayJoin([1]) = 1",
    "dictGet('d', 'a', 1) = 1",
    "status = 1 OR TRUE",
    "route = CHAR(39)",
    "route LIKE ANY ('a')",
    "route LIKE 'x' ESCAPE '!'",
];

#[test]
fn rejected_inputs_fail_with_positioned_errors() {
    for input in REJECTED {
        for c in [
            ctx(),
            Ctx {
                live: true,
                ..ctx()
            },
        ] {
            match parse(input, &c) {
                Err(e) => {
                    assert!(!e.message.is_empty());
                    assert!(e.line >= 1 && e.col >= 1, "{input}: {e:?}");
                }
                Ok(Some(e)) => panic!("accepted hostile input {input:?} as {e}"),
                Ok(None) => panic!("hostile input {input:?} was treated as empty"),
            }
        }
    }
}

/// Inputs that are valid filters: the hostile-looking text must end up only
/// in parameters.
const ACCEPTED_AS_DATA: &[(&str, &str)] = &[
    ("route = 'x'' OR ''1''=''1'", "x' OR '1'='1"),
    (
        "route = '; DROP TABLE requests; --'",
        "; DROP TABLE requests; --",
    ),
    ("route = '/* */'", "/* */"),
    ("route = '\\\\'' OR 1=1 --'", "\\' OR 1=1 --"),
    (
        "route LIKE '%'' UNION SELECT 1 --%'",
        "%' UNION SELECT 1 --%",
    ),
    ("attr['x'') OR 1=1 --'] = 'v'", "x') OR 1=1 --"),
    // A backslash-escaped quote keeps the whole thing one string literal.
    ("route = '\\' OR 1=1 --'", "' OR 1=1 --"),
    ("route = '{p1:String}'", "{p1:String}"),
    ("route = 'system.users'", "system.users"),
    ("route = 'ä\u{0}b'", "ä\u{0}b"),
    ("match(path, '\\'')", "'"),
];

#[test]
fn hostile_strings_become_parameters() {
    for (input, expected) in ACCEPTED_AS_DATA {
        let expr = parse(input, &ctx())
            .unwrap_or_else(|e| panic!("{input}: {e}"))
            .unwrap();
        let sql = compile(&expr);
        assert_safe(&sql);
        let values: Vec<&ParamValue> = sql.params.iter().map(|p| &p.value).collect();
        assert!(
            values.contains(&&ParamValue::Str((*expected).to_string())),
            "{input}: expected param {expected:?}, got {values:?}"
        );
    }
}

#[test]
fn search_syntax_is_just_as_safe() {
    for input in [
        "route:\"'; DROP TABLE requests; --\"",
        "\"x' OR 1=1 --\"",
        "attr.x')--:v",
        "status:>=500 \"/* */\"",
        "-route:*'*",
    ] {
        if let Some(expr) = parse_search(input, &ctx()).unwrap_or_else(|e| panic!("{input}: {e}")) {
            assert_safe(&compile(&expr));
        }
    }
    for bad in ["system.users:x", "status:1;DROP", "req_body:x", "nope:1"] {
        assert!(parse_search(bad, &ctx()).is_err(), "{bad}");
    }
}

#[test]
fn deep_nesting_is_refused_without_stack_overflow() {
    for depth in [33, 100, 1_000, 1_990] {
        let s = format!("{}status = 1{}", "(".repeat(depth), ")".repeat(depth));
        assert!(parse(&s, &ctx()).is_err(), "depth {depth}");
        let nots = format!("{}status = 1", "NOT ".repeat(depth));
        assert!(parse(&nots, &ctx()).is_err(), "NOT depth {depth}");
    }
    // Nesting within the limit is fine.
    let ok = format!("{}status = 1{}", "(".repeat(20), ")".repeat(20));
    assert!(parse(&ok, &ctx()).is_ok());
}

#[test]
fn huge_and_odd_inputs() {
    let long = "status = 1 OR ".repeat(400) + "status = 2";
    assert!(parse(&long, &ctx()).is_err());
    assert!(parse(&"x".repeat(10_000), &ctx()).is_err());
    for odd in [
        "\u{feff}status = 1",
        "status\u{00a0}= 1",
        "status = 1\u{202e}",
        "🙂 = 1",
        "status = '🙂'",
    ] {
        if let Ok(Some(e)) = parse(odd, &ctx()) {
            assert_safe(&compile(&e));
        }
    }
}

#[test]
fn the_wrapper_is_safe_too() {
    let e = parse("route = 'x'' OR 1=1 --'", &ctx()).unwrap();
    let sql = where_clause(Table::Requests, "p'roject", 0, 1, e.as_ref());
    assert_safe(&sql);
    assert_eq!(sql.params[0].value, ParamValue::Str("p'roject".into()));
}
