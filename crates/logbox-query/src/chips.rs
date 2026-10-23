//! Filter chips (`field op value ×`) ↔ AST.
//!
//! Chips can express a top-level AND of simple conditions. Anything else
//! (OR, NOT, functions, ranges on two sides) converts to `None`, and the UI
//! says "this query can only be edited as SQL".

use serde::{Deserialize, Serialize};

use crate::ast::{CmpOp, Expr, Literal, Operand, Ty, format_time};
use crate::parse::{Ctx, coerce_value, resolve_column};
use crate::registry::{Kind, map_alias};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChipOp {
    #[serde(rename = "=")]
    Eq,
    #[serde(rename = "!=")]
    Ne,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "not_in")]
    NotIn,
    /// Case-insensitive substring.
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "not_contains")]
    NotContains,
    #[serde(rename = "like")]
    Like,
    #[serde(rename = "not_like")]
    NotLike,
    #[serde(rename = "is_empty")]
    IsEmpty,
    #[serde(rename = "is_not_empty")]
    IsNotEmpty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChipValue {
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<ChipScalar>),
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChipScalar {
    Bool(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    /// Column name, or a map column (`attributes`, `correlation`) with `key`.
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub op: ChipOp,
    #[serde(default = "none")]
    pub value: ChipValue,
}

fn none() -> ChipValue {
    ChipValue::None
}

/// An error tied to one chip.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChipError {
    pub index: usize,
    pub message: String,
}

pub fn chips_to_expr(chips: &[Chip], ctx: &Ctx) -> Result<Option<Expr>, ChipError> {
    let mut items = Vec::with_capacity(chips.len());
    for (index, c) in chips.iter().enumerate() {
        items.push(chip_expr(c, ctx).map_err(|message| ChipError { index, message })?);
    }
    Ok(match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Expr::And(items)),
    })
}

fn operand(c: &Chip, ctx: &Ctx) -> Result<Operand, String> {
    if let Some(key) = &c.key {
        let name = map_alias(&c.field).ok_or_else(|| format!("`{}` is not a map", c.field))?;
        let col = resolve_column(ctx, name)?;
        if key.is_empty() || key.chars().count() > 128 {
            return Err("map keys must be 1 to 128 characters".into());
        }
        return Ok(Operand::MapKey {
            map: col,
            key: key.clone(),
        });
    }
    let col = resolve_column(ctx, &c.field)?;
    if col.kind == Kind::Map {
        return Err(format!("`{}` needs a key", c.field));
    }
    Ok(Operand::Column(col))
}

fn scalar_literal(v: &ChipScalar, ty: Ty) -> Result<Literal, String> {
    let lit = match v {
        ChipScalar::Bool(b) => Literal::Bool(*b),
        ChipScalar::Number(n) => number(*n),
        ChipScalar::Text(s) => match ty {
            Ty::Num => s
                .trim()
                .parse::<f64>()
                .map(number)
                .map_err(|_| format!("`{s}` is not a number"))?,
            Ty::Bool => match s.as_str() {
                "true" => Literal::Bool(true),
                "false" => Literal::Bool(false),
                _ => return Err(format!("`{s}` is not true or false")),
            },
            _ => Literal::Str(s.clone()),
        },
    };
    coerce_value(lit, ty)
}

fn number(n: f64) -> Literal {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        Literal::Int(n as i64)
    } else {
        Literal::Float(n)
    }
}

fn single(v: &ChipValue) -> Result<ChipScalar, String> {
    match v {
        ChipValue::Bool(b) => Ok(ChipScalar::Bool(*b)),
        ChipValue::Number(n) => Ok(ChipScalar::Number(*n)),
        ChipValue::Text(s) => Ok(ChipScalar::Text(s.clone())),
        ChipValue::List(_) => Err("expected a single value".into()),
        ChipValue::None => Err("a value is required".into()),
    }
}

