//! SQL `WHERE` text → Logbox AST.
//!
//! 1. Tokenize the original text (so positions match what the user typed),
//!    drop an optional leading `WHERE` and trailing `;`, and reject comments,
//!    extra statements and look-alike quote characters.
//! 2. Parse one expression with `sqlparser` (ClickHouse dialect).
//! 3. Convert the allowed subset into [`Expr`], type-checking against the
//!    column registry. Everything else is rejected with a positioned error.

use sqlparser::ast::{
    self as sql, AccessExpr, BinaryOperator, DateTimeField, FunctionArg, FunctionArgExpr,
    FunctionArguments, ObjectNamePart, Spanned, Subscript, UnaryOperator, Value,
};
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::keywords::Keyword;
use sqlparser::parser::{Parser, ParserError};
use sqlparser::tokenizer::{Location, Span, Token, TokenWithSpan, Tokenizer, Whitespace};

use crate::ast::{CmpOp, Expr, Interval, Literal, Operand, Sign, Ty, Unit};
use crate::error::{QueryError, Result};
use crate::registry::{Column, Kind, Table, map_alias};
use crate::time::parse_datetime_ms;

pub const MAX_CHARS: usize = 4_000;
pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: usize = 256;
pub const MAX_IN_LIST: usize = 500;
pub const MAX_REGEX: usize = 200;
pub const MAX_STRING: usize = 1_000;
const PARSER_RECURSION_LIMIT: usize = 64;

/// What the query is for.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub table: Table,
    /// Whether bodies and headers may be referenced (the `can_view_bodies` permission).
    pub allow_sensitive: bool,
    /// Live-tail filters can only use columns present in the live summary.
    pub live: bool,
}

impl Ctx {
    pub fn new(table: Table) -> Self {
        Self {
            table,
            allow_sensitive: false,
            live: false,
        }
    }
}

/// Parses a filter. `Ok(None)` means an empty filter (match everything).
pub fn parse(text: &str, ctx: &Ctx) -> Result<Option<Expr>> {
    if text.chars().count() > MAX_CHARS {
        return Err(QueryError::at_start(format!(
            "the query is longer than {MAX_CHARS} characters"
        )));
    }
    let dialect = ClickHouseDialect {};
    let tokens = Tokenizer::new(&dialect, text)
        .tokenize_with_location()
        .map_err(|e| from_message(&e.to_string()))?;
    let Some(tokens) = clean_tokens(tokens)? else {
        return Ok(None);
    };
    let mut parser = Parser::new(&dialect)
        .with_recursion_limit(PARSER_RECURSION_LIMIT)
        .with_tokens_with_locations(tokens);
    let expr = parser.parse_expr().map_err(parser_error)?;
    let next = parser.peek_token();
    if next.token != Token::EOF {
        return Err(trailing_error(&next));
    }
    let mut conv = Converter {
        ctx: *ctx,
        nodes: 0,
    };
    let out = conv.condition(&expr, 1)?;
    Ok(Some(out))
}

const LOOKALIKE_QUOTES: &[char] = &[
    '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}', '\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}',
    '\u{2032}', '\u{2033}', '\u{00AB}', '\u{00BB}', '\u{FF07}', '\u{FF02}', '\u{0060}',
];

/// Strips the optional `WHERE` and trailing `;`, and rejects comments,
/// extra statements and stray characters. `None` means nothing is left.
fn clean_tokens(tokens: Vec<TokenWithSpan>) -> Result<Option<Vec<TokenWithSpan>>> {
    for t in &tokens {
        match &t.token {
            Token::Whitespace(Whitespace::SingleLineComment { .. })
            | Token::Whitespace(Whitespace::MultiLineComment(_)) => {
                return Err(QueryError::at("SQL comments are not allowed", t.span));
            }
            Token::Char(c) if LOOKALIKE_QUOTES.contains(c) => {
                return Err(QueryError::at(
                    format!(
                        "`{c}` is not a quote character here; use ASCII single quotes, e.g. 'value'"
                    ),
                    t.span,
                ));
            }
            Token::Char(c) => {
                return Err(QueryError::at(
                    format!("unexpected character `{c}`"),
                    t.span,
                ));
            }
            _ => {}
        }
    }
    let mut toks: Vec<TokenWithSpan> = tokens
        .into_iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_) | Token::EOF))
        .collect();
    if let Some(first) = toks.first()
        && matches!(&first.token, Token::Word(w) if w.keyword == Keyword::WHERE && w.quote_style.is_none())
    {
        toks.remove(0);
    }
    while matches!(toks.last().map(|t| &t.token), Some(Token::SemiColon)) {
        toks.pop();
    }
    if let Some(semi) = toks.iter().find(|t| t.token == Token::SemiColon) {
        return Err(QueryError::at(
            "only one expression is allowed; remove the `;` and what follows",
            semi.span,
        ));
    }
    if toks.is_empty() {
        return Ok(None);
    }
    let end = toks.last().map_or(Location::new(1, 1), |t| t.span.end);
    toks.push(TokenWithSpan::new(Token::EOF, Span::new(end, end)));
    Ok(Some(toks))
}

fn trailing_error(next: &TokenWithSpan) -> QueryError {
    let msg = match &next.token {
        Token::Word(w) if matches!(w.keyword, Keyword::UNION | Keyword::SELECT | Keyword::FROM) => {
            format!(
                "`{}` is not allowed: write a filter condition only",
                w.value
            )
        }
        Token::Word(w) if matches!(w.keyword, Keyword::ORDER | Keyword::GROUP | Keyword::LIMIT) => {
            format!(
                "`{}` is not part of a filter; use the sort, group-by and limit controls instead",
                w.value
            )
        }
        other => format!("unexpected `{other}` after the end of the condition"),
    };
    QueryError::at(msg, next.span)
}

