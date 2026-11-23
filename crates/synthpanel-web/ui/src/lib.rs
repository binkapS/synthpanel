// Copyright (C) 2026 Binkap S
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Slint entry point for the SynthPanel web UI.
//!
//! The component tree lives in `app.slint` and its `widgets/` and `pages/`
//! modules. This crate contains the WebAssembly bridge: it translates Slint
//! callbacks into same-origin API requests and pushes the results back into
//! Slint models.

slint::include_modules!();

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod terminal;
#[cfg(target_arch = "wasm32")]
mod models;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() -> Result<(), wasm_bindgen::JsValue> {
    app::install()
}
