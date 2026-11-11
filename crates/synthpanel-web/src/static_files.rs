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
        let mime = content_type(&file.path().to_string_lossy());
        let mut response = (file.contents()).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime)
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
        } else if requested == "sw.js" {
            // The service worker must never be served from a stale cache.
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
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

/// The content type of an embedded asset.
///
/// `mime_guess` does not know the web application manifest type, so it is
/// mapped explicitly.
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "webmanifest" => "application/manifest+json",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => {
            let guessed = mime_guess::from_path(path).first_or_octet_stream();
            let text = guessed.essence_str().to_string();
            match text.as_str() {
                "text/plain" => "text/plain; charset=utf-8",
                "application/octet-stream" => "application/octet-stream",
                _ => "application/octet-stream",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_manifest_type() {
        assert_eq!(
            content_type("manifest.webmanifest"),
            "application/manifest+json"
        );
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/app.js"),
            "text/javascript; charset=utf-8"
        );
    }
}
