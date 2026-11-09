#![allow(clippy::unwrap_used, clippy::expect_used)] // test code
//! Property tests: parse → print → parse round-trips, compiled SQL is always
//! safe, parameters match literals, and arbitrary input never panics.

mod common;

use common::assert_safe;
use common::strategies::{expr, flatten};
use proptest::prelude::*;
use trailscope_query::ast::{Expr, Operand};
use trailscope_query::compile::compile;
use trailscope_query::{Ctx, Table, chips_to_expr, expr_to_chips, parse, parse_search, to_search};

fn ctx() -> Ctx {
    Ctx::new(Table::Requests)
}

fn literal_count(e: &Expr) -> usize {
    fn op(o: &Operand) -> usize {
        match o {
            Operand::Literal(_) => 1,
            Operand::MapKey { .. } | Operand::TimeOffset { .. } => 1,
            Operand::Lower(a) | Operand::Upper(a) | Operand::Length(a) => op(a),
            Operand::Position(a, b) => op(a) + op(b),
            Operand::Column(_) | Operand::Now => 0,
        }
    }
    match e {
        Expr::And(xs) | Expr::Or(xs) => xs.iter().map(literal_count).sum(),
        Expr::Not(x) => literal_count(x),
        Expr::Compare { left, right, .. } => op(left) + op(right),
        Expr::In { expr, list, .. } => op(expr) + list.len(),
        Expr::Between {
            expr, low, high, ..
        } => op(expr) + op(low) + op(high),
        Expr::Like { expr, .. }
        | Expr::StartsWith(expr, _)
        | Expr::EndsWith(expr, _)
        | Expr::Match(expr, _) => op(expr) + 1,
        Expr::IsEmpty { expr, .. } => op(expr),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    #[test]
    fn print_then_parse_round_trips(e in expr()) {
        let text = e.to_string();
        let back = parse(&text, &ctx()).map_err(|err| TestCaseError::fail(format!("{text}\n{err}")))?;
        prop_assert_eq!(back, Some(e), "{}", text);
    }

    #[test]
    fn compiled_sql_is_safe_and_fully_parameterized(e in expr()) {
        let sql = compile(&e);
        assert_safe(&sql);
        prop_assert_eq!(sql.params.len(), literal_count(&e));
    }

    #[test]
    fn arbitrary_text_never_panics_and_compiles_safely(s in "\\PC{0,200}") {
        if let Ok(Some(e)) = parse(&s, &ctx()) {
            assert_safe(&compile(&e));
        }
        if let Ok(Some(e)) = parse_search(&s, &ctx()) {
            assert_safe(&compile(&e));
        }
    }

    #[test]
    fn sql_like_text_never_panics(s in "[a-z_ ()'=<>!,%*.;\\-\\[\\]0-9]{0,120}") {
        if let Ok(Some(e)) = parse(&s, &ctx()) {
            assert_safe(&compile(&e));
        }
    }

    #[test]
    fn chips_round_trip_when_representable(e in expr()) {
        if let Some(chips) = expr_to_chips(&e) {
            let back = chips_to_expr(&chips, &ctx()).unwrap().unwrap();
            prop_assert_eq!(expr_to_chips(&back), Some(chips));
        }
    }

    #[test]
    fn search_round_trips_when_representable(e in expr()) {
        if let Some(s) = to_search(&e, &ctx()) {
            let back = parse_search(&s, &ctx()).map_err(|err| TestCaseError::fail(format!("{s}\n{err}")))?;
            prop_assert_eq!(back.map(flatten), Some(e), "{}", s);
        }
    }
}
