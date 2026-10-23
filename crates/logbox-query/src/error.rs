use serde::Serialize;

/// A query error with the position the editor should underline.
/// `line` and `col` are 1-based and count characters; `len` is in characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryError {
    pub message: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

impl QueryError {
    pub fn new(message: impl Into<String>, line: u32, col: u32, len: u32) -> Self {
        Self {
            message: message.into(),
            line: line.max(1),
            col: col.max(1),
            len: len.max(1),
        }
    }

    pub fn at_start(message: impl Into<String>) -> Self {
        Self::new(message, 1, 1, 1)
    }

    pub fn at(message: impl Into<String>, span: sqlparser::tokenizer::Span) -> Self {
        let (line, col) = (span.start.line as u32, span.start.column as u32);
        let len = if span.end.line == span.start.line && span.end.column > span.start.column {
            (span.end.column - span.start.column) as u32
        } else {
            1
        };
        if line == 0 {
            return Self::at_start(message);
        }
        Self::new(message, line, col, len)
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (line {}, column {})",
            self.message, self.line, self.col
        )
    }
}

impl std::error::Error for QueryError {}

pub type Result<T> = std::result::Result<T, QueryError>;
