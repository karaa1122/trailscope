//! Search syntax → the same AST as SQL mode.
//!
//! ```text
//! status:>=500 route:/checkout/* user:u_123 -method:GET client:ios,android "free text"
//! status:5xx duration:>800ms attr.tenant:acme correlation.account_id:acc_1
//! ```
//!
//! - `field:value` is equality; `field:>N`, `>=`, `<`, `<=`, `!value` compare.
//! - `*` in a value turns it into a LIKE pattern; `a,b,c` is an IN list.
//! - `-field:value` negates. `NxX` (e.g. `5xx`) on `status` is a range.
//! - Bare words and `"quoted text"` match the table's free-text columns
//!   (`path` and `error_message` for requests; `name`, `message` and
//!   `error_message` for events), case-insensitively.
//! - `attr.key` / `attr[key]` and `correlation.key` read map values.

use crate::ast::{CmpOp, Expr, Literal, Operand};
use crate::error::{QueryError, Result};
use crate::parse::{Ctx, coerce_value, resolve_column};
use crate::registry::{Kind, map_alias};

const ALIASES: &[(&str, &str)] = &[
    ("user", "user_id"),
    ("env", "environment"),
    ("svc", "service"),
    ("type", "error_type"),
    ("error", "error_type"),
    ("duration", "duration_ms"),
    ("latency", "duration_ms"),
    ("fingerprint", "error_fingerprint"),
    ("fp", "error_fingerprint"),
    ("version", "client_version"),
    ("ua", "user_agent"),
    ("request", "request_id"),
    ("trace", "trace_id"),
];

struct Term<'a> {
    negated: bool,
    field: Option<&'a str>,
    value: String,
    quoted: bool,
    col: u32,
    len: u32,
}

/// Parses search syntax. `Ok(None)` means an empty search.
pub fn parse_search(text: &str, ctx: &Ctx) -> Result<Option<Expr>> {
    if text.chars().count() > crate::parse::MAX_CHARS {
        return Err(QueryError::at_start("the search is too long"));
    }
    let terms = tokenize(text)?;
    let mut items = Vec::new();
    for t in &terms {
        let e = term(t, ctx).map_err(|m| QueryError::new(m, 1, t.col, t.len))?;
        items.push(if t.negated { negate(e) } else { e });
    }
    if items.len() > crate::parse::MAX_NODES / 4 {
        return Err(QueryError::at_start("the search has too many terms"));
    }
    Ok(match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Expr::And(items)),
    })
}

fn negate(e: Expr) -> Expr {
    match e {
        Expr::Compare {
            left,
            op: CmpOp::Eq,
            right,
        } => Expr::Compare {
            left,
            op: CmpOp::Ne,
            right,
        },
        Expr::In {
            expr,
            list,
            negated,
        } => Expr::In {
            expr,
            list,
            negated: !negated,
        },
        Expr::Like {
            expr,
            pattern,
            negated,
            case_insensitive,
        } => Expr::Like {
            expr,
            pattern,
            negated: !negated,
            case_insensitive,
        },
        other => Expr::Not(Box::new(other)),
    }
}

fn tokenize(text: &str) -> Result<Vec<Term<'_>>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut terms = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].1.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let negated = chars[i].1 == '-' && i + 1 < chars.len() && !chars[i + 1].1.is_whitespace();
        if negated {
            i += 1;
        }
        // field:
        let mut field = None;
        let field_start = i;
        let mut j = i;
        while j < chars.len()
            && !chars[j].1.is_whitespace()
            && chars[j].1 != '"'
            && chars[j].1 != ':'
        {
            j += 1;
        }
        if j < chars.len() && chars[j].1 == ':' && j > field_start {
            let (from, to) = (chars[field_start].0, chars[j].0);
            field = Some(&text[from..to]);
            i = j + 1;
        }
        // value: quoted or bare
        let (value, quoted) = if i < chars.len() && chars[i].1 == '"' {
            let mut v = String::new();
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                match chars[i].1 {
                    '\\' if i + 1 < chars.len() => {
                        v.push(chars[i + 1].1);
                        i += 2;
                    }
                    '"' => {
                        closed = true;
                        i += 1;
                        break;
                    }
                    c => {
                        v.push(c);
                        i += 1;
                    }
                }
            }
            if !closed {
                return Err(QueryError::new("unclosed quote", 1, start as u32 + 1, 1));
            }
            (v, true)
        } else {
            let mut v = String::new();
            while i < chars.len() && !chars[i].1.is_whitespace() {
                v.push(chars[i].1);
                i += 1;
            }
            (v, false)
        };
        let len = (i - start) as u32;
        let col = text[..chars[start].0].chars().count() as u32 + 1;
        if field.is_some() && value.is_empty() && !quoted {
            return Err(QueryError::new("missing value after `:`", 1, col, len));
        }
        terms.push(Term {
            negated,
            field,
            value,
            quoted,
            col,
            len,
        });
    }
    Ok(terms)
}

