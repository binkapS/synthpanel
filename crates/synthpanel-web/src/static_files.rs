//! Static asset serving for the embedded Svelte SPA.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use include_dir::{Dir, include_dir};

static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/frontend/dist");

/// Serve an embedded asset, falling back to `index.html` for SPA routes.
pub async fn serve(path: &str) -> Response {
    let requested = path.trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    // Reject traversal attempts before touching the embedded directory.
    if requested.split('/').any(|segment| segment == "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    if let Some(file) = ASSETS.get_file(requested) {
        let mime = mime_guess::from_path(file.path()).first_or_octet_stream();
        let mut response = (file.contents()).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime.as_ref())
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        );
        if requested == "index.html" {
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
            );
        } else if requested.starts_with("assets/") {
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            );
        }
        return response;
    }

    // Client-side routes fall back to the SPA entry point.
    if !requested.contains('.') {
        if let Some(index) = ASSETS.get_file("index.html") {
            let mut response = (index.contents()).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
            );
            return response;
        }
    }

    StatusCode::NOT_FOUND.into_response()
}
