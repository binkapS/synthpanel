# Deprecated

This Svelte 5 SPA is the pre-migration frontend. The production UI is now the
Slint WebAssembly app in `../ui/` (built by `ui/build-ui.sh` and embedded by
`crates/synthpanel-web/build.rs`).

The directory is kept temporarily as a reference while the Slint UI reaches
full acceptance parity and E2E coverage is updated. Do not add new features
here; new UI work belongs in `crates/synthpanel-web/ui/`.
