//! Evaluates the same AST against one in-memory record, so the live tail and
//! real-time alerts filter exactly like Explore does in ClickHouse.
//!
//! Semantics follow the compiled SQL: string comparisons are bytewise,
//! `lower`/`upper`/`length`/`position` are Unicode-aware (the SQL uses the
//! `UTF8` variants), LIKE treats `%`/`_` as wildcards with `\` as escape, a
//! missing map key reads as an empty string.

use std::borrow::Cow;
use std::cmp::Ordering;

use logbox_core::rows::{LiveRequest, RequestRow};
use regex::Regex;

use crate::ast::{CmpOp, Expr, Literal, Operand, Sign};

/// A value read from a record.
#[derive(Debug, Clone, PartialEq)]
pub enum Val<'a> {
    Str(Cow<'a, str>),
    Num(f64),
    U64(u64),
    Bool(bool),
    /// Milliseconds since the epoch.
    Time(i64),
}

/// Anything the evaluator can read columns from.
pub trait Record {
    /// The value of a registry column, or `None` if the record doesn't carry it.
    fn value(&self, column: &str) -> Option<Val<'_>>;
    /// The value of `map[key]`, or `None` when the key is absent.
    fn map_get(&self, map: &str, key: &str) -> Option<&str>;
}

/// An expression prepared for repeated evaluation (patterns compiled once).
#[derive(Debug, Clone)]
pub struct Prepared {
    root: Node,
}

#[derive(Debug, Clone)]
enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Compare(Operand, CmpOp, Operand),
    In(Operand, Vec<Literal>, bool),
    Between(Operand, Operand, Operand, bool),
    Like(Operand, Regex, bool),
    IsEmpty(Operand, bool),
    StartsWith(Operand, String),
    EndsWith(Operand, String),
    Match(Operand, Regex),
}

impl Prepared {
    pub fn new(expr: &Expr) -> Result<Self, String> {
        Ok(Self {
            root: prepare(expr)?,
        })
    }

    pub fn matches(&self, rec: &dyn Record, now_ms: i64) -> bool {
        eval(&self.root, rec, now_ms)
    }
}

fn prepare(e: &Expr) -> Result<Node, String> {
    Ok(match e {
        Expr::And(xs) => Node::And(xs.iter().map(prepare).collect::<Result<_, _>>()?),
        Expr::Or(xs) => Node::Or(xs.iter().map(prepare).collect::<Result<_, _>>()?),
        Expr::Not(x) => Node::Not(Box::new(prepare(x)?)),
        Expr::Compare { left, op, right } => Node::Compare(left.clone(), *op, right.clone()),
        Expr::In {
            expr,
            list,
            negated,
        } => Node::In(expr.clone(), list.clone(), *negated),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Node::Between(expr.clone(), low.clone(), high.clone(), *negated),
        Expr::Like {
            expr,
            pattern,
            negated,
            case_insensitive,
        } => {
            let re = Regex::new(&like_to_regex(pattern, *case_insensitive))
                .map_err(|e| e.to_string())?;
            Node::Like(expr.clone(), re, *negated)
        }
        Expr::IsEmpty { expr, negated } => Node::IsEmpty(expr.clone(), *negated),
        Expr::StartsWith(x, s) => Node::StartsWith(x.clone(), s.clone()),
        Expr::EndsWith(x, s) => Node::EndsWith(x.clone(), s.clone()),
        Expr::Match(x, re) => {
            let rust = crate::regex::re2_pattern(re)?;
            Node::Match(x.clone(), Regex::new(&rust).map_err(|e| e.to_string())?)
        }
    })
}

/// SQL LIKE pattern → anchored regex. `%` = any run, `_` = one character,
/// `\` escapes the next character.
pub fn like_to_regex(pattern: &str, case_insensitive: bool) -> String {
    let mut out = String::from(if case_insensitive { "(?is)^" } else { "(?s)^" });
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push_str(&regex::escape(&next.to_string()));
                } else {
                    out.push_str(&regex::escape("\\"));
                }
            }
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    out.push('$');
    out
}