fn escape_like(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn chip_expr(c: &Chip, ctx: &Ctx) -> Result<Expr, String> {
    let op = operand(c, ctx)?;
    let ty = op.ty();
    let cmp = |o: CmpOp| -> Result<Expr, String> {
        let lit = scalar_literal(&single(&c.value)?, ty)?;
        if ty == Ty::Bool && !matches!(o, CmpOp::Eq | CmpOp::Ne) {
            return Err("booleans can only be compared with = or !=".into());
        }
        Ok(Expr::Compare {
            left: op.clone(),
            op: o,
            right: Operand::Literal(lit),
        })
    };
    let text = || -> Result<String, String> {
        if ty != Ty::Str {
            return Err(format!("`{}` is not a text field", c.field));
        }
        match single(&c.value)? {
            ChipScalar::Text(s) => Ok(s),
            ChipScalar::Number(n) => Ok(n.to_string()),
            ChipScalar::Bool(b) => Ok(b.to_string()),
        }
    };
    match c.op {
        ChipOp::Eq => cmp(CmpOp::Eq),
        ChipOp::Ne => cmp(CmpOp::Ne),
        ChipOp::Gt => cmp(CmpOp::Gt),
        ChipOp::Ge => cmp(CmpOp::Ge),
        ChipOp::Lt => cmp(CmpOp::Lt),
        ChipOp::Le => cmp(CmpOp::Le),
        ChipOp::In | ChipOp::NotIn => {
            let values: Vec<ChipScalar> = match &c.value {
                ChipValue::List(v) => v.clone(),
                other => vec![single(other)?],
            };
            if values.is_empty() || values.len() > crate::parse::MAX_IN_LIST {
                return Err("in needs 1 to 500 values".into());
            }
            let list = values
                .iter()
                .map(|v| scalar_literal(v, ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::In {
                expr: op,
                list,
                negated: c.op == ChipOp::NotIn,
            })
        }
        ChipOp::Contains | ChipOp::NotContains => Ok(Expr::Like {
            pattern: format!("%{}%", escape_like(&text()?)),
            expr: op,
            negated: c.op == ChipOp::NotContains,
            case_insensitive: true,
        }),
        ChipOp::Like | ChipOp::NotLike => Ok(Expr::Like {
            pattern: {
                let p = text()?;
                crate::parse::check_like_pattern(&p)?;
                p
            },
            expr: op,
            negated: c.op == ChipOp::NotLike,
            case_insensitive: false,
        }),
        ChipOp::IsEmpty | ChipOp::IsNotEmpty => Ok(Expr::IsEmpty {
            expr: op,
            negated: c.op == ChipOp::IsNotEmpty,
        }),
    }
}

/// Converts an AST to chips when it's a top-level AND of simple conditions.
pub fn expr_to_chips(expr: &Expr) -> Option<Vec<Chip>> {
    let items: Vec<&Expr> = match expr {
        Expr::And(xs) => xs.iter().collect(),
        other => vec![other],
    };
    items.into_iter().map(to_chip).collect()
}

fn field_of(o: &Operand) -> Option<(String, Option<String>)> {
    match o {
        Operand::Column(c) => Some((c.name.to_string(), None)),
        Operand::MapKey { map, key } => Some((map.name.to_string(), Some(key.clone()))),
        _ => None,
    }
}

fn chip_scalar(l: &Literal) -> ChipScalar {
    match l {
        Literal::Str(s) => ChipScalar::Text(s.clone()),
        Literal::Int(i) => ChipScalar::Number(*i as f64),
        Literal::Float(f) => ChipScalar::Number(*f),
        Literal::Bool(b) => ChipScalar::Bool(*b),
        Literal::Time(ms) => ChipScalar::Text(format_time(*ms)),
        Literal::U64(u) => ChipScalar::Text(u.to_string()),
    }
}

fn chip_value(l: &Literal) -> ChipValue {
    match chip_scalar(l) {
        ChipScalar::Text(s) => ChipValue::Text(s),
        ChipScalar::Number(n) => ChipValue::Number(n),
        ChipScalar::Bool(b) => ChipValue::Bool(b),
    }
}

/// `%text%` with no other wildcards, unescaped.
fn contains_text(pattern: &str) -> Option<String> {
    let inner = pattern.strip_prefix('%')?.strip_suffix('%')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '%' | '_' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

fn to_chip(e: &Expr) -> Option<Chip> {
    match e {
        Expr::Compare { left, op, right } => {
            let (operand, op, lit) = match (left, right) {
                (o, Operand::Literal(l)) => (o, *op, l),
                (Operand::Literal(l), o) => (o, op.flipped(), l),
                _ => return None,
            };
            let (field, key) = field_of(operand)?;
            let op = match op {
                CmpOp::Eq => ChipOp::Eq,
                CmpOp::Ne => ChipOp::Ne,
                CmpOp::Gt => ChipOp::Gt,
                CmpOp::Ge => ChipOp::Ge,
                CmpOp::Lt => ChipOp::Lt,
                CmpOp::Le => ChipOp::Le,
            };
            Some(Chip {
                field,
                key,
                op,
                value: chip_value(lit),
            })
        }
        Expr::In {
            expr,
            list,
            negated,
        } => {
            let (field, key) = field_of(expr)?;
            Some(Chip {
                field,
                key,
                op: if *negated { ChipOp::NotIn } else { ChipOp::In },
                value: ChipValue::List(list.iter().map(chip_scalar).collect()),
            })
        }
        Expr::Like {
            expr,
            pattern,
            negated,
            case_insensitive,
        } => {
            let (field, key) = field_of(expr)?;
            if *case_insensitive {
                let text = contains_text(pattern)?;
                return Some(Chip {
                    field,
                    key,
                    op: if *negated {
                        ChipOp::NotContains
                    } else {
                        ChipOp::Contains
                    },
                    value: ChipValue::Text(text),
                });
            }
            Some(Chip {
                field,
                key,
                op: if *negated {
                    ChipOp::NotLike
                } else {
                    ChipOp::Like
                },
                value: ChipValue::Text(pattern.clone()),
            })
        }
        Expr::IsEmpty { expr, negated } => {
            let (field, key) = field_of(expr)?;
            Some(Chip {
                field,
                key,
                op: if *negated {
                    ChipOp::IsNotEmpty
                } else {
                    ChipOp::IsEmpty
                },
                value: ChipValue::None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::registry::Table;

    fn ctx() -> Ctx {
        Ctx::new(Table::Requests)
    }

    #[test]
    fn round_trips_simple_queries() {
        for q in [
            "status >= 500 AND route LIKE '/checkout/%' AND attr['tenant'] IN ('acme', 'globex')",
            "client NOT IN ('ios') AND user_id IS NOT NULL",
            "error_message ILIKE '%time\\\\_out%'",
            "500 <= status",
            "body_truncated = true AND error_fingerprint = 42 AND timestamp >= '2026-01-01'",
        ] {
            let e = parse(q, &ctx()).unwrap().unwrap();
            let chips = expr_to_chips(&e).unwrap_or_else(|| panic!("{q}"));
            let json = serde_json::to_string(&chips).unwrap();
            let back: Vec<Chip> = serde_json::from_str(&json).unwrap();
            let rebuilt = chips_to_expr(&back, &ctx()).unwrap().unwrap();
            let normalized = parse(&rebuilt.to_string(), &ctx()).unwrap().unwrap();
            assert_eq!(expr_to_chips(&normalized), Some(chips), "{q} -> {json}");
        }
    }

    #[test]
    fn complex_queries_are_sql_only() {
        for q in [
            "status = 1 OR status = 2",
            "NOT status = 1",
            "lower(method) = 'get'",
            "startsWith(path, '/a')",
        ] {
            assert_eq!(
                expr_to_chips(&parse(q, &ctx()).unwrap().unwrap()),
                None,
                "{q}"
            );
        }
    }

    #[test]
    fn chip_json_shape_and_errors() {
        let chips: Vec<Chip> = serde_json::from_str(
            r#"[{"field":"status","op":">=","value":500},
                {"field":"attributes","key":"tenant","op":"in","value":["acme","globex"]},
                {"field":"path","op":"contains","value":"50%"},
                {"field":"user_id","op":"is_empty"}]"#,
        )
        .unwrap();
        let e = chips_to_expr(&chips, &ctx()).unwrap().unwrap();
        assert_eq!(
            e.to_string(),
            "status >= 500 AND attr['tenant'] IN ('acme', 'globex') AND path ILIKE '%50\\\\%%' AND user_id IS NULL"
        );
        let bad: Vec<Chip> = serde_json::from_str(
            r#"[{"field":"status","op":"=","value":200},{"field":"nope","op":"=","value":1}]"#,
        )
        .unwrap();
        let err = chips_to_expr(&bad, &ctx()).unwrap_err();
        assert_eq!(err.index, 1);
        assert!(err.message.contains("unknown column"));
    }
}
