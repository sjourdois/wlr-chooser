//! wlr-peek's localised UI strings. Catalog: `i18n/<lang>/wlr_peek.ftl`; loader plumbing in
//! [`wlr_i18n`]. Use the [`tr!`](crate::tr) macro for lookups.

#[cfg(feature = "i18n")]
mod imp {
    use rust_embed::RustEmbed;
    use std::sync::LazyLock;
    use wlr_i18n::FluentLanguageLoader;

    #[derive(RustEmbed)]
    #[folder = "i18n/"]
    struct Localizations;

    /// This crate's process-wide Fluent loader, preloaded with the English fallback.
    pub static LOADER: LazyLock<FluentLanguageLoader> =
        LazyLock::new(|| wlr_i18n::build_loader("wlr_peek", &Localizations));

    /// Negotiate the desktop locale (call once at startup).
    pub fn init() {
        wlr_i18n::select(&LOADER, &Localizations);
    }
}

#[cfg(feature = "i18n")]
pub use imp::{LOADER, init};

#[cfg(not(feature = "i18n"))]
mod imp {
    // `fallback(id, args) -> String`, generated from the `en` catalog by `build.rs`.
    include!(concat!(env!("OUT_DIR"), "/i18n_fallback.rs"));

    /// No-op: there is no locale to negotiate without Fluent.
    pub fn init() {}
}

#[cfg(not(feature = "i18n"))]
pub use imp::{fallback, init};

/// Look up a UI message, optionally with `name = value` arguments.
#[cfg(feature = "i18n")]
#[macro_export]
macro_rules! tr {
    ($id:literal) => {
        $crate::i18n::LOADER.get($id)
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = ::std::collections::HashMap::new();
        $( args.insert(::std::stringify!($name), $value); )+
        $crate::i18n::LOADER.get_args($id, args)
    }};
}

/// English-only fallback variant (no `i18n` feature).
#[cfg(not(feature = "i18n"))]
#[macro_export]
macro_rules! tr {
    ($id:literal) => {
        $crate::i18n::fallback($id, &[])
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fallback(
            $id,
            &[ $( (::std::stringify!($name), ::std::format!("{}", $value)) ),+ ],
        )
    };
}
