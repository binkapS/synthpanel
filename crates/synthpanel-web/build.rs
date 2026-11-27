//! Build script for the embedded Slint UI.
//!
//! `cargo build -p synthpanel-web` builds `ui/` for wasm32 and runs
//! `wasm-bindgen` so the resulting assets can be embedded with `include_dir!`.
//! Set `SYNTHPANEL_SKIP_UI_BUILD=1` to reuse the existing `ui/dist` (useful
//! when iterating on server code), or `SYNTHPANEL_UI_DIR` to point elsewhere.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ui_dir = std::env::var("SYNTHPANEL_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("ui"));
    let dist = ui_dir.join("dist");

    // Rebuild when any source changes. Watching the whole ui directory also
    // covers new pages, widgets and icons.
    println!("cargo:rerun-if-changed={}", ui_dir.display());
    println!("cargo:rerun-if-env-changed=SYNTHPANEL_SKIP_UI_BUILD");
    println!("cargo:rerun-if-env-changed=WASM_BINDGEN");

    let skip = std::env::var_os("SYNTHPANEL_SKIP_UI_BUILD").is_some();
    let stale = !dist.join("index.html").exists() || is_stale(&ui_dir, &dist);

    if !skip && stale {
        if let Err(error) = build_ui(&ui_dir) {
            println!("cargo:warning=Slint UI build failed: {error}");
            println!("cargo:warning=Embedding a placeholder UI; rerun ui/build-ui.sh to fix it.");
        }
    }

    ensure_placeholder(&dist);
}

/// Whether any UI source is newer than the last successful build.
fn is_stale(ui_dir: &Path, dist: &Path) -> bool {
    let stamp = match std::fs::metadata(dist.join(".stamp")).and_then(|meta| meta.modified()) {
        Ok(stamp) => stamp,
        Err(_) => return true,
    };
    newest_mtime(&ui_dir.join("app.slint"))
        .into_iter()
        .chain(newest_mtime(&ui_dir.join("widgets")))
        .chain(newest_mtime(&ui_dir.join("pages")))
        .chain(newest_mtime(&ui_dir.join("locales")))
        .chain(newest_mtime(&ui_dir.join("assets")))
        .chain(newest_mtime(&ui_dir.join("src")))
        .chain(newest_mtime(&ui_dir.join("web")))
        .chain(newest_mtime(&ui_dir.join("build.rs")))
        .chain(newest_mtime(&ui_dir.join("Cargo.toml")))
        .any(|mtime| mtime > stamp)
}

fn newest_mtime(path: &Path) -> Option<SystemTime> {
    if path.is_file() {
        return std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
    }
    let entries = std::fs::read_dir(path).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name == "target" || name == "dist")
            {
                return None;
            }
            newest_mtime(&path)
        })
        .max()
}

fn build_ui(ui_dir: &Path) -> Result<(), String> {
    let script = ui_dir.join("build-ui.sh");
    let status = Command::new("bash")
        .arg(&script)
        .current_dir(ui_dir)
        .status()
        .map_err(|error| format!("could not run {}: {error}", script.display()))?;
    if !status.success() {
        return Err(format!("{} exited with {status}", script.display()));
    }
    Ok(())
}

/// `include_dir!` needs at least an index.html at compile time. If the WASM
/// toolchain is unavailable we still want `cargo build` to succeed with a
/// readable diagnostic instead of a compile error.
fn ensure_placeholder(dist: &Path) {
    if dist.join("index.html").exists() {
        return;
    }
    let _ = std::fs::create_dir_all(dist);
    let html = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>SynthPanel</title></head>
<body style="font:14px system-ui;background:#0f172a;color:#e2e8f0;padding:2rem">
<h1>SynthPanel UI not built</h1>
<p>The Slint WebAssembly bundle is missing. Build it with:</p>
<pre>cargo install wasm-bindgen-cli --version 0.2.130
rustup target add wasm32-unknown-unknown
crates/synthpanel-web/ui/build-ui.sh</pre>
<p>Then rebuild <code>synthpanel-web</code>.</p>
</body></html>
"#;
    let _ = std::fs::write(dist.join("index.html"), html);
}
