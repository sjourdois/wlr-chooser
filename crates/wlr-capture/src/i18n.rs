//! Internationalisation via Fluent (`i18n-embed`). Translation catalogs live in
//! `i18n/<lang>/wlr_capture.ftl` (this crate) and are embedded into the binary.
//! The UI language is negotiated from the desktop locale, falling back to English.
//!
//! Use the [`tr!`](crate::tr) macro for message lookups. It performs a runtime
//! lookup against the shared [`LOADER`], so it works unchanged from any crate in
//! the workspace (the catalogs live here, in `wlr-capture`); the
//! `tests::every_catalog_loads` test guards against malformed catalogs.

use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// The process-wide Fluent loader, preloaded with the fallback language.
pub static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader = fluent_language_loader!();
    loader
        .load_fallback_language(&Localizations)
        .expect("fallback language must be present");
    // No bidirectional isolation marks around placeables (we render plain LTR text).
    loader.set_use_isolating(false);
    loader
});

/// Initialise localisation (call once at startup).
///
/// The UI follows the desktop locale (`LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG`),
/// falling back to English; set `LANGUAGE` (e.g. `LANGUAGE=ja`) to override.
pub fn init() {
    let requested = DesktopLanguageRequester::requested_languages();
    let _ = i18n_embed::select(&*LOADER, &Localizations, &requested);
}

/// Look up a Fluent message, optionally with `name = value` arguments.
///
/// Runtime lookup against [`LOADER`]; fully qualified through `$crate` so it works
/// from any crate in the workspace without extra dependencies. Argument values
/// only need to be `Into<FluentValue>` (e.g. `String`, `&str`, integers).
#[macro_export]
macro_rules! tr {
    ($id:literal) => {
        $crate::i18n::LOADER.get($id)
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        // Values keep their own type; `get_args` accepts any `V: Into<FluentValue>`
        // (e.g. `String`, `&str`, integers). One map, so all args share a type.
        let mut args = ::std::collections::HashMap::new();
        $( args.insert(::std::stringify!($name), $value); )+
        $crate::i18n::LOADER.get_args($id, args)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every embedded catalog must parse and load (catches malformed `.ftl`).
    #[test]
    fn every_catalog_loads() {
        let loader = fluent_language_loader!();
        let langs = loader
            .available_languages(&Localizations)
            .expect("list languages");
        assert!(
            langs.len() >= 13,
            "expected ≥13 languages, got {}",
            langs.len()
        );
        for lang in langs {
            loader
                .load_languages(&Localizations, std::slice::from_ref(&lang))
                .unwrap_or_else(|e| panic!("catalog {lang} failed to load: {e}"));
        }
    }
}