fn eval(n: &Node, r: &dyn Record, now: i64) -> bool {
    match n {
        Node::And(xs) => xs.iter().all(|x| eval(x, r, now)),
        Node::Or(xs) => xs.iter().any(|x| eval(x, r, now)),
        Node::Not(x) => !eval(x, r, now),
        Node::Compare(a, op, b) => match (value(a, r, now), value(b, r, now)) {
            (Some(a), Some(b)) => compare(&a, &b).is_some_and(|o| holds(*op, o)),
            _ => false,
        },
        Node::In(x, list, negated) => {
            let Some(v) = value(x, r, now) else {
                return false;
            };
            let found = list
                .iter()
                .any(|l| compare(&v, &literal(l)) == Some(Ordering::Equal));
            found != *negated
        }
        Node::Between(x, lo, hi, negated) => {
            let (Some(v), Some(lo), Some(hi)) =
                (value(x, r, now), value(lo, r, now), value(hi, r, now))
            else {
                return false;
            };
            let inside = matches!(compare(&v, &lo), Some(Ordering::Greater | Ordering::Equal))
                && matches!(compare(&v, &hi), Some(Ordering::Less | Ordering::Equal));
            inside != *negated
        }
        Node::Like(x, re, negated) => match value(x, r, now) {
            Some(Val::Str(s)) => re.is_match(&s) != *negated,
            _ => false,
        },
        Node::IsEmpty(x, negated) => {
            let empty = match value(x, r, now) {
                None => true,
                Some(Val::Str(s)) => s.is_empty(),
                Some(Val::Num(f)) => f == 0.0,
                Some(Val::U64(u)) => u == 0,
                Some(Val::Bool(b)) => !b,
                Some(Val::Time(t)) => t == 0,
            };
            empty != *negated
        }
        Node::StartsWith(x, s) => {
            matches!(value(x, r, now), Some(Val::Str(v)) if v.starts_with(s.as_str()))
        }
        Node::EndsWith(x, s) => {
            matches!(value(x, r, now), Some(Val::Str(v)) if v.ends_with(s.as_str()))
        }
        Node::Match(x, re) => matches!(value(x, r, now), Some(Val::Str(v)) if re.is_match(&v)),
    }
}

fn holds(op: CmpOp, o: Ordering) -> bool {
    match op {
        CmpOp::Eq => o == Ordering::Equal,
        CmpOp::Ne => o != Ordering::Equal,
        CmpOp::Lt => o == Ordering::Less,
        CmpOp::Le => o != Ordering::Greater,
        CmpOp::Gt => o == Ordering::Greater,
        CmpOp::Ge => o != Ordering::Less,
    }
}

