// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Slint entry point for the SynthPanel web UI.
//!
//! The component tree lives in `app.slint` and its `widgets/` and `pages/`
//! modules. This crate only contains the WebAssembly bridge: it translates
//! Slint callbacks into same-origin API requests and pushes the results back
//! into Slint models.

slint::include_modules!();

#[cfg(target_arch = "wasm32")]
mod bridge {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = ["window", "synthpanel_api"], js_name = request)]
        fn js_request(method: &str, path: &str, body: Option<String>) -> js_sys::Promise;

        #[wasm_bindgen(js_namespace = ["window", "synthpanel_api"], js_name = storage_get)]
        fn js_storage_get(key: &str) -> Option<String>;

        #[wasm_bindgen(js_namespace = ["window", "synthpanel_api"], js_name = storage_set)]
        fn js_storage_set(key: &str, value: &str);

        #[wasm_bindgen(js_namespace = ["window", "synthpanel_api"], js_name = apply_theme)]
        fn js_apply_theme(theme: &str);
    }

    /// Perform a same-origin API request through the browser.
    pub async fn api(
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let body = body.map(|value| value.to_string());
        let promise = js_request(method, path, body);
        let value = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|error| format!("request failed: {error:?}"))?;
        let text = value
            .dyn_into::<js_sys::JsString>()
            .map_err(|_| "the API bridge returned a non-string response".to_string())?
            .as_string()
            .ok_or_else(|| "the API bridge returned an empty response".to_string())?;
        let envelope: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| format!("invalid API response: {error}"))?;
        let status = envelope
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if status >= 400 {
            let message = envelope
                .get("body")
                .and_then(|body| body.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed");
            return Err(message.to_string());
        }
        Ok(envelope
            .get("body")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    pub fn storage_get(key: &str) -> Option<String> {
        js_storage_get(key)
    }

    pub fn storage_set(key: &str, value: &str) {
        js_storage_set(key, value)
    }

    pub fn apply_theme(theme: &str) {
        js_apply_theme(theme)
    }
}

/// Bootstrap the UI. `wasm-bindgen` calls this automatically when the module
/// is initialised by `slint-loader.js`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    let app = AppWindow::new().map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;

    let dark = bridge::storage_get("synthpanel-theme").as_deref() == Some("dark");
    app.set_dark(dark);
    bridge::apply_theme(if dark { "dark" } else { "light" });

    app.run()
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}