/// sqlparser messages end with `at Line: L, Column: C`.
fn from_message(msg: &str) -> QueryError {
    let msg = msg
        .trim_start_matches("sql parser error: ")
        .trim_start_matches("sql tokenizer error: ");
    if let Some(i) = msg.find(" at Line: ") {
        let (text, loc) = msg.split_at(i);
        let nums: Vec<u32> = loc
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if let [line, col, ..] = nums[..] {
            return QueryError::new(friendly(text), line, col, 1);
        }
    }
    QueryError::at_start(friendly(msg))
}

fn friendly(msg: &str) -> String {
    let m = msg.trim();
    if let Some(rest) = m.strip_prefix("Expected: ") {
        return match rest.strip_suffix(", found: EOF") {
            Some(what) => format!("expected {what}, but the query ended"),
            None => format!("expected {rest}"),
        };
    }
    m.to_string()
}

fn parser_error(e: ParserError) -> QueryError {
    match e {
        ParserError::RecursionLimitExceeded => QueryError::at_start(format!(
            "the condition is nested too deeply (maximum depth {MAX_DEPTH})"
        )),
        other => from_message(&other.to_string()),
    }
}

struct Converter {
    ctx: Ctx,
    nodes: usize,
}

fn span_of(e: &sql::Expr) -> Span {
    e.span()
}

