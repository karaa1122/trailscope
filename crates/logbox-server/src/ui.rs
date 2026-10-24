//! Serves the web UI embedded in the binary (built into `web/dist`).
//!
//! Unknown paths fall back to `index.html` so client-side routes work. Hashed
//! assets are cached for a year; `index.html` is never cached.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

/// The UI's Content-Security-Policy: everything from this origin, inline
/// styles for the editor and React style attributes, nothing else.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                   font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub fn router() -> Router {
    Router::new().fallback(serve)
}

async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path.starts_with("v1/") {
        return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": { "code": "not_found", "message": "no such endpoint" } })))
            .into_response();
    }
    if !path.is_empty()
        && let Some(file) = Assets::get(path)
    {
        let mime = file.metadata.mimetype().to_string();
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return with_headers(
            Response::new(Body::from(file.data.into_owned())),
            &mime,
            cache,
            mime.starts_with("text/html"),
        );
    }
    match Assets::get("index.html") {
        Some(index) => with_headers(Response::new(Body::from(index.data.into_owned())), "text/html; charset=utf-8", "no-cache", true),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!doctype html><title>Logbox</title><body style=\"font-family:system-ui;padding:40px\">\
             <h1>The web UI isn't built into this binary</h1>\
             <p>Run <code>pnpm --filter @logbox/web build</code> and rebuild <code>logbox</code>, or use the Docker image. \
             The API is available at <a href=\"/api/docs\">/api/docs</a>.</p></body>",
        )
            .into_response(),
    }
}

fn with_headers(mut resp: Response, mime: &str, cache: &str, html: bool) -> Response {
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(cache) {
        h.insert(header::CACHE_CONTROL, v);
    }
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if html {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        h.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        );
    }
    resp
}
