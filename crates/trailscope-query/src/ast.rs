//! Trailscope's own small, typed AST. User text is converted into this, and only
//! this is compiled to SQL, evaluated in memory or turned into chips.

use std::fmt;

use crate::registry::{Column, Kind};

/// The value type an operand produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Str,
    Num,
    U64,
    Bool,
    Time,
}

impl Ty {
    pub fn of_kind(kind: Kind) -> Option<Ty> {
        match kind {
            Kind::String => Some(Ty::Str),
            Kind::Number => Some(Ty::Num),
            Kind::U64 => Some(Ty::U64),
            Kind::Bool => Some(Ty::Bool),
            Kind::Time => Some(Ty::Time),
            Kind::Map => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Ty::Str => "a string",
            Ty::Num => "a number",
            Ty::U64 => "an ID",
            Ty::Bool => "a boolean",
            Ty::Time => "a timestamp",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Milliseconds since the epoch (from a datetime string).
    Time(i64),
    U64(u64),
}

impl Literal {
    pub fn ty(&self) -> Ty {
        match self {
            Literal::Str(_) => Ty::Str,
            Literal::Int(_) | Literal::Float(_) => Ty::Num,
            Literal::Bool(_) => Ty::Bool,
            Literal::Time(_) => Ty::Time,
            Literal::U64(_) => Ty::U64,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Literal::Int(i) => Some(*i as f64),
            Literal::Float(f) => Some(*f),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
}

impl Unit {
    pub fn millis(self) -> i64 {
        match self {
            Unit::Second => 1_000,
            Unit::Minute => 60_000,
            Unit::Hour => 3_600_000,
            Unit::Day => 86_400_000,
        }
    }

    pub fn keyword(self) -> &'static str {
        match self {
            Unit::Second => "SECOND",
            Unit::Minute => "MINUTE",
            Unit::Hour => "HOUR",
            Unit::Day => "DAY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub amount: i64,
    pub unit: Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Plus,
    Minus,
}

/// Something that produces a value.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Column(&'static Column),
    /// `attributes['key']`, `correlation['key']`, `req_headers['key']`.
    MapKey {
        map: &'static Column,
        key: String,
    },
    Literal(Literal),
    Now,
    /// `now() - INTERVAL 15 MINUTE`
    TimeOffset {
        base: Box<Operand>,
        sign: Sign,
        interval: Interval,
    },
    Lower(Box<Operand>),
    Upper(Box<Operand>),
    Length(Box<Operand>),
    /// `position(haystack, needle)`
    Position(Box<Operand>, Box<Operand>),
}

impl Operand {
    pub fn ty(&self) -> Ty {
        match self {
            Operand::Column(c) => Ty::of_kind(c.kind).unwrap_or(Ty::Str),
            Operand::MapKey { .. } | Operand::Lower(_) | Operand::Upper(_) => Ty::Str,
            Operand::Literal(l) => l.ty(),
            Operand::Now | Operand::TimeOffset { .. } => Ty::Time,
            Operand::Length(_) | Operand::Position(..) => Ty::Num,
        }
    }

    /// Whether the operand reads row data (as opposed to constants).
    pub fn references_row(&self) -> bool {
        match self {
            Operand::Column(_) | Operand::MapKey { .. } => true,
            Operand::Literal(_) | Operand::Now => false,
            Operand::TimeOffset { base, .. } => base.references_row(),
            Operand::Lower(a) | Operand::Upper(a) | Operand::Length(a) => a.references_row(),
            Operand::Position(a, b) => a.references_row() || b.references_row(),
        }
    }

    /// Visits every column the operand reads.
    pub fn columns(&self, out: &mut Vec<&'static Column>) {
        match self {
            Operand::Column(c) => out.push(c),
            Operand::MapKey { map, .. } => out.push(map),
            Operand::Literal(_) | Operand::Now => {}
            Operand::TimeOffset { base, .. } => base.columns(out),
            Operand::Lower(a) | Operand::Upper(a) | Operand::Length(a) => a.columns(out),
            Operand::Position(a, b) => {
                a.columns(out);
                b.columns(out);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    /// The operator with sides swapped: `5 < x` is `x > 5`.
    pub fn flipped(self) -> CmpOp {
        match self {
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
            other => other,
        }
    }
}

/// A boolean condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Compare {
        left: Operand,
        op: CmpOp,
        right: Operand,
    },
    In {
        expr: Operand,
        list: Vec<Literal>,
        negated: bool,
    },
    Between {
        expr: Operand,
        low: Operand,
        high: Operand,
        negated: bool,
    },
    Like {
        expr: Operand,
        pattern: String,
        negated: bool,
        case_insensitive: bool,
    },
    /// `IS NULL` (empty value) / `IS NOT NULL`.
    IsEmpty {
        expr: Operand,
        negated: bool,
    },
    StartsWith(Operand, String),
    EndsWith(Operand, String),
    /// `match(s, 'regex')`
    Match(Operand, String),
}

impl Expr {
    /// Every column the expression reads.
    pub fn columns(&self) -> Vec<&'static Column> {
        let mut out = Vec::new();
        self.visit_columns(&mut out);
        out
    }

    fn visit_columns(&self, out: &mut Vec<&'static Column>) {
        match self {
            Expr::And(xs) | Expr::Or(xs) => xs.iter().for_each(|x| x.visit_columns(out)),
            Expr::Not(x) => x.visit_columns(out),
            Expr::Compare { left, right, .. } => {
                left.columns(out);
                right.columns(out);
            }
            Expr::In { expr, .. }
            | Expr::Like { expr, .. }
            | Expr::IsEmpty { expr, .. }
            | Expr::StartsWith(expr, _)
            | Expr::EndsWith(expr, _)
            | Expr::Match(expr, _) => expr.columns(out),
            Expr::Between {
                expr, low, high, ..
            } => {
                expr.columns(out);
                low.columns(out);
                high.columns(out);
            }
        }
    }

    /// Number of top-level conditions (for "5 conditions" in the status line).
    pub fn condition_count(&self) -> usize {
        match self {
            Expr::And(xs) | Expr::Or(xs) => xs.iter().map(Expr::condition_count).sum(),
            Expr::Not(x) => x.condition_count(),
            _ => 1,
        }
    }
}

// ---- printing back to SQL text ---------------------------------------------

/// Quotes a string literal so the parser reads it back unchanged.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

pub fn format_time(ms: i64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Str(s) => f.write_str(&quote(s)),
            Literal::Int(i) => write!(f, "{i}"),
            Literal::Float(x) => {
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    write!(f, "{x:.1}")
                } else {
                    write!(f, "{x}")
                }
            }
            Literal::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Literal::Time(ms) => f.write_str(&quote(&format_time(*ms))),
            Literal::U64(u) => write!(f, "{u}"),
        }
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Column(c) => f.write_str(c.name),
            Operand::MapKey { map, key } => {
                let name = if map.name == "attributes" {
                    "attr"
                } else {
                    map.name
                };
                write!(f, "{name}[{}]", quote(key))
            }
            Operand::Literal(l) => {
                // A negative number on the right of `-` would read as `- -1`.
                write!(f, "{l}")
            }
            Operand::Now => f.write_str("now()"),
            Operand::TimeOffset {
                base,
                sign,
                interval,
            } => {
                let s = if *sign == Sign::Plus { '+' } else { '-' };
                write!(
                    f,
                    "{base} {s} INTERVAL {} {}",
                    interval.amount,
                    interval.unit.keyword()
                )
            }
            Operand::Lower(a) => write!(f, "lower({a})"),
            Operand::Upper(a) => write!(f, "upper({a})"),
            Operand::Length(a) => write!(f, "length({a})"),
            Operand::Position(a, b) => write!(f, "position({a}, {b})"),
        }
    }
}