fn compare(a: &Val<'_>, b: &Val<'_>) -> Option<Ordering> {
    match (a, b) {
        (Val::Str(a), Val::Str(b)) => Some(a.as_bytes().cmp(b.as_bytes())),
        (Val::Num(a), Val::Num(b)) => a.partial_cmp(b),
        (Val::U64(a), Val::U64(b)) => Some(a.cmp(b)),
        (Val::Bool(a), Val::Bool(b)) => Some(a.cmp(b)),
        (Val::Time(a), Val::Time(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn literal(l: &Literal) -> Val<'_> {
    match l {
        Literal::Str(s) => Val::Str(Cow::Borrowed(s)),
        Literal::Int(i) => Val::Num(*i as f64),
        Literal::Float(f) => Val::Num(*f),
        Literal::Bool(b) => Val::Bool(*b),
        Literal::Time(t) => Val::Time(*t),
        Literal::U64(u) => Val::U64(*u),
    }
}

fn value<'a>(o: &'a Operand, r: &'a dyn Record, now: i64) -> Option<Val<'a>> {
    match o {
        Operand::Column(c) => r.value(c.name),
        Operand::MapKey { map, key } => Some(Val::Str(Cow::Borrowed(
            r.map_get(map.name, key).unwrap_or(""),
        ))),
        Operand::Literal(l) => Some(literal(l)),
        Operand::Now => Some(Val::Time(now)),
        Operand::TimeOffset {
            base,
            sign,
            interval,
        } => match value(base, r, now)? {
            Val::Time(t) => {
                let d = interval.amount.saturating_mul(interval.unit.millis());
                Some(Val::Time(if *sign == Sign::Plus {
                    t.saturating_add(d)
                } else {
                    t.saturating_sub(d)
                }))
            }
            _ => None,
        },
        Operand::Lower(a) => match value(a, r, now)? {
            Val::Str(s) => Some(Val::Str(Cow::Owned(s.to_lowercase()))),
            _ => None,
        },
        Operand::Upper(a) => match value(a, r, now)? {
            Val::Str(s) => Some(Val::Str(Cow::Owned(s.to_uppercase()))),
            _ => None,
        },
        Operand::Length(a) => match value(a, r, now)? {
            Val::Str(s) => Some(Val::Num(s.chars().count() as f64)),
            _ => None,
        },
        Operand::Position(h, n) => match (value(h, r, now)?, value(n, r, now)?) {
            (Val::Str(h), Val::Str(n)) => Some(Val::Num(position_utf8(&h, &n) as f64)),
            _ => None,
        },
    }
}

/// 1-based character position of `needle` in `hay`, 0 if absent, 1 for an
/// empty needle (like ClickHouse `positionUTF8`).
fn position_utf8(hay: &str, needle: &str) -> usize {
    match hay.find(needle) {
        Some(byte) => hay[..byte].chars().count() + 1,
        None => 0,
    }
}

fn map_lookup<'a>(m: &'a [(String, String)], key: &str) -> Option<&'a str> {
    m.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

impl Record for LiveRequest {
    fn value(&self, column: &str) -> Option<Val<'_>> {
        let s = |v: &str| Some(Val::Str(Cow::Owned(v.to_string())));
        match column {
            "timestamp" => Some(Val::Time(self.timestamp)),
            "environment" => s(&self.environment),
            "service" => s(&self.service),
            "release" => s(&self.release),
            "request_id" => s(&self.request_id),
            "trace_id" => s(&self.trace_id),
            "method" => s(&self.method),
            "route" => s(&self.route),
            "path" => s(&self.path),
            "query" => s(&self.query),
            "status" => Some(Val::Num(f64::from(self.status))),
            "duration_ms" => Some(Val::Num(f64::from(self.duration_ms))),
            "req_size" => Some(Val::Num(f64::from(self.req_size))),
            "res_size" => Some(Val::Num(f64::from(self.res_size))),
            "user_id" => s(&self.user_id),
            "client" => s(&self.client),
            "client_version" => s(&self.client_version),
            "ip" => s(&self.ip),
            "user_agent" => s(&self.user_agent),
            "error_type" => s(&self.error_type),
            "error_message" => s(&self.error_message),
            "error_fingerprint" => Some(Val::U64(self.error_fingerprint.parse().unwrap_or(0))),
            "sample_rate" => Some(Val::Num(f64::from(self.sample_rate))),
            _ => None,
        }
    }

    fn map_get(&self, map: &str, key: &str) -> Option<&str> {
        match map {
            "attributes" => map_lookup(&self.attributes, key),
            "correlation" => map_lookup(&self.correlation, key),
            _ => None,
        }
    }
}

impl Record for RequestRow {
    fn value(&self, column: &str) -> Option<Val<'_>> {
        let s = |v: &'_ str| Some(Val::Str(Cow::Owned(v.to_string())));
        match column {
            "timestamp" => Some(Val::Time(self.timestamp)),
            "environment" => s(&self.environment),
            "service" => s(&self.service),
            "release" => s(&self.release),
            "request_id" => s(&self.request_id),
            "trace_id" => s(&self.trace_id),
            "span_id" => s(&self.span_id),
            "method" => s(&self.method),
            "route" => s(&self.route),
            "path" => s(&self.path),
            "query" => s(&self.query),
            "status" => Some(Val::Num(f64::from(self.status))),
            "duration_ms" => Some(Val::Num(f64::from(self.duration_ms))),
            "req_size" => Some(Val::Num(f64::from(self.req_size))),
            "res_size" => Some(Val::Num(f64::from(self.res_size))),
            "user_id" => s(&self.user_id),
            "client" => s(&self.client),
            "client_version" => s(&self.client_version),
            "ip" => s(&self.ip),
            "user_agent" => s(&self.user_agent),
            "req_body" => s(&self.req_body),
            "res_body" => s(&self.res_body),
            "body_truncated" => Some(Val::Bool(self.body_truncated != 0)),
            "error_type" => s(&self.error_type),
            "error_message" => s(&self.error_message),
            "error_stack" => s(&self.error_stack),
            "error_fingerprint" => Some(Val::U64(self.error_fingerprint)),
            "sample_rate" => Some(Val::Num(f64::from(self.sample_rate))),
            _ => None,
        }
    }

    fn map_get(&self, map: &str, key: &str) -> Option<&str> {
        match map {
            "attributes" => map_lookup(&self.attributes, key),
            "correlation" => map_lookup(&self.correlation, key),
            "req_headers" => map_lookup(&self.req_headers, key),
            "res_headers" => map_lookup(&self.res_headers, key),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Ctx, parse};
    use crate::registry::Table;

    fn req() -> LiveRequest {
        LiveRequest {
            timestamp: 1_800_000_000_000,
            service: "api".into(),
            method: "GET".into(),
            route: "/users/:id".into(),
            path: "/users/42".into(),
            status: 503,
            duration_ms: 1200.0,
            user_id: "u_1".into(),
            client: "web".into(),
            error_type: "TimeoutError".into(),
            error_fingerprint: "42".into(),
            attributes: vec![("tenant".into(), "acme".into())],
            correlation: vec![("account_id".into(), "acc_1".into())],
            ..Default::default()
        }
    }

    fn check(s: &str) -> bool {
        let mut ctx = Ctx::new(Table::Requests);
        ctx.live = true;
        let e = parse(s, &ctx).unwrap().unwrap();
        Prepared::new(&e)
            .unwrap()
            .matches(&req(), 1_800_000_060_000)
    }

    #[test]
    fn evaluates_every_operator() {
        let yes = [
            "status = 503",
            "status != 500",
            "status > 500",
            "status >= 503",
            "status < 600",
            "status <= 503",
            "500 < status",
            "status IN (502, 503)",
            "status NOT IN (200)",
            "status BETWEEN 500 AND 599",
            "status NOT BETWEEN 200 AND 299",
            "route LIKE '/users/%'",
            "route LIKE '/users/:i_'",
            "route NOT LIKE '/a%'",
            "path ILIKE '/USERS/%'",
            "release IS NULL",
            "user_id IS NOT NULL",
            "attr['missing'] IS NULL",
            "attr['tenant'] = 'acme'",
            "correlation['account_id'] IN ('acc_1')",
            "NOT status = 200",
            "(status = 1 OR status = 503) AND method = 'GET'",
            "lower(method) = 'get'",
            "upper(client) = 'WEB'",
            "length(path) = 9",
            "position(path, '42') = 8",
            "startsWith(path, '/users')",
            "endsWith(path, '42')",
            "match(path, '^/users/[0-9]+$')",
            "timestamp > now() - INTERVAL 5 MINUTE",
            "timestamp < now()",
            "timestamp >= '2027-01-15 08:00:00'",
            "error_fingerprint = 42",
            "duration_ms > 1000",
            "route > '/a'",
        ];
        for s in yes {
            assert!(check(s), "expected match: {s}");
        }
        let no = [
            "status = 200",
            "route LIKE '/users'",
            "attr['tenant'] = 'other'",
            "timestamp > now()",
            "timestamp < now() - INTERVAL 1 HOUR",
            "match(path, '^/v1')",
            "startsWith(path, 'users')",
            "error_fingerprint = 43",
            "status IN (200, 201)",
        ];
        for s in no {
            assert!(!check(s), "expected no match: {s}");
        }
    }

    #[test]
    fn like_escapes() {
        let re = Regex::new(&like_to_regex(r"100\%_x", false)).unwrap();
        assert!(re.is_match("100%ax"));
        assert!(!re.is_match("1000ax"));
        let re = Regex::new(&like_to_regex("a.b(c)%", false)).unwrap();
        assert!(re.is_match("a.b(c)zzz"));
        assert!(!re.is_match("aXb(c)"));
    }

    #[test]
    fn position_is_character_based() {
        assert_eq!(position_utf8("héllo", "l"), 3);
        assert_eq!(position_utf8("abc", "z"), 0);
        assert_eq!(position_utf8("abc", ""), 1);
    }
}
