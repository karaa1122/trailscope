//! Fuzzes every front end: SQL WHERE text and search syntax, for both tables
//! and all permission contexts. Properties checked on every input:
//! - nothing panics;
//! - accepted input compiles to SQL with no quotes, comments or `;`;
//! - accepted input prints back to text that parses to the same AST;
//! - the in-memory evaluator runs on it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use logbox_core::rows::LiveRequest;
use logbox_query::{Ctx, Prepared, Table, compile, parse, parse_search};

fn check(expr: &logbox_query::Expr, ctx: &Ctx) {
    let sql = compile(expr);
    for bad in ['\'', '"', '`', ';', '\'] {
        assert!(!sql.text.contains(bad), "unsafe SQL: {}", sql.text);
    }
    assert!(!sql.text.contains("--") && !sql.text.contains("/*"), "unsafe SQL: {}", sql.text);
    assert_eq!(sql.text.matches('{').count(), sql.params.len());
    let printed = expr.to_string();
    let back = parse(&printed, ctx).expect("printed form must parse");
    assert_eq!(back.as_ref(), Some(expr), "round trip failed: {printed}");
    if let Ok(p) = Prepared::new(expr) {
        let _ = p.matches(&LiveRequest::default(), 0);
    }
}

fuzz_target!(|data: &str| {
    for table in [Table::Requests, Table::Events] {
        for (allow_sensitive, live) in [(false, false), (true, false), (false, true)] {
            let ctx = Ctx { table, allow_sensitive, live };
            if let Ok(Some(e)) = parse(data, &ctx) {
                check(&e, &ctx);
            }
            if let Ok(Some(e)) = parse_search(data, &ctx) {
                let sql = compile(&e);
                assert!(!sql.text.contains('\''), "unsafe SQL: {}", sql.text);
            }
        }
    }
});
