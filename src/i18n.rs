//! Internationalisation via Fluent (`i18n-embed`). Translation catalogs live in
//! `i18n/<lang>/wlr_chooser.ftl` and are embedded into the binary. The UI language is
//! negotiated from the desktop locale, falling back to English.
//!
//! Use the [`tr!`](crate::tr) macro for message lookups; it is checked against the
//! fallback catalog at compile time.

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
#[macro_export]
macro_rules! tr {
    ($id:literal) => { i18n_embed_fl::fl!($crate::i18n::LOADER, $id) };
    ($id:literal, $($args:tt)*) => { i18n_embed_fl::fl!($crate::i18n::LOADER, $id, $($args)*) };
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