impl Converter {
    fn tick(&mut self, depth: usize, at: Span) -> Result<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(QueryError::at(
                format!("the condition is too large (more than {MAX_NODES} parts)"),
                at,
            ));
        }
        if depth > MAX_DEPTH {
            return Err(QueryError::at(
                format!("the condition is nested too deeply (maximum depth {MAX_DEPTH})"),
                at,
            ));
        }
        Ok(())
    }

    // ---- conditions ----------------------------------------------------------

    fn condition(&mut self, e: &sql::Expr, depth: usize) -> Result<Expr> {
        let at = span_of(e);
        self.tick(depth, at)?;
        match e {
            sql::Expr::Nested(inner) => self.condition(inner, depth + 1),
            sql::Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::And | BinaryOperator::Or => {
                    let is_and = matches!(op, BinaryOperator::And);
                    let mut items = Vec::new();
                    for side in [left, right] {
                        // A flat `a AND b AND c` chain is not nesting: only grow
                        // the depth when the operator changes.
                        let same =
                            matches!(side.as_ref(), sql::Expr::BinaryOp { op: o, .. } if o == op);
                        let d = if same { depth } else { depth + 1 };
                        match self.condition(side, d)? {
                            Expr::And(xs) if is_and => items.extend(xs),
                            Expr::Or(xs) if !is_and => items.extend(xs),
                            x => items.push(x),
                        }
                    }
                    Ok(if is_and {
                        Expr::And(items)
                    } else {
                        Expr::Or(items)
                    })
                }
                BinaryOperator::Eq
                | BinaryOperator::NotEq
                | BinaryOperator::Lt
                | BinaryOperator::LtEq
                | BinaryOperator::Gt
                | BinaryOperator::GtEq => {
                    let cmp = match op {
                        BinaryOperator::Eq => CmpOp::Eq,
                        BinaryOperator::NotEq => CmpOp::Ne,
                        BinaryOperator::Lt => CmpOp::Lt,
                        BinaryOperator::LtEq => CmpOp::Le,
                        BinaryOperator::Gt => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    self.compare(left, cmp, right, at, depth)
                }
                BinaryOperator::Plus | BinaryOperator::Minus => Err(QueryError::at(
                    "expected a condition, e.g. `status >= 500`, not arithmetic",
                    at,
                )),
                other => Err(QueryError::at(
                    format!("operator `{other}` is not supported"),
                    at,
                )),
            },
            sql::Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => Ok(Expr::Not(Box::new(self.condition(expr, depth + 1)?))),
            sql::Expr::InList {
                expr,
                list,
                negated,
            } => {
                let operand = self.operand(expr, depth + 1)?;
                self.require_row(&operand, at)?;
                if list.is_empty() {
                    return Err(QueryError::at("IN needs at least one value", at));
                }
                if list.len() > MAX_IN_LIST {
                    return Err(QueryError::at(
                        format!("IN lists are limited to {MAX_IN_LIST} values"),
                        at,
                    ));
                }
                let want = operand.ty();
                let mut items = Vec::with_capacity(list.len());
                for item in list {
                    let lit = self.literal(item, depth + 1)?;
                    items.push(coerce(lit, want, span_of(item))?);
                }
                Ok(Expr::In {
                    expr: operand,
                    list: items,
                    negated: *negated,
                })
            }
            sql::Expr::InSubquery { .. } | sql::Expr::Subquery(_) | sql::Expr::Exists { .. } => {
                Err(QueryError::at("subqueries are not allowed", at))
            }
            sql::Expr::InUnnest { .. } => Err(QueryError::at("UNNEST is not supported", at)),
            sql::Expr::Between {
                expr,
                negated,
                low,
                high,
            } => {
                let operand = self.operand(expr, depth + 1)?;
                self.require_row(&operand, at)?;
                let want = operand.ty();
                if want == Ty::Bool {
                    return Err(QueryError::at(
                        "BETWEEN needs a number, string or timestamp",
                        at,
                    ));
                }
                let low = self.typed_operand(low, want, depth + 1)?;
                let high = self.typed_operand(high, want, depth + 1)?;
                Ok(Expr::Between {
                    expr: operand,
                    low,
                    high,
                    negated: *negated,
                })
            }
            sql::Expr::Like {
                negated,
                any,
                expr,
                pattern,
                escape_char,
            } => self.like(
                *negated,
                *any,
                expr,
                pattern,
                escape_char.is_some(),
                false,
                at,
                depth,
            ),
            sql::Expr::ILike {
                negated,
                any,
                expr,
                pattern,
                escape_char,
            } => self.like(
                *negated,
                *any,
                expr,
                pattern,
                escape_char.is_some(),
                true,
                at,
                depth,
            ),
            sql::Expr::IsNull(inner) | sql::Expr::IsNotNull(inner) => {
                let operand = self.operand(inner, depth + 1)?;
                self.require_row(&operand, at)?;
                Ok(Expr::IsEmpty {
                    expr: operand,
                    negated: matches!(e, sql::Expr::IsNotNull(_)),
                })
            }
            sql::Expr::IsTrue(_)
            | sql::Expr::IsFalse(_)
            | sql::Expr::IsNotTrue(_)
            | sql::Expr::IsNotFalse(_) => Err(QueryError::at(
                "use `= true` or `= false` instead of IS TRUE / IS FALSE",
                at,
            )),
            sql::Expr::Function(f) => self.predicate_function(f, at, depth),
            sql::Expr::Identifier(id)
                if self
                    .column_by_name(&id.value)
                    .is_some_and(|c| c.kind == Kind::Bool) =>
            {
                // A bare boolean column: `body_truncated`.
                let operand = self.operand(e, depth + 1)?;
                Ok(Expr::Compare {
                    left: operand,
                    op: CmpOp::Eq,
                    right: Operand::Literal(Literal::Bool(true)),
                })
            }
            other => Err(self.unsupported(other, true)),
        }
    }

    fn compare(
        &mut self,
        left: &sql::Expr,
        op: CmpOp,
        right: &sql::Expr,
        at: Span,
        depth: usize,
    ) -> Result<Expr> {
        let l = self.operand(left, depth + 1)?;
        let r = self.operand(right, depth + 1)?;
        if !l.references_row() && !r.references_row() {
            return Err(QueryError::at(
                "a comparison needs a column on one side, e.g. `status = 500`",
                at,
            ));
        }
        // Coerce the constant side to the column side's type.
        let (l, r) = match (l, r) {
            (col, Operand::Literal(lit)) if col.references_row() => {
                let want = col.ty();
                let lit = coerce(lit, want, span_of(right))?;
                (col, Operand::Literal(lit))
            }
            (Operand::Literal(lit), col) => {
                let want = col.ty();
                let lit = coerce(lit, want, span_of(left))?;
                (Operand::Literal(lit), col)
            }
            pair => pair,
        };
        let (lt, rt) = (l.ty(), r.ty());
        if !compatible(lt, rt) {
            return Err(QueryError::at(
                format!("can't compare {} with {}", lt.describe(), rt.describe()),
                at,
            ));
        }
        if lt == Ty::Bool && !matches!(op, CmpOp::Eq | CmpOp::Ne) {
            return Err(QueryError::at(
                "booleans can only be compared with = or !=",
                at,
            ));
        }
        Ok(Expr::Compare {
            left: l,
            op,
            right: r,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn like(
        &mut self,
        negated: bool,
        any: bool,
        expr: &sql::Expr,
        pattern: &sql::Expr,
        has_escape: bool,
        case_insensitive: bool,
        at: Span,
        depth: usize,
    ) -> Result<Expr> {
        if any || has_escape {
            return Err(QueryError::at("LIKE ANY and ESCAPE are not supported", at));
        }
        let operand = self.operand(expr, depth + 1)?;
        self.require_row(&operand, at)?;
        if operand.ty() != Ty::Str {
            return Err(QueryError::at(
                format!(
                    "LIKE needs a string, but this is {}",
                    operand.ty().describe()
                ),
                span_of(expr),
            ));
        }
        let pattern = self.string_literal(
            pattern,
            depth + 1,
            "LIKE needs a quoted pattern, e.g. '/users/%'",
        )?;
        check_like_pattern(&pattern).map_err(|m| QueryError::at(m, at))?;
        Ok(Expr::Like {
            expr: operand,
            pattern,
            negated,
            case_insensitive,
        })
    }

    fn predicate_function(&mut self, f: &sql::Function, at: Span, depth: usize) -> Result<Expr> {
        let name = function_name(f, at)?;
        let args = self.function_args(f, at)?;
        match name.to_ascii_lowercase().as_str() {
            "startswith" | "endswith" => {
                let [a, b] = expect_args::<2>(&args, &name, at)?;
                let s = self.string_operand(a, depth + 1)?;
                self.require_row(&s, at)?;
                let needle = self.string_literal(
                    b,
                    depth + 1,
                    "the second argument must be a quoted string",
                )?;
                Ok(if name.eq_ignore_ascii_case("startswith") {
                    Expr::StartsWith(s, needle)
                } else {
                    Expr::EndsWith(s, needle)
                })
            }
            "match" => {
                let [a, b] = expect_args::<2>(&args, &name, at)?;
                let s = self.string_operand(a, depth + 1)?;
                self.require_row(&s, at)?;
                let re = self.string_literal(
                    b,
                    depth + 1,
                    "match() needs a quoted regex, e.g. match(path, '^/v[0-9]+/')",
                )?;
                if re.chars().count() > MAX_REGEX {
                    return Err(QueryError::at(
                        format!("regexes are limited to {MAX_REGEX} characters"),
                        span_of(b),
                    ));
                }
                if let Err(e) = crate::regex::re2_pattern(&re) {
                    let last = e.lines().last().unwrap_or("").trim();
                    let last = last.strip_prefix("error: ").unwrap_or(last);
                    return Err(QueryError::at(format!("invalid regex: {last}"), span_of(b)));
                }
                Ok(Expr::Match(s, re))
            }
            "lower" | "upper" | "length" | "position" | "now" => Err(QueryError::at(
                format!(
                    "`{name}()` returns a value, not a condition; compare it, e.g. `{name}(...) = ...`"
                ),
                at,
            )),
            _ => Err(unknown_function(&name, at)),
        }
    }

    // ---- operands ------------------------------------------------------------

    fn typed_operand(&mut self, e: &sql::Expr, want: Ty, depth: usize) -> Result<Operand> {
        let op = self.operand(e, depth)?;
        let op = match op {
            Operand::Literal(l) => Operand::Literal(coerce(l, want, span_of(e))?),
            other => other,
        };
        if !compatible(op.ty(), want) {
            return Err(QueryError::at(
                format!("expected {}, found {}", want.describe(), op.ty().describe()),
                span_of(e),
            ));
        }
        Ok(op)
    }

    fn string_operand(&mut self, e: &sql::Expr, depth: usize) -> Result<Operand> {
        let op = self.operand(e, depth)?;
        if op.ty() != Ty::Str {
            return Err(QueryError::at(
                format!("expected a string, found {}", op.ty().describe()),
                span_of(e),
            ));
        }
        Ok(op)
    }

    fn operand(&mut self, e: &sql::Expr, depth: usize) -> Result<Operand> {
        let at = span_of(e);
        self.tick(depth, at)?;
        match e {
            sql::Expr::Nested(inner) => self.operand(inner, depth + 1),
            sql::Expr::Identifier(id) => {
                let col = self.column(&id.value, id.span)?;
                if col.kind == Kind::Map {
                    let alias = if col.name == "attributes" {
                        "attr"
                    } else {
                        col.name
                    };
                    return Err(QueryError::at(
                        format!("`{}` is a map; use {alias}['key']", id.value),
                        id.span,
                    ));
                }
                Ok(Operand::Column(col))
            }
            sql::Expr::CompoundIdentifier(parts) => {
                let first = parts
                    .first()
                    .map(|p| p.value.to_ascii_lowercase())
                    .unwrap_or_default();
                if first == "system" || first == "information_schema" {
                    return Err(QueryError::at("system tables are not accessible", at));
                }
                let last = parts.last().map(|p| p.value.as_str()).unwrap_or_default();
                Err(QueryError::at(
                    format!("qualified names are not supported; write just `{last}`"),
                    at,
                ))
            }
            sql::Expr::CompoundFieldAccess { root, access_chain } => {
                self.map_access(root, access_chain, at, depth)
            }
            sql::Expr::Value(v) => Ok(Operand::Literal(self.value(&v.value, v.span)?)),
            sql::Expr::UnaryOp {
                op: UnaryOperator::Minus,
                expr,
            } => match self.operand(expr, depth + 1)? {
                Operand::Literal(Literal::Int(i)) => Ok(Operand::Literal(Literal::Int(-i))),
                Operand::Literal(Literal::Float(x)) => Ok(Operand::Literal(Literal::Float(-x))),
                _ => Err(QueryError::at("`-` only applies to numbers here", at)),
            },
            sql::Expr::UnaryOp {
                op: UnaryOperator::Plus,
                expr,
            } => self.operand(expr, depth + 1),
            sql::Expr::BinaryOp { left, op, right }
                if matches!(op, BinaryOperator::Plus | BinaryOperator::Minus) =>
            {
                let base = self.operand(left, depth + 1)?;
                if base.ty() != Ty::Time {
                    return Err(QueryError::at(
                        "arithmetic is only supported between a timestamp and an INTERVAL, e.g. now() - INTERVAL 15 MINUTE",
                        at,
                    ));
                }
                let sql::Expr::Interval(iv) = right.as_ref() else {
                    return Err(QueryError::at(
                        "expected an INTERVAL here, e.g. INTERVAL 15 MINUTE",
                        span_of(right),
                    ));
                };
                self.tick(depth + 1, span_of(right))?;
                let interval = self.interval(iv, span_of(right))?;
                let sign = if matches!(op, BinaryOperator::Plus) {
                    Sign::Plus
                } else {
                    Sign::Minus
                };
                Ok(Operand::TimeOffset {
                    base: Box::new(base),
                    sign,
                    interval,
                })
            }
            sql::Expr::Interval(_) => Err(QueryError::at(
                "an INTERVAL must be added to or subtracted from a timestamp, e.g. now() - INTERVAL 1 HOUR",
                at,
            )),
            sql::Expr::Position { expr, r#in } => {
                let needle = self.string_operand(expr, depth + 1)?;
                let hay = self.string_operand(r#in, depth + 1)?;
                Ok(Operand::Position(Box::new(hay), Box::new(needle)))
            }
            sql::Expr::Function(f) => self.value_function(f, at, depth),
            other => Err(self.unsupported(other, false)),
        }
    }

    fn map_access(
        &mut self,
        root: &sql::Expr,
        chain: &[AccessExpr],
        at: Span,
        depth: usize,
    ) -> Result<Operand> {
        let sql::Expr::Identifier(id) = root else {
            return Err(QueryError::at(
                "only attr['key'], correlation['key'] and header maps support [ ]",
                at,
            ));
        };
        let Some(name) = map_alias(&id.value) else {
            return Err(QueryError::at(
                format!(
                    "`{}` is not a map; [ ] works on attr, correlation, req_headers and res_headers",
                    id.value
                ),
                id.span,
            ));
        };
        let col = self.column(name, id.span)?;
        let [AccessExpr::Subscript(Subscript::Index { index })] = chain else {
            return Err(QueryError::at("use a single key, e.g. attr['tenant']", at));
        };
        let key = self.string_literal(
            index,
            depth + 1,
            "map keys must be quoted strings, e.g. attr['tenant']",
        )?;
        if key.is_empty() || key.chars().count() > 128 {
            return Err(QueryError::at(
                "map keys must be 1 to 128 characters",
                span_of(index),
            ));
        }
        Ok(Operand::MapKey { map: col, key })
    }

    fn value_function(&mut self, f: &sql::Function, at: Span, depth: usize) -> Result<Operand> {
        let name = function_name(f, at)?;
        let args = self.function_args(f, at)?;
        match name.to_ascii_lowercase().as_str() {
            "now" => {
                expect_args::<0>(&args, &name, at)?;
                Ok(Operand::Now)
            }
            "lower" | "upper" | "length" => {
                let [a] = expect_args::<1>(&args, &name, at)?;
                let s = Box::new(self.string_operand(a, depth + 1)?);
                Ok(match name.to_ascii_lowercase().as_str() {
                    "lower" => Operand::Lower(s),
                    "upper" => Operand::Upper(s),
                    _ => Operand::Length(s),
                })
            }
            "position" => {
                let [a, b] = expect_args::<2>(&args, &name, at)?;
                let hay = self.string_operand(a, depth + 1)?;
                let needle = self.string_operand(b, depth + 1)?;
                Ok(Operand::Position(Box::new(hay), Box::new(needle)))
            }
            "startswith" | "endswith" | "match" => Err(QueryError::at(
                format!("`{name}()` is a condition on its own; use it without comparing"),
                at,
            )),
            _ => Err(unknown_function(&name, at)),
        }
    }

    fn function_args<'a>(&self, f: &'a sql::Function, at: Span) -> Result<Vec<&'a sql::Expr>> {
        if f.filter.is_some()
            || f.over.is_some()
            || !f.within_group.is_empty()
            || f.null_treatment.is_some()
        {
            return Err(QueryError::at(
                "FILTER, OVER and WITHIN GROUP are not supported",
                at,
            ));
        }
        if !matches!(f.parameters, FunctionArguments::None) {
            return Err(QueryError::at("parametric functions are not supported", at));
        }
        match &f.args {
            FunctionArguments::None => Ok(Vec::new()),
            FunctionArguments::Subquery(_) => Err(QueryError::at("subqueries are not allowed", at)),
            FunctionArguments::List(list) => {
                if list.duplicate_treatment.is_some() || !list.clauses.is_empty() {
                    return Err(QueryError::at(
                        "DISTINCT and argument clauses are not supported",
                        at,
                    ));
                }
                list.args
                    .iter()
                    .map(|a| match a {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Ok(e),
                        _ => Err(QueryError::at("only plain arguments are supported", at)),
                    })
                    .collect()
            }
        }
    }

    // ---- literals ------------------------------------------------------------

    fn literal(&mut self, e: &sql::Expr, depth: usize) -> Result<Literal> {
        match self.operand(e, depth)? {
            Operand::Literal(l) => Ok(l),
            _ => Err(QueryError::at("expected a literal value", span_of(e))),
        }
    }

    fn string_literal(&mut self, e: &sql::Expr, depth: usize, hint: &str) -> Result<String> {
        match self.literal(e, depth) {
            Ok(Literal::Str(s)) => Ok(s),
            _ => Err(QueryError::at(hint, span_of(e))),
        }
    }

    fn value(&self, v: &Value, at: Span) -> Result<Literal> {
        match v {
            Value::Number(n, _) => {
                parse_number(n).ok_or_else(|| QueryError::at(format!("invalid number `{n}`"), at))
            }
            Value::SingleQuotedString(s) => {
                if s.chars().count() > MAX_STRING {
                    return Err(QueryError::at(
                        format!("strings are limited to {MAX_STRING} characters"),
                        at,
                    ));
                }
                Ok(Literal::Str(s.clone()))
            }
            Value::Boolean(b) => Ok(Literal::Bool(*b)),
            Value::Null => Err(QueryError::at(
                "use `IS NULL` / `IS NOT NULL` to check for empty values",
                at,
            )),
            Value::DoubleQuotedString(_) => Err(QueryError::at(
                "double quotes are for names; use single quotes for strings, e.g. 'value'",
                at,
            )),
            Value::Placeholder(_) => Err(QueryError::at("placeholders are not supported", at)),
            _ => Err(QueryError::at(
                "this kind of literal is not supported; use a plain 'string' or number",
                at,
            )),
        }
    }

    fn interval(&mut self, iv: &sql::Interval, at: Span) -> Result<Interval> {
        if iv.last_field.is_some()
            || iv.leading_precision.is_some()
            || iv.fractional_seconds_precision.is_some()
        {
            return Err(QueryError::at(
                "use a simple interval, e.g. INTERVAL 15 MINUTE",
                at,
            ));
        }
        let unit = match iv.leading_field {
            Some(DateTimeField::Second | DateTimeField::Seconds) => Unit::Second,
            Some(DateTimeField::Minute | DateTimeField::Minutes) => Unit::Minute,
            Some(DateTimeField::Hour | DateTimeField::Hours) => Unit::Hour,
            Some(DateTimeField::Day | DateTimeField::Days) => Unit::Day,
            _ => {
                return Err(QueryError::at(
                    "INTERVAL units are SECOND, MINUTE, HOUR or DAY",
                    at,
                ));
            }
        };
        let amount = match iv.value.as_ref() {
            sql::Expr::Value(v) => match &v.value {
                Value::Number(n, _) => n.parse::<i64>().ok(),
                Value::SingleQuotedString(s) => s.trim().parse::<i64>().ok(),
                _ => None,
            },
            _ => None,
        };
        match amount {
            Some(a) if (0..=366 * 24 * 3600).contains(&a) => Ok(Interval { amount: a, unit }),
            _ => Err(QueryError::at(
                "INTERVAL needs a whole, non-negative number, e.g. INTERVAL 15 MINUTE",
                at,
            )),
        }
    }

    // ---- columns ---------------------------------------------------------------

    fn column_by_name(&self, name: &str) -> Option<&'static Column> {
        self.ctx.table.column(name)
    }

    fn column(&self, name: &str, at: Span) -> Result<&'static Column> {
        resolve_column(&self.ctx, name).map_err(|m| QueryError::at(m, at))
    }

    fn require_row(&self, op: &Operand, at: Span) -> Result<()> {
        if op.references_row() {
            Ok(())
        } else {
            Err(QueryError::at(
                "this condition needs a column, e.g. `route LIKE '/users/%'`",
                at,
            ))
        }
    }

    fn unsupported(&self, e: &sql::Expr, condition: bool) -> QueryError {
        let at = span_of(e);
        let what = match e {
            sql::Expr::Cast { .. } | sql::Expr::Convert { .. } => "casts are not supported",
            sql::Expr::Case { .. } => "CASE is not supported",
            sql::Expr::Subquery(_) | sql::Expr::Exists { .. } | sql::Expr::InSubquery { .. } => {
                "subqueries are not allowed"
            }
            sql::Expr::Tuple(_) => "tuples are not supported",
            sql::Expr::Array(_) => "arrays are not supported; use IN ('a', 'b')",
            sql::Expr::Map(_) | sql::Expr::Dictionary(_) | sql::Expr::Struct { .. } => {
                "map and struct literals are not supported"
            }
            sql::Expr::Lambda(_) => "lambdas are not supported",
            sql::Expr::Wildcard(_) | sql::Expr::QualifiedWildcard(..) => {
                "`*` is not valid in a filter"
            }
            sql::Expr::TypedString(_) => {
                "typed literals are not supported; compare timestamps with a string, e.g. timestamp > '2026-01-02 10:00'"
            }
            sql::Expr::Value(_)
            | sql::Expr::Identifier(_)
            | sql::Expr::CompoundFieldAccess { .. }
                if condition =>
            {
                "expected a condition, e.g. `status >= 500`"
            }
            sql::Expr::SimilarTo { .. } | sql::Expr::RLike { .. } => {
                "use match(column, 'regex') for regular expressions"
            }
            sql::Expr::Extract { .. }
            | sql::Expr::Ceil { .. }
            | sql::Expr::Floor { .. }
            | sql::Expr::Substring { .. }
            | sql::Expr::Trim { .. }
            | sql::Expr::Overlay { .. } => {
                "this function is not supported; see the list of allowed functions in the syntax help"
            }
            _ => "this syntax is not supported in a filter",
        };
        QueryError::at(what, at)
    }
}

