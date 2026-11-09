//! `match(col, 'regex')` support: ClickHouse runs patterns with RE2, the live
//! tail with Rust's `regex`. This module keeps them in agreement.
//!
//! Validation rejects what RE2 can't compile (stacked repetition like `+*`,
//! repetition counts over 1000, nested classes and class set operations,
//! Rust-only flags and assertions). For evaluation, the Perl classes
//! `\d \w \s` and the word boundaries `\b \B`, which are ASCII-only in RE2
//! but Unicode-aware in Rust, are rewritten to explicit ASCII forms.

use regex_syntax::ast::{
    self, AssertionKind, Ast, ClassPerlKind, ClassSet, ClassSetItem, RepetitionKind,
    RepetitionRange,
};

const MAX_REPEAT: u32 = 1000;

/// Validates `pattern` against RE2's rules and returns the equivalent Rust
/// pattern for in-memory evaluation.
pub fn re2_pattern(pattern: &str) -> Result<String, String> {
    let parsed = ast::parse::ParserBuilder::new()
        .nest_limit(32)
        .build()
        .parse(pattern)
        .map_err(|e| e.kind().to_string())?;
    let mut edits: Vec<(usize, usize, &'static str)> = Vec::new();
    walk(&parsed, pattern, &mut edits)?;
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut out = pattern.to_string();
    for (start, end, replacement) in edits {
        out.replace_range(start..end, replacement);
    }
    // Final sanity check with the engine the evaluator uses.
    regex::Regex::new(&out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn perl_outside(kind: &ClassPerlKind, negated: bool) -> &'static str {
    match (kind, negated) {
        (ClassPerlKind::Digit, false) => "[0-9]",
        (ClassPerlKind::Digit, true) => "[^0-9]",
        (ClassPerlKind::Word, false) => "[0-9A-Za-z_]",
        (ClassPerlKind::Word, true) => "[^0-9A-Za-z_]",
        (ClassPerlKind::Space, false) => "[\\t\\n\\x0C\\r ]",
        (ClassPerlKind::Space, true) => "[^\\t\\n\\x0C\\r ]",
    }
}

fn perl_inside(kind: &ClassPerlKind) -> &'static str {
    match kind {
        ClassPerlKind::Digit => "0-9",
        ClassPerlKind::Word => "0-9A-Za-z_",
        ClassPerlKind::Space => "\\t\\n\\x0C\\r ",
    }
}

fn walk(a: &Ast, src: &str, edits: &mut Vec<(usize, usize, &'static str)>) -> Result<(), String> {
    match a {
        Ast::Empty(_) | Ast::Literal(_) | Ast::Dot(_) | Ast::ClassUnicode(_) => Ok(()),
        Ast::Flags(f) => check_flags(&src[f.span.start.offset..f.span.end.offset]),
        Ast::Assertion(x) => match x.kind {
            AssertionKind::StartLine
            | AssertionKind::EndLine
            | AssertionKind::StartText
            | AssertionKind::EndText => Ok(()),
            AssertionKind::WordBoundary => {
                edits.push((x.span.start.offset, x.span.end.offset, "(?-u:\\b)"));
                Ok(())
            }
            AssertionKind::NotWordBoundary => {
                edits.push((x.span.start.offset, x.span.end.offset, "(?-u:\\B)"));
                Ok(())
            }
            _ => Err("this word-boundary form isn't supported; use \\b".into()),
        },
        Ast::ClassPerl(p) => {
            edits.push((
                p.span.start.offset,
                p.span.end.offset,
                perl_outside(&p.kind, p.negated),
            ));
            Ok(())
        }
        Ast::ClassBracketed(c) => class_set(&c.kind, edits),
        Ast::Repetition(r) => {
            if matches!(*r.ast, Ast::Repetition(_)) {
                return Err(
                    "repetition operators can't be stacked (e.g. `+*`); use a group: (a+)*".into(),
                );
            }
            if let RepetitionKind::Range(range) = &r.op.kind {
                let too_big = match range {
                    RepetitionRange::Exactly(n) | RepetitionRange::AtLeast(n) => *n > MAX_REPEAT,
                    RepetitionRange::Bounded(a, b) => *a > MAX_REPEAT || *b > MAX_REPEAT,
                };
                if too_big {
                    return Err(format!("repetition counts are limited to {MAX_REPEAT}"));
                }
            }
            walk(&r.ast, src, edits)
        }
        Ast::Group(g) => {
            if let ast::GroupKind::NonCapturing(flags) = &g.kind {
                check_flags(&src[flags.span.start.offset..flags.span.end.offset])?;
            }
            walk(&g.ast, src, edits)
        }
        Ast::Alternation(x) => x.asts.iter().try_for_each(|a| walk(a, src, edits)),
        Ast::Concat(x) => x.asts.iter().try_for_each(|a| walk(a, src, edits)),
    }
}

fn class_set(set: &ClassSet, edits: &mut Vec<(usize, usize, &'static str)>) -> Result<(), String> {
    match set {
        ClassSet::BinaryOp(_) => {
            Err("character class operators (&&, --, ~~) aren't supported".into())
        }
        ClassSet::Item(item) => class_item(item, edits),
    }
}

fn class_item(
    item: &ClassSetItem,
    edits: &mut Vec<(usize, usize, &'static str)>,
) -> Result<(), String> {
    match item {
        ClassSetItem::Empty(_)
        | ClassSetItem::Literal(_)
        | ClassSetItem::Range(_)
        | ClassSetItem::Ascii(_)
        | ClassSetItem::Unicode(_) => Ok(()),
        ClassSetItem::Perl(p) => {
            if p.negated {
                return Err("\\D, \\W and \\S aren't supported inside [ ]; use an explicit range like [^0-9]".into());
            }
            edits.push((p.span.start.offset, p.span.end.offset, perl_inside(&p.kind)));
            Ok(())
        }
        ClassSetItem::Bracketed(_) => Err("nested character classes aren't supported".into()),
        ClassSetItem::Union(u) => u.items.iter().try_for_each(|i| class_item(i, edits)),
    }
}

/// RE2 understands the i, m, s and U flags only.
fn check_flags(text: &str) -> Result<(), String> {
    if text
        .chars()
        .all(|c| matches!(c, 'i' | 'm' | 's' | 'U' | '-' | '(' | ')' | '?' | ':'))
    {
        Ok(())
    } else {
        Err("only the i, m, s and U flags are supported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_re2_rejects() {
        for bad in [
            ".$+*",
            "a**",
            "a+*",
            "a{1001}",
            "[a[b]]",
            "[a&&b]",
            "(?x)a",
            "\\b{start}",
            "[\\D]",
            "(",
            "a{2,1}",
        ] {
            assert!(re2_pattern(bad).is_err(), "{bad}");
        }
        for ok in [
            "^/v[0-9]+/",
            "a*?",
            "(a+)*",
            "(?i)abc",
            "(?P<id>\\d+)",
            "\\pL+",
            "[[:alpha:]]",
            "x{1000}",
        ] {
            assert!(re2_pattern(ok).is_ok(), "{ok}: {:?}", re2_pattern(ok));
        }
    }

    #[test]
    fn perl_classes_are_ascii_like_re2() {
        let re = regex::Regex::new(&re2_pattern("^\\d+$").unwrap()).unwrap();
        assert!(re.is_match("123"));
        assert!(
            !re.is_match("١٢٣"),
            "Arabic-Indic digits are not \\d in RE2"
        );
        let re = regex::Regex::new(&re2_pattern("^[\\w-]+$").unwrap()).unwrap();
        assert!(re.is_match("a_b-9"));
        assert!(!re.is_match("é"));
        let re = regex::Regex::new(&re2_pattern("\\bcat\\b").unwrap()).unwrap();
        assert!(re.is_match("a cat!"));
        // `\d` inside a literal escape stays untouched.
        assert_eq!(re2_pattern("\\\\d").unwrap(), "\\\\d");
    }
}
