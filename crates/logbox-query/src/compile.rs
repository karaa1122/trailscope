//! AST → ClickHouse SQL.
//!
//! The output only ever contains column names from the registry, a fixed set
//! of function names and keywords, and `{pN:Type}` parameter placeholders.
//! Every value from the user (strings, numbers, map keys, patterns) is a
//! parameter, so the compiled SQL never contains a quote character.

use serde::Serialize;

use crate::ast::{CmpOp, Expr, Literal, Operand, Sign, Ty, Unit};
use crate::registry::{Kind, Table};

/// A typed query parameter, sent to ClickHouse as `param_<name>`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    Str(String),
    Int(i64),
    UInt(u64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Param {
    pub name: String,
    pub ty: &'static str,
    pub value: ParamValue,
}

/// Compiled SQL with its parameters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sql {
    pub text: String,
    pub params: Vec<Param>,
}

/// Collects parameters as SQL is emitted. Names are `{prefix}{n}`.
pub struct Params {
    prefix: &'static str,
    pub list: Vec<Param>,
}

impl Params {
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            list: Vec::new(),
        }
    }

    /// Adds a parameter and returns its `{name:Type}` placeholder.
    pub fn add(&mut self, ty: &'static str, value: ParamValue) -> String {
        let name = format!("{}{}", self.prefix, self.list.len() + 1);
        let placeholder = format!("{{{name}:{ty}}}");
        self.list.push(Param { name, ty, value });
        placeholder
    }

    pub fn str(&mut self, s: &str) -> String {
        self.add("String", ParamValue::Str(s.to_string()))
    }

    pub fn int(&mut self, i: i64) -> String {
        self.add("Int64", ParamValue::Int(i))
    }
}

/// Compiles a filter expression (without the project/time wrapper).
pub fn compile(expr: &Expr) -> Sql {
    let mut params = Params::new("p");
    let text = expr_sql(expr, &mut params);
    Sql {
        text,
        params: params.list,
    }
}

/// Compiles into an existing parameter set (when combining several filters).
pub fn compile_into(expr: &Expr, params: &mut Params) -> String {
    expr_sql(expr, params)
}

/// The full `WHERE` clause: project, time range, then the user's filter.
/// `from_ms`/`to_ms` are inclusive bounds from the UI time picker; the
/// expression can only narrow them further.
pub fn where_clause(
    table: Table,
    project_id: &str,
    from_ms: i64,
    to_ms: i64,
    filter: Option<&Expr>,
) -> Sql {
    let mut params = Params::new("p");
    let pid = params.add("String", ParamValue::Str(project_id.to_string()));
    let from = params.int(from_ms);
    let to = params.int(to_ms);
    let ts = time_column(table);
    let mut text = format!(
        "project_id = {pid} AND {ts} >= fromUnixTimestamp64Milli({from}) AND {ts} <= fromUnixTimestamp64Milli({to})"
    );
    if let Some(expr) = filter {
        text.push_str(" AND (");
        text.push_str(&expr_sql(expr, &mut params));
        text.push(')');
    }
    Sql {
        text,
        params: params.list,
    }
}

fn time_column(table: Table) -> &'static str {
    match table {
        Table::Requests | Table::Events => "timestamp",
    }
}

fn expr_sql(e: &Expr, p: &mut Params) -> String {
    match e {
        Expr::And(xs) => join(xs, " AND ", p),
        Expr::Or(xs) => join(xs, " OR ", p),
        Expr::Not(x) => format!("NOT ({})", expr_sql(x, p)),
        Expr::Compare { left, op, right } => {
            let (l, r) = (operand_sql(left, p), operand_sql(right, p));
            format!("({l} {} {r})", cmp_sql(*op))
        }
        Expr::In {
            expr,
            list,
            negated,
        } => {
            let x = operand_sql(expr, p);
            let items: Vec<String> = list.iter().map(|l| literal_sql(l, p)).collect();
            let not = if *negated { "NOT " } else { "" };
            format!("({x} {not}IN ({}))", items.join(", "))
        }
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let (x, lo, hi) = (
                operand_sql(expr, p),
                operand_sql(low, p),
                operand_sql(high, p),
            );
            let not = if *negated { "NOT " } else { "" };
            format!("({x} {not}BETWEEN {lo} AND {hi})")
        }
        Expr::Like {
            expr,
            pattern,
            negated,
            case_insensitive,
        } => {
            let x = operand_sql(expr, p);
            let pat = p.str(pattern);
            let f = match (*negated, *case_insensitive) {
                (false, false) => "like",
                (true, false) => "notLike",
                (false, true) => "ilike",
                (true, true) => "notILike",
            };
            format!("{f}({x}, {pat})")
        }
        Expr::IsEmpty { expr, negated } => {
            let x = operand_sql(expr, p);
            let check = match expr.ty() {
                Ty::Str => format!("empty({x})"),
                Ty::Time => format!("(toUnixTimestamp64Milli({x}) = 0)"),
                Ty::Num | Ty::U64 | Ty::Bool => format!("({x} = 0)"),
            };
            if *negated {
                format!("NOT {check}")
            } else {
                check
            }
        }
        Expr::StartsWith(x, s) => {
            let (x, s) = (operand_sql(x, p), p.str(s));
            format!("startsWith({x}, {s})")
        }
        Expr::EndsWith(x, s) => {
            let (x, s) = (operand_sql(x, p), p.str(s));
            format!("endsWith({x}, {s})")
        }
        Expr::Match(x, re) => {
            let (x, re) = (operand_sql(x, p), p.str(re));
            format!("match({x}, {re})")
        }
    }
}

