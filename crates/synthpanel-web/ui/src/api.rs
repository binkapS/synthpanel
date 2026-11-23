// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The WebAssembly bridge: same-origin HTTP, storage, sockets and themes.
//!
//! Everything here talks to `window.synthpanel_api`, a small object installed
//! by `slint-loader.js`. Keeping the browser primitives in JavaScript lets the
//! Rust side stay plain `serde_json` and avoids a second cookie jar.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

fn api() -> JsValue {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("synthpanel_api")).unwrap_or(JsValue::NULL)
}

fn function(name: &str) -> Option<js_sys::Function> {
    js_sys::Reflect::get(&api(), &JsValue::from_str(name))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
}

fn call(name: &str, args: &[JsValue]) -> Option<JsValue> {
    let function = function(name)?;
    let arguments = js_sys::Array::new();
    for argument in args {
        arguments.push(argument);
    }
    js_sys::Reflect::apply(&function, &api(), &arguments).ok()
}

/// Perform a same-origin API request through the browser.
///
/// The JS bridge attaches cookies and the CSRF header; the response arrives
/// as JSON `{status, body}`.
pub async fn http(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let body = body
        .map(|value| JsValue::from_str(&value.to_string()))
        .unwrap_or(JsValue::NULL);
    let function = function("request").ok_or_else(|| "missing synthpanel_api.request".to_string())?;
    let arguments = js_sys::Array::new();
    arguments.push(&JsValue::from_str(method));
    arguments.push(&JsValue::from_str(path));
    arguments.push(&body);
    let promise = js_sys::Reflect::apply(&function, &api(), &arguments)
        .map_err(|error| format!("synthpanel_api.request failed: {error:?}"))?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "synthpanel_api.request did not return a promise".to_string())?;

    let value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|error| format!("request failed: {error:?}"))?;
    let text = value
        .as_string()
        .ok_or_else(|| "the API bridge returned a non-string response".to_string())?;
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
            .or_else(|| {
                envelope
                    .get("body")
                    .and_then(|body| body.get("message"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("request failed");
        return Err(message.to_string());
    }
    Ok(envelope.get("body").cloned().unwrap_or(serde_json::Value::Null))
}

pub fn storage_get(key: &str) -> Option<String> {
    call("storage_get", &[JsValue::from_str(key)]).and_then(|value| value.as_string())
}

pub fn storage_set(key: &str, value: &str) {
    let _ = call("storage_set", &[JsValue::from_str(key), JsValue::from_str(value)]);
}

pub fn apply_theme(theme: &str) {
    let _ = call("apply_theme", &[JsValue::from_str(theme)]);
}

/// Open a WebSocket handled by the JS bridge. `id` is an arbitrary handle.
pub fn socket_open(id: u32, path: &str) {
    let _ = call("socket_open", &[JsValue::from(id), JsValue::from_str(path)]);
}

pub fn socket_send(id: u32, data: &str) {
    let _ = call("socket_send", &[JsValue::from(id), JsValue::from_str(data)]);
}

pub fn socket_close(id: u32) {
    let _ = call("socket_close", &[JsValue::from(id)]);
}

/// Register `callback` as `window[name]`. The closure is kept alive for the
/// lifetime of the page.
pub fn set_global(name: &str, callback: wasm_bindgen::closure::Closure<dyn FnMut(JsValue)>) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        callback.as_ref(),
    );
    callback.forget();
}
