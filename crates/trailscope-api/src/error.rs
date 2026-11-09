//! API errors: a status, a stable machine-readable code and a human message.
//!
//! Internal failures are logged with detail and returned as a generic message,
//! so database errors never leak to clients.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Serialize) -> Self {
        self.details = serde_json::to_value(details).ok();
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "sign in to continue",
        )
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn too_many(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message)
    }

    pub fn invalid_query(err: trailscope_query::QueryError) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_query",
            err.message.clone(),
        )
        .with_details(err)
    }

    /// Logs the real error and returns a generic 500.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "something went wrong; see the server log",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": { "code": self.code, "message": self.message } });
        if let Some(d) = self.details
            && let Some(obj) = body["error"].as_object_mut()
        {
            obj.insert("details".into(), d);
        }
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e
            && db.is_unique_violation()
        {
            return ApiError::conflict("that already exists");
        }
        if matches!(e, sqlx::Error::RowNotFound) {
            return ApiError::not_found("record");
        }
        ApiError::internal(e)
    }
}

impl From<clickhouse::error::Error> for ApiError {
    fn from(e: clickhouse::error::Error) -> Self {
        let msg = e.to_string();
        // ClickHouse error codes for the reader's resource limits.
        let limit = |code: &str| msg.contains(&format!("Code: {code}."));
        if limit("159") {
            return ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "query_timeout",
                "the query took too long; narrow the time range or add a filter",
            );
        }
        if limit("158") || limit("396") {
            return ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "query_too_large",
                "the query reads too many rows; narrow the time range or add a filter",
            );
        }
        if limit("241") {
            return ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "query_too_large",
                "the query needs too much memory; narrow the time range or add a filter",
            );
        }
        ApiError::internal(format!("clickhouse: {msg}"))
    }
}

impl From<trailscope_core::Error> for ApiError {
    fn from(e: trailscope_core::Error) -> Self {
        ApiError::internal(e)
    }
}
