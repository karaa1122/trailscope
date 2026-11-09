#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test code
//! Proptest strategies for random, well-typed filter expressions.

use proptest::prelude::*;
use trailscope_query::ast::{CmpOp, Expr, Interval, Literal, Operand, Sign, Unit};
use trailscope_query::registry::{Kind, REQUESTS};

pub fn columns_of(kind: Kind) -> Vec<&'static trailscope_query::Column> {
    REQUESTS
        .iter()
        .filter(|c| c.kind == kind && !c.sensitive)
        .collect()
}

pub fn text() -> impl Strategy<Value = String> {
    // Printable text including the characters that need escaping.
    proptest::string::string_regex("[a-zA-Z0-9 _%/:.'\\\\\"éø🙂-]{0,24}").unwrap()
}

pub fn str_operand() -> impl Strategy<Value = Operand> {
    let cols = columns_of(Kind::String);
    let maps: Vec<_> = REQUESTS
        .iter()
        .filter(|c| c.kind == Kind::Map && !c.sensitive)
        .collect();
    let base = prop_oneof![
        proptest::sample::select(cols).prop_map(Operand::Column),
        (proptest::sample::select(maps), "[a-z_]{1,10}")
            .prop_map(|(map, key)| Operand::MapKey { map, key }),
    ];
    base.prop_recursive(1, 4, 1, |inner| {
        prop_oneof![
            inner.clone().prop_map(|o| Operand::Lower(Box::new(o))),
            inner.prop_map(|o| Operand::Upper(Box::new(o))),
        ]
    })
}

pub fn num_literal() -> impl Strategy<Value = Literal> {
    prop_oneof![
        (-100_000i64..100_000).prop_map(Literal::Int),
        (-1.0e6f64..1.0e6)
            .prop_filter("non-integral", |f| f.fract() != 0.0)
            .prop_map(Literal::Float),
    ]
}

pub fn cmp_op() -> impl Strategy<Value = CmpOp> {
    proptest::sample::select(vec![
        CmpOp::Eq,
        CmpOp::Ne,
        CmpOp::Lt,
        CmpOp::Le,
        CmpOp::Gt,
        CmpOp::Ge,
    ])
}

pub fn leaf() -> impl Strategy<Value = Expr> {
    let num_cols = columns_of(Kind::Number);
    prop_oneof![
        (
            proptest::sample::select(num_cols.clone()),
            cmp_op(),
            num_literal()
        )
            .prop_map(|(c, op, l)| Expr::Compare {
                left: Operand::Column(c),
                op,
                right: Operand::Literal(l),
            }),
        (str_operand(), cmp_op(), text()).prop_map(|(o, op, s)| Expr::Compare {
            left: o,
            op,
            right: Operand::Literal(Literal::Str(s)),
        }),
        (
            str_operand(),
            proptest::collection::vec(text(), 1..5),
            any::<bool>()
        )
            .prop_map(|(o, list, negated)| Expr::In {
                expr: o,
                list: list.into_iter().map(Literal::Str).collect(),
                negated,
            }),
        (
            proptest::sample::select(num_cols),
            num_literal(),
            num_literal(),
            any::<bool>()
        )
            .prop_map(|(c, a, b, negated)| Expr::Between {
                expr: Operand::Column(c),
                low: Operand::Literal(a),
                high: Operand::Literal(b),
                negated,
            }),
        (str_operand(), text(), any::<bool>(), any::<bool>())
            .prop_filter("valid LIKE pattern", |(_, p, _, _)| {
                trailscope_query::parse::check_like_pattern(p).is_ok()
            })
            .prop_map(|(o, pattern, negated, ci)| Expr::Like {
                expr: o,
                pattern,
                negated,
                case_insensitive: ci,
            }),
        (str_operand(), any::<bool>()).prop_map(|(o, negated)| Expr::IsEmpty { expr: o, negated }),
        (str_operand(), text()).prop_map(|(o, s)| Expr::StartsWith(o, s)),
        (str_operand(), text()).prop_map(|(o, s)| Expr::EndsWith(o, s)),
        (str_operand(), "[a-z0-9^$.*+]{1,12}")
            .prop_filter("valid regex", |(_, re)| {
                trailscope_query::regex::re2_pattern(re).is_ok()
            })
            .prop_map(|(o, re)| Expr::Match(o, re)),
        (str_operand(), cmp_op(), 0i64..100).prop_map(|(o, op, n)| Expr::Compare {
            left: Operand::Length(Box::new(o)),
            op,
            right: Operand::Literal(Literal::Int(n)),
        }),
        (
            cmp_op(),
            0i64..1000,
            proptest::sample::select(vec![Unit::Second, Unit::Minute, Unit::Hour, Unit::Day]),
            any::<bool>()
        )
            .prop_map(|(op, amount, unit, plus)| Expr::Compare {
                left: Operand::Column(REQUESTS.iter().find(|c| c.name == "timestamp").unwrap()),
                op,
                right: Operand::TimeOffset {
                    base: Box::new(Operand::Now),
                    sign: if plus { Sign::Plus } else { Sign::Minus },
                    interval: Interval { amount, unit },
                },
            }),
    ]
}

/// Normalizes nested And/Or the way the parser flattens them.
pub fn flatten(e: Expr) -> Expr {
    match e {
        Expr::And(xs) => {
            let mut out = Vec::new();
            for x in xs.into_iter().map(flatten) {
                match x {
                    Expr::And(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 {
                out.pop().unwrap()
            } else {
                Expr::And(out)
            }
        }
        Expr::Or(xs) => {
            let mut out = Vec::new();
            for x in xs.into_iter().map(flatten) {
                match x {
                    Expr::Or(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 {
                out.pop().unwrap()
            } else {
                Expr::Or(out)
            }
        }
        Expr::Not(x) => Expr::Not(Box::new(flatten(*x))),
        other => other,
    }
}

pub fn expr() -> impl Strategy<Value = Expr> {
    leaf()
        .prop_recursive(4, 24, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 2..4).prop_map(Expr::And),
                proptest::collection::vec(inner.clone(), 2..4).prop_map(Expr::Or),
                inner.prop_map(|e| Expr::Not(Box::new(e))),
            ]
        })
        .prop_map(flatten)
}