fn function_name(f: &sql::Function, at: Span) -> Result<String> {
    match f.name.0.as_slice() {
        [ObjectNamePart::Identifier(id)] => Ok(id.value.clone()),
        parts => {
            let full: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
            if full
                .first()
                .is_some_and(|s| s.eq_ignore_ascii_case("system"))
            {
                return Err(QueryError::at("system tables are not accessible", at));
            }
            Err(unknown_function(&full.join("."), at))
        }
    }
}

fn expect_args<'a, const N: usize>(
    args: &[&'a sql::Expr],
    name: &str,
    at: Span,
) -> Result<[&'a sql::Expr; N]> {
    <[&sql::Expr; N]>::try_from(args.to_vec()).map_err(|_| {
        let s = if N == 1 { "" } else { "s" };
        QueryError::at(format!("`{name}()` takes {N} argument{s}"), at)
    })
}

const FUNCTION_NAMES: &[&str] = &[
    "lower",
    "upper",
    "length",
    "startsWith",
    "endsWith",
    "position",
    "match",
    "now",
];

fn unknown_function(name: &str, at: Span) -> QueryError {
    let hint = closest(name, FUNCTION_NAMES.iter().copied())
        .map(|s| format!("; did you mean `{s}`?"))
        .unwrap_or_else(|| format!("; allowed: {}", FUNCTION_NAMES.join(", ")));
    QueryError::at(format!("unknown function `{name}`{hint}"), at)
}

