# SynthPanel UI translations

Slint 1.19 translates strings marked with `@tr("...")` in `.slint` files.
English is the **reference language**: the strings in the `.slint` sources are
the msgids, and `@tr` falls back to the source string when no translation is
installed. No catalog has to be loaded for English.

## Layout

```
locales/
├── README.md                        (this file)
└── en/LC_MESSAGES/
    ├── synthpanel-ui.po             default catalog (domain = Cargo package name)
    └── synthpanel.po                same catalog for domain "synthpanel"
```

Slint derives the translation domain from `CARGO_PKG_NAME` (here
`synthpanel-ui`) and looks for `<locale>/LC_MESSAGES/<domain>.po` when
translations are bundled at compile time, or `<domain>.mo` when they are
loaded at runtime through gettext. Both filenames are provided so the catalog
works with either the default domain or an explicitly configured
`synthpanel` domain. Keep the two files in sync (or delete the one you do not
use) once the domain is fixed in `build.rs`.

The `msgctxt` values are Slint's default translation context: the name of the
component containing the `@tr` (`AppShell` for the sidebar/header,
`LoginPage` for the sign-in card). If you regenerate catalogs with
`slint-tr-extractor --no-default-translation-context`, remove the `msgctxt`
lines in both the catalogs and the `@tr` calls.

## Adding a locale

1. Copy `en/LC_MESSAGES/<domain>.po` to `<locale>/LC_MESSAGES/<domain>.po`
   (e.g. `de/LC_MESSAGES/synthpanel-ui.po`).
2. Fill in the `msgstr` fields; leave `msgid`/`msgctxt` untouched. Keep the
   `Plural-Forms` header accurate for the language.
3. For compile-time bundling, nothing else is needed: the `.po` file itself is
   compiled into the WASM by `slint-build` (host tooling such as
   `slint-tr-extractor` can be used to refresh msgids when UI strings change).
4. For runtime gettext, compile the binary catalog with
   `msgfmt -o <locale>/LC_MESSAGES/<domain>.mo <locale>/LC_MESSAGES/<domain>.po`
   and ship the `locales/` folder next to the app.

## Wiring (handled outside this folder)

- Compile-time bundling (works on WASM, no gettext runtime): in `build.rs`,
  use `slint_build::compile_with_config("app.slint",
  slint_build::CompilerConfiguration::new().with_bundled_translations("locales"))`.
  `slint-build` 1.19 has the `bundle-translations` compiler feature enabled by
  default, so no Cargo feature changes are required. Alternatively set the
  `SLINT_BUNDLE_TRANSLATIONS=locales` environment variable for the build.
- Runtime gettext: enable the `gettext` feature on the `slint` dependency and
  call `slint::init_translations!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales"))`
  before creating any UI. This needs the `msgfmt`-produced `.mo` files and is
  generally unsuitable for a browser/WASM target, where bundling is preferred.
- `synthpanel-ui.po` is the catalog that works as-is: `slint-build` derives the
  domain from `CARGO_PKG_NAME` and does not expose a public API to override it
  in 1.19. `synthpanel.po` is provided for the case where the package is renamed
  to `synthpanel` (or a custom compiler configuration sets that domain); both
  gettext and bundling must use the same domain.

Nothing in `Cargo.toml` or `build.rs` was changed as part of the UI scaffold;
English rendering is unaffected until a catalog is wired in.
