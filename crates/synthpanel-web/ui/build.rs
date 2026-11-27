fn main() {
    // Bundle the English reference catalog (and any future locales) into the
    // generated Rust. `@tr()` strings fall back to their source text when no
    // translation exists, so English works with no runtime gettext support.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("locales");
    slint_build::compile_with_config("app.slint", config)
        .expect("failed to compile the Slint UI");
}
