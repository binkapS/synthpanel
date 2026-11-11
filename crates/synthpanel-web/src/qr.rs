//! A QR code renderer for the two-factor setup.
//!
//! The authenticator apps expect a QR code with the `otpauth://` URI. The
//! code is rendered as an SVG so it stays sharp on every display and does not
//! need an image encoder.

use anyhow::{Result, anyhow};

/// Render `text` as an SVG QR code.
///
/// The SVG contains a single path with one rectangle per dark module, so it
/// is small and scales without loss.
pub fn svg(text: &str) -> Result<String> {
    let code = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium)
        .map_err(|error| anyhow!("cannot encode the QR code: {error}"))?;
    let size = code.size();
    let quiet = 2;
    let dimension = size + quiet * 2;

    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if code.get_module(x, y) {
                path.push_str(&format!("M{},{}h1v1h-1z", x + quiet, y + quiet));
            }
        }
    }

    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {dimension} {dimension}\" \
         shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"Two-factor setup QR code\">\
         <rect width=\"{dimension}\" height=\"{dimension}\" fill=\"#ffffff\"/>\
         <path d=\"{path}\" fill=\"#0f172a\"/></svg>"
    ))
}

/// Render `text` as a `data:` URI that can be used in an `<img>` tag.
pub fn data_uri(text: &str) -> Result<String> {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    let svg = svg(text)?;
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(svg.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_qr_code() {
        let svg = svg("otpauth://totp/SynthPanel:admin@example.com?secret=ABC").unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("<path d=\"M"));
    }

    #[test]
    fn renders_a_data_uri() {
        let uri = data_uri("otpauth://totp/test").unwrap();
        assert!(uri.starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn rejects_text_that_does_not_fit() {
        let long = "x".repeat(8000);
        assert!(svg(&long).is_err());
    }
}