/// Escapes LIKE wildcards in literal text.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn term(t: &Term<'_>, ctx: &Ctx) -> std::result::Result<Expr, String> {
    let Some(field) = t.field else {
        return free_text(&t.value, ctx);
    };
    let operand = field_operand(field, ctx)?;
    let ty = operand.ty();
    let mut v = t.value.as_str();
    if !t.quoted {
        // Ranges like status:5xx
        if field_is(&operand, "status") && v.len() == 3 && v.ends_with("xx") {
            let d = v[..1]
                .parse::<i64>()
                .map_err(|_| format!("`{v}` is not a status class"))?;
            return Ok(Expr::Between {
                expr: operand,
                low: Operand::Literal(Literal::Int(d * 100)),
                high: Operand::Literal(Literal::Int(d * 100 + 99)),
                negated: false,
            });
        }
        for (prefix, op) in [
            (">=", CmpOp::Ge),
            ("<=", CmpOp::Le),
            (">", CmpOp::Gt),
            ("<", CmpOp::Lt),
            ("!", CmpOp::Ne),
        ] {
            if let Some(rest) = v.strip_prefix(prefix) {
                let lit = typed_literal(rest, ty, &operand)?;
                return Ok(Expr::Compare {
                    left: operand,
                    op,
                    right: Operand::Literal(lit),
                });
            }
        }
        if v.contains(',') {
            let list = v
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| typed_literal(s, ty, &operand))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if list.len() > crate::parse::MAX_IN_LIST {
                return Err("too many values".into());
            }
            return Ok(Expr::In {
                expr: operand,
                list,
                negated: false,
            });
        }
        if v.contains('*') {
            if ty != crate::ast::Ty::Str {
                return Err(format!("`*` only works on text fields, not on `{field}`"));
            }
            let pattern: String = v.split('*').map(escape_like).collect::<Vec<_>>().join("%");
            return Ok(Expr::Like {
                expr: operand,
                pattern,
                negated: false,
                case_insensitive: false,
            });
        }
        v = v.strip_prefix('=').unwrap_or(v);
    }
    let lit = typed_literal(v, ty, &operand)?;
    Ok(Expr::Compare {
        left: operand,
        op: CmpOp::Eq,
        right: Operand::Literal(lit),
    })
}

fn field_is(op: &Operand, name: &str) -> bool {
    matches!(op, Operand::Column(c) if c.name == name)
}

fn field_operand(field: &str, ctx: &Ctx) -> std::result::Result<Operand, String> {
    // attr.key, attr[key], correlation.key
    let split = field
        .split_once('.')
        .or_else(|| field.strip_suffix(']').and_then(|f| f.split_once('[')));
    if let Some((map, key)) = split
        && let Some(name) = map_alias(map)
    {
        let col = resolve_column(ctx, name)?;
        let key = key.trim_matches(|c| c == '\'' || c == '"');
        if key.is_empty() {
            return Err(format!("`{field}` needs a key, e.g. {map}.tenant"));
        }
        return Ok(Operand::MapKey {
            map: col,
            key: key.to_string(),
        });
    }
    let lower = field.to_ascii_lowercase();
    let name = ALIASES
        .iter()
        .find(|(a, _)| *a == lower)
        .map_or(lower.as_str(), |(_, c)| *c);
    let col = resolve_column(ctx, name)?;
    if col.kind == Kind::Map {
        return Err(format!("`{field}` is a map; use {field}.key:value"));
    }
    Ok(Operand::Column(col))
}

fn typed_literal(
    raw: &str,
    ty: crate::ast::Ty,
    operand: &Operand,
) -> std::result::Result<Literal, String> {
    use crate::ast::Ty;
    let lit = match ty {
        Ty::Num => {
            let is_duration = matches!(operand, Operand::Column(c) if c.name == "duration_ms");
            parse_num(raw, is_duration).ok_or_else(|| format!("`{raw}` is not a number"))?
        }
        Ty::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Literal::Bool(true),
            "false" | "no" | "0" => Literal::Bool(false),
            _ => return Err(format!("`{raw}` is not true or false")),
        },
        _ => Literal::Str(raw.to_string()),
    };
    coerce_value(lit, ty)
}

