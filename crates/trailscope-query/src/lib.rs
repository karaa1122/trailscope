//! The Trailscope query language: a SQL `WHERE` expression (like Jira's JQL, but
//! SQL), plus a search syntax and filter chips that produce the same AST.
//!
//! **User text never reaches ClickHouse as SQL.** Text is parsed with
//! `sqlparser`, converted into Trailscope's own typed [`ast::Expr`] (only a small,
//! whitelisted subset is accepted), type-checked against the column
//! [`registry`], and compiled to SQL in which every value is a typed query
//! parameter and every identifier comes from the registry.
//!
//! ```
//! use trailscope_query::{Ctx, Table, compile, parse};
//!
//! let ctx = Ctx::new(Table::Requests);
//! let expr = parse("WHERE status >= 500 AND attr['tenant'] = 'acme'", &ctx).unwrap().unwrap();
//! let sql = compile(&expr);
//! assert_eq!(sql.text, "((status >= {p1:Int64}) AND (attributes[{p2:String}] = {p3:String}))");
//! assert_eq!(sql.params.len(), 3);
//! ```

pub mod ast;
pub mod chips;
pub mod compile;
pub mod error;
pub mod eval;
pub mod parse;
pub mod regex;
pub mod registry;
pub mod search;
mod time;

pub use ast::Expr;
pub use chips::{Chip, ChipError, ChipOp, ChipValue, chips_to_expr, expr_to_chips};
pub use compile::{Param, ParamValue, Params, Sql, compile, compile_into, where_clause};
pub use error::QueryError;
pub use eval::{Prepared, Record, Val};
pub use parse::{Ctx, parse};
pub use registry::{Column, Kind, Table};
pub use search::{parse_search, to_search};

/// How a query was written, as stored in saved views and URLs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum QueryInput {
    Sql { text: String },
    Search { text: String },
    Filters { chips: Vec<Chip> },
}

impl QueryInput {
    /// Parses whichever syntax this is into the shared AST.
    pub fn to_expr(&self, ctx: &Ctx) -> Result<Option<Expr>, QueryError> {
        match self {
            QueryInput::Sql { text } => parse(text, ctx),
            QueryInput::Search { text } => parse_search(text, ctx),
            QueryInput::Filters { chips } => chips_to_expr(chips, ctx).map_err(|e| QueryError {
                message: format!("filter {}: {}", e.index + 1, e.message),
                line: 1,
                col: 1,
                len: 1,
            }),
        }
    }
}