/// Looks up a column and checks the context allows it. Shared by the SQL,
/// search and chip front ends so they enforce the same rules.
pub(crate) fn resolve_column(
    ctx: &Ctx,
    name: &str,
) -> std::result::Result<&'static Column, String> {
    let lower = name.to_ascii_lowercase();
    if lower == "system" || lower.starts_with("system.") {
        return Err("system tables are not accessible".into());
    }
    let Some(col) = ctx.table.column(name) else {
        let hint = closest(name, ctx.table.columns().iter().map(|c| c.name))
            .map(|s| format!("; did you mean `{s}`?"))
            .unwrap_or_default();
        return Err(format!("unknown column `{name}`{hint}"));
    };
    if !col.filterable {
        return Err(format!("`{}` can't be filtered on", col.name));
    }
    if col.sensitive && !ctx.allow_sensitive {
        return Err(format!(
            "`{}` contains request data you don't have permission to view",
            col.name
        ));
    }
    if ctx.live && !col.live {
        return Err(format!(
            "`{}` isn't available in the live tail; use Explore to filter on it",
            col.name
        ));
    }
    Ok(col)
}

/// The candidate within a small edit distance, if any.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    candidates
        .map(|c| (levenshtein(&lower, &c.to_ascii_lowercase()), c))
        .filter(|(d, c)| *d <= if c.len() < 5 { 1 } else { 2 })
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn parse_number(n: &str) -> Option<Literal> {
    if let Ok(i) = n.parse::<i64>() {
        return Some(Literal::Int(i));
    }
    n.parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map(Literal::Float)
}

