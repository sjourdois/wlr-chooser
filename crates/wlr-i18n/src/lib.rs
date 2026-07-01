//! Shared i18n infrastructure for the wlr-utils tools.
//!
//! Each tool crate owns its own Fluent catalog (`i18n/<lang>/<crate>.ftl`) and its own
//! loader — so `wlr-capture` (the engine library) carries no UI strings at all. This crate
//! provides the reusable plumbing so a crate's `i18n` module stays a few lines:
//!
//! - with the `i18n` feature (default) it wraps `i18n-embed` / Fluent ([`build_loader`],
//!   [`select`]);
//! - without it, [`build::generate_fallback`] turns the crate's `en` catalog into a plain
//!   `fallback(id)` function at build time, so English-only builds pull in no Fluent stack.
//!
//! A consuming crate defines a `RustEmbed` `Localizations` over its `i18n/` folder, a
//! `LOADER` built with [`build_loader`], an `init()` calling [`select`], and its own `tr!`
//! macro bound to that `LOADER`.

#![warn(missing_docs)]

#[cfg(feature = "i18n")]
mod runtime {
    pub use i18n_embed::I18nAssets;
    pub use i18n_embed::fluent::FluentLanguageLoader;
    pub use rust_embed;
    use i18n_embed::{DesktopLanguageRequester, LanguageLoader};

    /// Build a Fluent loader for `domain` (the catalog stem, e.g. `wlr_draw`) with English
    /// as the fallback language, preloading the `en` catalog from `assets`. Use it to
    /// initialise a crate's `LOADER` static.
    pub fn build_loader(domain: &str, assets: &dyn I18nAssets) -> FluentLanguageLoader {
        let en: unic_langid::LanguageIdentifier =
            "en".parse().expect("`en` is a valid language id");
        let loader = FluentLanguageLoader::new(domain, en);
        // Plain LTR text — no bidirectional isolation marks around placeables.
        loader.set_use_isolating(false);
        loader
            .load_fallback_language(assets)
            .expect("the `en` fallback catalog must be present");
        loader
    }

    /// Negotiate the desktop locale (`LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG`) into
    /// `loader`, falling back to English. Call once at startup.
    pub fn select(loader: &FluentLanguageLoader, assets: &dyn I18nAssets) {
        let requested = DesktopLanguageRequester::requested_languages();
        let _ = i18n_embed::select(loader, assets, &requested);
    }
}

#[cfg(feature = "i18n")]
pub use runtime::{FluentLanguageLoader, I18nAssets, build_loader, rust_embed, select};

/// Build-script helper for the English fallback used when the `i18n` feature is off. Pure
/// `std`; a consuming crate depends on this crate `default-features = false` as a
/// build-dependency and calls [`generate_fallback`] from its `build.rs`.
pub mod build {
    use std::fmt::Write as _;
    use std::path::Path;

    /// Parse a crate's `en` Fluent catalog (simple one-line `key = value` entries) into a
    /// `fallback(id, args) -> String` function written to `$OUT_DIR/i18n_fallback.rs`,
    /// substituting `{ $name }` placeables from `args`. Also re-runs when `i18n/` changes.
    pub fn generate_fallback(en_catalog: &str) {
        println!("cargo:rerun-if-changed=i18n");
        let src = std::fs::read_to_string(en_catalog)
            .unwrap_or_else(|e| panic!("reading {en_catalog}: {e}"));
        let mut arms = String::new();
        for line in src.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            writeln!(arms, "        {:?} => {:?},", key.trim(), value.trim()).unwrap();
        }
        let code = format!(
            "/// English fallback text generated from the `en` Fluent catalog.\n\
             pub fn fallback(id: &str, args: &[(&'static str, String)]) -> String {{\n\
             \x20   let template: &str = match id {{\n\
             {arms}\
             \x20       _ => id,\n\
             \x20   }};\n\
             \x20   let mut out = template.to_string();\n\
             \x20   for (name, value) in args {{\n\
             \x20       out = out.replace(&format!(\"{{{{ ${{name}} }}}}\"), value);\n\
             \x20   }}\n\
             \x20   out\n\
             }}\n"
        );
        let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("i18n_fallback.rs");
        std::fs::write(out, code).expect("writing i18n_fallback.rs");
    }
}