fn join(xs: &[Expr], sep: &str, p: &mut Params) -> String {
    let parts: Vec<String> = xs.iter().map(|x| expr_sql(x, p)).collect();
    format!("({})", parts.join(sep))
}

fn cmp_sql(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn operand_sql(o: &Operand, p: &mut Params) -> String {
    match o {
        // Registry names are static, lower-case identifiers (checked by a test).
        Operand::Column(c) => c.name.to_string(),
        Operand::MapKey { map, key } => {
            debug_assert_eq!(map.kind, Kind::Map);
            let k = p.str(key);
            format!("{}[{k}]", map.name)
        }
        Operand::Literal(l) => literal_sql(l, p),
        Operand::Now => "now64(3)".to_string(),
        Operand::TimeOffset {
            base,
            sign,
            interval,
        } => {
            let b = operand_sql(base, p);
            let n = p.int(interval.amount);
            let f = match interval.unit {
                Unit::Second => "toIntervalSecond",
                Unit::Minute => "toIntervalMinute",
                Unit::Hour => "toIntervalHour",
                Unit::Day => "toIntervalDay",
            };
            let s = if *sign == Sign::Plus { '+' } else { '-' };
            format!("({b} {s} {f}({n}))")
        }
        // Unicode-aware variants, matching the in-memory evaluator.
        Operand::Lower(a) => format!("lowerUTF8({})", operand_sql(a, p)),
        Operand::Upper(a) => format!("upperUTF8({})", operand_sql(a, p)),
        Operand::Length(a) => format!("lengthUTF8({})", operand_sql(a, p)),
        Operand::Position(h, n) => {
            let (h, n) = (operand_sql(h, p), operand_sql(n, p));
            format!("positionUTF8({h}, {n})")
        }
    }
}

fn literal_sql(l: &Literal, p: &mut Params) -> String {
    match l {
        Literal::Str(s) => p.str(s),
        Literal::Int(i) => p.int(*i),
        Literal::Float(f) => p.add("Float64", ParamValue::Float(*f)),
        Literal::Bool(b) => p.add("UInt8", ParamValue::UInt(u64::from(*b))),
        Literal::U64(u) => p.add("UInt64", ParamValue::UInt(*u)),
        Literal::Time(ms) => {
            let n = p.int(*ms);
            format!("fromUnixTimestamp64Milli({n})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Ctx, parse};

    fn sql(s: &str) -> Sql {
        compile(&parse(s, &Ctx::new(Table::Requests)).unwrap().unwrap())
    }

    #[test]
    fn literals_become_parameters() {
        let s = sql(
            "status >= 500 AND route LIKE '/checkout/%' AND attr['tenant'] IN ('acme', 'globex')",
        );
        assert_eq!(
            s.text,
            "((status >= {p1:Int64}) AND like(route, {p2:String}) AND (attributes[{p3:String}] IN ({p4:String}, {p5:String})))"
        );
        assert_eq!(s.params[0].value, ParamValue::Int(500));
        assert_eq!(s.params[2].value, ParamValue::Str("tenant".into()));
    }

    #[test]
    fn time_and_functions() {
        let s = sql("timestamp > now() - INTERVAL 15 MINUTE AND lower(method) = 'get'");
        assert_eq!(
            s.text,
            "((timestamp > (now64(3) - toIntervalMinute({p1:Int64}))) AND (lowerUTF8(method) = {p2:String}))"
        );
        let s = sql("timestamp >= '2026-01-01'");
        assert_eq!(
            s.text,
            "(timestamp >= fromUnixTimestamp64Milli({p1:Int64}))"
        );
        assert_eq!(s.params[0].value, ParamValue::Int(1_767_225_600_000));
    }

    #[test]
    fn empty_checks_and_negations() {
        assert_eq!(sql("user_id IS NULL").text, "empty(user_id)");
        assert_eq!(
            sql("attr['k'] IS NOT NULL").text,
            "NOT empty(attributes[{p1:String}])"
        );
        assert_eq!(sql("status IS NULL").text, "(status = 0)");
        assert_eq!(
            sql("NOT (status = 1 OR status = 2)").text,
            "NOT (((status = {p1:Int64}) OR (status = {p2:Int64})))"
        );
        assert_eq!(
            sql("route NOT ILIKE '%x%'").text,
            "notILike(route, {p1:String})"
        );
    }

    #[test]
    fn ids_and_booleans_keep_exact_types() {
        let s = sql("error_fingerprint = '18446744073709551615' AND body_truncated");
        assert_eq!(s.params[0].ty, "UInt64");
        assert_eq!(s.params[0].value, ParamValue::UInt(u64::MAX));
        assert_eq!(s.params[1].ty, "UInt8");
    }

    #[test]
    fn where_clause_wraps_project_and_time() {
        let e = parse("status >= 500", &Ctx::new(Table::Requests)).unwrap();
        let s = where_clause(Table::Requests, "proj", 10, 20, e.as_ref());
        assert_eq!(
            s.text,
            "project_id = {p1:String} AND timestamp >= fromUnixTimestamp64Milli({p2:Int64}) AND timestamp <= fromUnixTimestamp64Milli({p3:Int64}) AND ((status >= {p4:Int64}))"
        );
        let none = where_clause(Table::Requests, "proj", 10, 20, None);
        assert!(!none.text.contains("AND ("));
    }
}