fn write_list(f: &mut fmt::Formatter<'_>, items: &[Literal]) -> fmt::Result {
    f.write_str("(")?;
    for (i, l) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{l}")?;
    }
    f.write_str(")")
}

fn write_child(f: &mut fmt::Formatter<'_>, child: &Expr, parent_is_and: bool) -> fmt::Result {
    let needs_parens = match child {
        Expr::Or(_) => parent_is_and,
        Expr::And(_) => !parent_is_and,
        _ => false,
    };
    if needs_parens {
        write!(f, "({child})")
    } else {
        write!(f, "{child}")
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::And(xs) | Expr::Or(xs) => {
                let is_and = matches!(self, Expr::And(_));
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(if is_and { " AND " } else { " OR " })?;
                    }
                    write_child(f, x, is_and)?;
                }
                Ok(())
            }
            Expr::Not(x) => match **x {
                Expr::And(_) | Expr::Or(_) | Expr::Not(_) => write!(f, "NOT ({x})"),
                _ => write!(f, "NOT {x}"),
            },
            Expr::Compare { left, op, right } => write!(f, "{left} {} {right}", op.symbol()),
            Expr::In {
                expr,
                list,
                negated,
            } => {
                write!(f, "{expr} {}IN ", if *negated { "NOT " } else { "" })?;
                write_list(f, list)
            }
            Expr::Between {
                expr,
                low,
                high,
                negated,
            } => write!(
                f,
                "{expr} {}BETWEEN {low} AND {high}",
                if *negated { "NOT " } else { "" }
            ),
            Expr::Like {
                expr,
                pattern,
                negated,
                case_insensitive,
            } => write!(
                f,
                "{expr} {}{} {}",
                if *negated { "NOT " } else { "" },
                if *case_insensitive { "ILIKE" } else { "LIKE" },
                quote(pattern)
            ),
            Expr::IsEmpty { expr, negated } => {
                write!(f, "{expr} IS {}NULL", if *negated { "NOT " } else { "" })
            }
            Expr::StartsWith(a, s) => write!(f, "startsWith({a}, {})", quote(s)),
            Expr::EndsWith(a, s) => write!(f, "endsWith({a}, {})", quote(s)),
            Expr::Match(a, s) => write!(f, "match({a}, {})", quote(s)),
        }
    }
}