/// Numbers, with `ms`/`s`/`m` suffixes for durations (`>1.5s`).
fn parse_num(raw: &str, duration: bool) -> Option<Literal> {
    let (num, mult) = if duration {
        if let Some(n) = raw.strip_suffix("ms") {
            (n, 1.0)
        } else if let Some(n) = raw.strip_suffix('s') {
            (n, 1000.0)
        } else if let Some(n) = raw.strip_suffix('m') {
            (n, 60_000.0)
        } else {
            (raw, 1.0)
        }
    } else {
        (raw, 1.0)
    };
    if mult == 1.0
        && let Ok(i) = num.parse::<i64>()
    {
        return Some(Literal::Int(i));
    }
    let f = num.parse::<f64>().ok().filter(|f| f.is_finite())? * mult;
    if f.fract() == 0.0 && f.abs() < 9e15 {
        Some(Literal::Int(f as i64))
    } else {
        Some(Literal::Float(f))
    }
}

fn free_text(text: &str, ctx: &Ctx) -> std::result::Result<Expr, String> {
    if text.is_empty() {
        return Err("empty search term".into());
    }
    let pattern = format!("%{}%", escape_like(text));
    let items: Vec<Expr> = ctx
        .table
        .free_text_columns()
        .filter(|c| resolve_column(ctx, c.name).is_ok())
        .map(|c| Expr::Like {
            expr: Operand::Column(c),
            pattern: pattern.clone(),
            negated: false,
            case_insensitive: true,
        })
        .collect();
    match items.len() {
        0 => Err("free-text search isn't available here".into()),
        1 => Ok(items.into_iter().next().unwrap_or(Expr::And(vec![]))),
        _ => Ok(Expr::Or(items)),
    }
}

/// Renders an AST as search syntax when it is a plain AND of simple terms.
pub fn to_search(expr: &Expr, ctx: &Ctx) -> Option<String> {
    let items: Vec<&Expr> = match expr {
        Expr::And(xs) => xs.iter().collect(),
        other => vec![other],
    };
    let mut out = Vec::new();
    for e in items {
        out.push(search_term(e, ctx)?);
    }
    Some(out.join(" "))
}

fn field_name(o: &Operand) -> Option<String> {
    match o {
        Operand::Column(c) => Some(c.name.to_string()),
        Operand::MapKey { map, key }
            if key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-') =>
        {
            let m = if map.name == "attributes" {
                "attr"
            } else {
                map.name
            };
            Some(format!("{m}.{key}"))
        }
        _ => None,
    }
}