/// ClickHouse rejects a LIKE pattern ending in an unpaired `\`.
pub fn check_like_pattern(p: &str) -> std::result::Result<(), String> {
    let trailing = p.chars().rev().take_while(|c| *c == '\\').count();
    if trailing % 2 == 1 {
        Err("a LIKE pattern can't end with a single `\\`; escape it as `\\\\`".into())
    } else {
        Ok(())
    }
}

/// Whether values of these types can be compared.
fn compatible(a: Ty, b: Ty) -> bool {
    a == b
}

/// Converts a literal to the type it's compared with, or explains why it can't be.
pub(crate) fn coerce(lit: Literal, want: Ty, at: Span) -> Result<Literal> {
    coerce_value(lit, want).map_err(|m| QueryError::at(m, at))
}

pub(crate) fn coerce_value(lit: Literal, want: Ty) -> std::result::Result<Literal, String> {
    match (want, lit) {
        (Ty::Str, l @ Literal::Str(_)) => Ok(l),
        (Ty::Num, l @ (Literal::Int(_) | Literal::Float(_))) => Ok(l),
        (Ty::Bool, l @ Literal::Bool(_)) => Ok(l),
        (Ty::Bool, Literal::Int(0)) => Ok(Literal::Bool(false)),
        (Ty::Bool, Literal::Int(1)) => Ok(Literal::Bool(true)),
        (Ty::U64, Literal::Int(i)) if i > 0 => Ok(Literal::U64(i as u64)),
        (Ty::U64, Literal::Str(s)) => s
            .trim()
            .parse::<u64>()
            .map(Literal::U64)
            .map_err(|_| "expected an ID made of digits".to_string()),
        (Ty::U64, l @ Literal::U64(_)) => Ok(l),
        (Ty::Time, Literal::Str(s)) => parse_datetime_ms(&s).map(Literal::Time).ok_or_else(|| {
            "expected a timestamp like '2026-01-02 10:00:00' or '2026-01-02T10:00:00Z'".to_string()
        }),
        (Ty::Time, l @ Literal::Time(_)) => Ok(l),
        (_, l) => {
            let hint = match (want, &l) {
                (Ty::Str, Literal::Int(_) | Literal::Float(_)) => " (quote it: '…')",
                (Ty::Num, Literal::Str(_)) => " (remove the quotes)",
                (Ty::Time, _) => " (use e.g. '2026-01-02 10:00:00' or now() - INTERVAL 1 HOUR)",
                _ => "",
            };
            Err(format!(
                "expected {}, found {}{hint}",
                want.describe(),
                l.ty().describe()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Expr {
        parse(s, &Ctx::new(Table::Requests)).unwrap().unwrap()
    }

    fn err(s: &str) -> QueryError {
        parse(s, &Ctx::new(Table::Requests)).unwrap_err()
    }

    #[test]
    fn empty_and_where_only_mean_no_filter() {
        for s in ["", "   ", "WHERE", "where ;", ";;"] {
            assert_eq!(parse(s, &Ctx::new(Table::Requests)).unwrap(), None, "{s:?}");
        }
    }

    #[test]
    fn strips_where_and_semicolons() {
        assert_eq!(ok("WHERE status = 500;"), ok("status = 500"));
        assert_eq!(ok("where status = 500 ; ;"), ok("status = 500"));
    }

    #[test]
    fn spec_example_parses() {
        let e = ok(
            "WHERE status >= 500 AND route LIKE '/checkout/%' AND duration_ms > 800\n  AND attr['tenant'] IN ('acme', 'globex') AND timestamp > now() - INTERVAL 15 MINUTE",
        );
        let Expr::And(items) = &e else {
            panic!("{e:?}")
        };
        assert_eq!(items.len(), 5);
        assert_eq!(e.condition_count(), 5);
    }

    #[test]
    fn every_operator() {
        for s in [
            "status = 500",
            "status != 500",
            "status <> 500",
            "status < 500",
            "status <= 500",
            "status > 500",
            "status >= 500",
            "500 <= status",
            "status IN (500, 502)",
            "status NOT IN (200)",
            "status BETWEEN 500 AND 599",
            "status NOT BETWEEN 200 AND 299",
            "route LIKE '/users/%'",
            "route NOT LIKE '/a%'",
            "route ILIKE '/USERS/%'",
            "route NOT ILIKE '/x%'",
            "user_id IS NULL",
            "user_id IS NOT NULL",
            "attr['k'] IS NULL",
            "NOT status = 200",
            "(status = 1 OR status = 2) AND method = 'GET'",
            "lower(method) = 'get'",
            "upper(client) = 'WEB'",
            "length(path) > 10",
            "startsWith(path, '/v1')",
            "endsWith(path, '.json')",
            "position(path, 'x') > 0",
            "POSITION('x' IN path) > 0",
            "match(path, '^/v[0-9]+/')",
            "timestamp > now() - INTERVAL 15 MINUTE",
            "timestamp < now() + INTERVAL 1 DAY",
            "timestamp > '2026-01-02 10:00:00'",
            "timestamp BETWEEN '2026-01-01' AND '2026-01-02T00:00:00Z'",
            "body_truncated = true",
            "body_truncated",
            "body_truncated = 0",
            "error_fingerprint = 1234567890123456789",
            "error_fingerprint = '1234567890123456789'",
            "correlation['account_id'] = 'acc_1'",
            "attributes['tenant'] = 'a'",
            "attr['region'] != 'eu'",
            "duration_ms > 1.5",
            "duration_ms > -1",
            "STATUS = 500",
            "\"status\" = 500",
        ] {
            let _ = parse(s, &Ctx::new(Table::Requests)).unwrap_or_else(|e| panic!("{s}: {e}"));
        }
    }

    #[test]
    fn helpful_errors_with_positions() {
        let cases: &[(&str, &str, u32)] = &[
            (
                "stauts = 500",
                "unknown column `stauts`; did you mean `status`?",
                1,
            ),
            (
                "status = 'x'",
                "expected a number, found a string (remove the quotes)",
                10,
            ),
            (
                "route = 5",
                "expected a string, found a number (quote it: '…')",
                9,
            ),
            ("status = 1 -- hi", "SQL comments are not allowed", 12),
            ("status = 1 /* hi */", "SQL comments are not allowed", 12),
            (
                "status = 1; DROP TABLE requests",
                "only one expression is allowed; remove the `;` and what follows",
                11,
            ),
            (
                "status = 1 UNION SELECT * FROM system.users",
                "`UNION` is not allowed: write a filter condition only",
                12,
            ),
            (
                "status = 1 ORDER BY timestamp",
                "`ORDER` is not part of a filter; use the sort, group-by and limit controls instead",
                12,
            ),
            (
                "foo(status) = 1",
                "unknown function `foo`; allowed: lower, upper, length, startsWith, endsWith, position, match, now",
                1,
            ),
            (
                "lowr(method) = 'get'",
                "unknown function `lowr`; did you mean `lower`?",
                1,
            ),
            (
                "attributes = 'x'",
                "`attributes` is a map; use attr['key']",
                1,
            ),
            (
                "route['x'] = 'y'",
                "`route` is not a map; [ ] works on attr, correlation, req_headers and res_headers",
                1,
            ),
            (
                "1 = 1",
                "a comparison needs a column on one side, e.g. `status = 500`",
                1,
            ),
            (
                "status = NULL",
                "use `IS NULL` / `IS NOT NULL` to check for empty values",
                10,
            ),
            ("route = \"x\"", "unknown column `x`", 9),
            (
                "CAST(status AS String) = '500'",
                "casts are not supported",
                6,
            ),
            ("status = (SELECT 1)", "subqueries are not allowed", 11),
            (
                "status IN (SELECT status FROM requests)",
                "subqueries are not allowed",
                1,
            ),
            (
                "system.users.name = 'x'",
                "system tables are not accessible",
                1,
            ),
            (
                "req_body LIKE '%x%'",
                "`req_body` contains request data you don't have permission to view",
                1,
            ),
            (
                "duration_ms + 5 > 10",
                "arithmetic is only supported between a timestamp and an INTERVAL, e.g. now() - INTERVAL 15 MINUTE",
                1,
            ),
            (
                "timestamp > now() - INTERVAL 3 WEEK",
                "INTERVAL units are SECOND, MINUTE, HOUR or DAY",
                30,
            ),
            ("match(path, '(')", "invalid regex: unclosed group", 13),
            (
                "status = ‘500’",
                "`‘` is not a quote character here; use ASCII single quotes, e.g. 'value'",
                10,
            ),
            ("route", "expected a condition, e.g. `status >= 500`", 1),
            (
                "status >= 500 AND",
                "expected an expression, but the query ended",
                18,
            ),
            (
                "method = 'GET' AND body_truncated > true",
                "booleans can only be compared with = or !=",
                20,
            ),
        ];
        for (input, message, col) in cases {
            let e = err(input);
            assert_eq!(&e.message, message, "{input}");
            assert_eq!(e.col, *col, "{input}: {e:?}");
        }
    }

    #[test]
    fn sensitive_and_live_contexts() {
        let mut ctx = Ctx::new(Table::Requests);
        ctx.allow_sensitive = true;
        assert!(parse("req_body LIKE '%x%' AND req_headers['accept'] = 'a'", &ctx).is_ok());
        ctx.live = true;
        let e = parse("req_body LIKE '%x%'", &ctx).unwrap_err();
        assert!(e.message.contains("live tail"), "{e:?}");
        assert!(parse("status >= 500 AND attr['k'] = 'v'", &ctx).is_ok());
    }

    #[test]
    fn limits() {
        let long = format!("route = '{}'", "x".repeat(4_000));
        assert!(err(&long).message.contains("longer than"));
        let list = (0..501)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            err(&format!("status IN ({list})"))
                .message
                .contains("500 values")
        );
        let regex = format!("match(path, '{}')", "a".repeat(201));
        assert!(err(&regex).message.contains("200 characters"));
        let deep = format!("{}status = 1{}", "(".repeat(40), ")".repeat(40));
        assert!(
            err(&deep).message.contains("nested too deeply"),
            "{:?}",
            err(&deep)
        );
        let wide = (0..130)
            .map(|i| format!("status = {i}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        assert!(err(&wide).message.contains("too large"));
    }

    #[test]
    fn events_table_has_its_own_columns() {
        let ctx = Ctx::new(Table::Events);
        assert!(parse("status = 'error' AND kind = 'job' AND attempt > 1", &ctx).is_ok());
        assert!(
            parse("status = 500", &ctx)
                .unwrap_err()
                .message
                .contains("expected a string")
        );
        assert!(
            parse("route = '/x'", &ctx)
                .unwrap_err()
                .message
                .contains("unknown column")
        );
    }
}