/// The value as written after `field:`. Strings that would be misread are
/// quoted; `quotable` is false after an operator prefix like `>`, where a
/// quoted value can't follow, so such values aren't representable.
fn search_value(l: &Literal, quotable: bool) -> Option<String> {
    let raw = match l {
        Literal::Str(s) => s.clone(),
        Literal::Time(ms) => crate::ast::format_time(*ms),
        // Numbers, booleans and IDs never need quoting.
        other => return Some(other.to_string()),
    };
    let plain = !raw.is_empty()
        && raw
            .chars()
            .all(|c| !c.is_whitespace() && !matches!(c, '"' | ',' | '*' | '\\'))
        && !raw.starts_with(['>', '<', '!', '=']);
    if plain {
        Some(raw)
    } else if quotable {
        Some(format!(
            "\"{}\"",
            raw.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    } else {
        None
    }
}

fn search_term(e: &Expr, ctx: &Ctx) -> Option<String> {
    match e {
        Expr::Compare {
            left,
            op,
            right: Operand::Literal(l),
        } => {
            let f = field_name(left)?;
            let v = search_value(l, matches!(op, CmpOp::Eq | CmpOp::Ne))?;
            Some(match op {
                CmpOp::Eq => format!("{f}:{v}"),
                CmpOp::Ne => format!("-{f}:{v}"),
                CmpOp::Gt => format!("{f}:>{v}"),
                CmpOp::Ge => format!("{f}:>={v}"),
                CmpOp::Lt => format!("{f}:<{v}"),
                CmpOp::Le => format!("{f}:<={v}"),
            })
        }
        Expr::In {
            expr,
            list,
            negated,
        } => {
            let f = field_name(expr)?;
            let vals = list
                .iter()
                .map(|l| search_value(l, false))
                .collect::<Option<Vec<String>>>()?;
            if list.len() < 2 {
                return None;
            }
            Some(format!(
                "{}{f}:{}",
                if *negated { "-" } else { "" },
                vals.join(",")
            ))
        }
        Expr::Like {
            expr,
            pattern,
            negated,
            case_insensitive: false,
        } => {
            let f = field_name(expr)?;
            // Search syntax has no empty value, and a value without `*` reads
            // back as `=`, so those patterns stay in SQL form.
            if !pattern.contains('%') || pattern.contains(['_', '\\', '*', '"', ' ']) {
                return None;
            }
            Some(format!(
                "{}{f}:{}",
                if *negated { "-" } else { "" },
                pattern.replace('%', "*")
            ))
        }
        Expr::Between {
            expr,
            low: Operand::Literal(Literal::Int(lo)),
            high: Operand::Literal(Literal::Int(hi)),
            negated: false,
        } if field_name(expr).as_deref() == Some("status") && lo % 100 == 0 && *hi == lo + 99 => {
            Some(format!("status:{}xx", lo / 100))
        }
        Expr::Or(xs) => {
            // Free text: ILIKE '%t%' over exactly the free-text columns.
            let mut text = None;
            let cols: Vec<&str> = ctx.table.free_text_columns().map(|c| c.name).collect();
            if xs.len() != cols.len() {
                return None;
            }
            for (x, want) in xs.iter().zip(&cols) {
                let Expr::Like {
                    expr: Operand::Column(c),
                    pattern,
                    negated: false,
                    case_insensitive: true,
                } = x
                else {
                    return None;
                };
                if c.name != *want {
                    return None;
                }
                let inner = pattern.strip_prefix('%')?.strip_suffix('%')?;
                if text.is_some_and(|t: &str| t != inner) {
                    return None;
                }
                text = Some(inner);
            }
            let t = text?;
            if t.contains(['%', '_', '\\']) {
                return None;
            }
            Some(if t.contains([' ', '"', ':']) || t.starts_with('-') {
                format!("\"{}\"", t.replace('"', "\\\""))
            } else {
                t.to_string()
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

    fn search(s: &str) -> Expr {
        parse_search(s, &ctx()).unwrap().unwrap()
    }

    fn sql(s: &str) -> Expr {
        parse(s, &ctx()).unwrap().unwrap()
    }

    #[test]
    fn spec_example() {
        assert_eq!(
            search(r#"status:>=500 route:/checkout/* user:u_123 "free text""#).to_string(),
            "status >= 500 AND route LIKE '/checkout/%' AND user_id = 'u_123' AND (path ILIKE '%free text%' OR error_message ILIKE '%free text%')"
        );
    }

    #[test]
    fn same_ast_as_sql() {
        let pairs = [
            ("status:503", "status = 503"),
            ("-method:GET", "method != 'GET'"),
            ("client:ios,android", "client IN ('ios', 'android')"),
            ("-client:ios,android", "client NOT IN ('ios', 'android')"),
            ("status:5xx", "status BETWEEN 500 AND 599"),
            ("duration:>1.5s", "duration_ms > 1500"),
            ("duration:<=800ms", "duration_ms <= 800"),
            ("attr.tenant:acme", "attr['tenant'] = 'acme'"),
            ("attr[tenant]:acme", "attr['tenant'] = 'acme'"),
            (
                "correlation.account_id:acc_1",
                "correlation['account_id'] = 'acc_1'",
            ),
            (
                "type:TimeoutError env:production",
                "error_type = 'TimeoutError' AND environment = 'production'",
            ),
            ("timestamp:>2026-01-02", "timestamp > '2026-01-02'"),
            ("status:!200", "status != 200"),
            (r#"route:"/a b""#, "route = '/a b'"),
            ("route:/v1/*/items", "route LIKE '/v1/%/items'"),
            ("route:/a_b*", r"route LIKE '/a\\_b%'"),
        ];
        for (s, q) in pairs {
            assert_eq!(search(s), sql(q), "{s}");
        }
    }

    #[test]
    fn errors_point_at_the_term() {
        let e = parse_search("status:500 stauts:1", &ctx()).unwrap_err();
        assert_eq!((e.col, e.len), (12, 8));
        assert!(e.message.contains("did you mean `status`"));
        assert!(
            parse_search("status:abc", &ctx())
                .unwrap_err()
                .message
                .contains("not a number")
        );
        assert!(
            parse_search("status:5*", &ctx())
                .unwrap_err()
                .message
                .contains("only works on text")
        );
        assert!(
            parse_search("route:\"unclosed", &ctx())
                .unwrap_err()
                .message
                .contains("unclosed")
        );
        assert!(
            parse_search("req_body:x", &ctx())
                .unwrap_err()
                .message
                .contains("permission")
        );
    }

    #[test]
    fn renders_back_to_search() {
        for s in [
            "status:>=500 route:/checkout/* user_id:u_123",
            "-method:GET client:ios,android status:5xx",
            "attr.tenant:acme \"free text\"",
            "timeout",
        ] {
            let e = search(s);
            let back = to_search(&e, &ctx()).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(search(&back), e, "{s} -> {back}");
        }
        assert_eq!(to_search(&sql("status = 1 OR route = '/a'"), &ctx()), None);
    }
}
