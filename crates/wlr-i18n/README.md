# wlr-i18n

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/wlr-i18n.svg)](https://crates.io/crates/wlr-i18n)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Shared localisation plumbing for the [wlr-utils](https://github.com/sjourdois/wlr-utils)
tools (`wlr-chooser`, `wlr-peek`, `wlr-shot`, `wlr-draw`).

Each tool crate **owns its own** Fluent catalog (`i18n/<lang>/<crate>.ftl`) and its own
loader — so [`wlr-capture`](../wlr-capture), the engine library, carries no UI strings at
all. This crate is the reusable core that keeps a tool's `i18n` module down to a few lines.

## What it gives you

- **With the `i18n` feature (default)** — a thin wrapper over
  [`i18n-embed`](https://crates.io/crates/i18n-embed) / Fluent:
  - `build_loader(domain, assets)` — a `FluentLanguageLoader` for `domain` (the catalog
    stem, e.g. `wlr_draw`), English preloaded as the fallback, bidi isolation off.
  - `select(loader, assets)` — negotiate the desktop locale
    (`LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG`), falling back to English. Call once at
    startup.
- **Without it (`--no-default-features`)** — a build-script helper,
  `build::generate_fallback("i18n/en/<crate>.ftl")`, that turns the crate's `en` catalog
  into a plain `fallback(id, args) -> String` function at build time. English-only builds
  then pull in **no Fluent stack** at all.

## Using it in a tool crate

A consuming crate defines a `RustEmbed` `Localizations` over its `i18n/` folder, a `LOADER`
built with `build_loader`, an `init()` that calls `select`, and its own `tr!` macro bound to
that `LOADER`:

```rust
use rust_embed::RustEmbed;
use std::sync::LazyLock;
use wlr_i18n::FluentLanguageLoader;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LOADER: LazyLock<FluentLanguageLoader> =
    LazyLock::new(|| wlr_i18n::build_loader("wlr_draw", &Localizations));

pub fn init() {
    wlr_i18n::select(&LOADER, &Localizations);
}
```

For the English-only path, depend on this crate `default-features = false` as a
**build-dependency** and call the generator from `build.rs`:

```rust
// build.rs
fn main() {
    wlr_i18n::build::generate_fallback("i18n/en/wlr_draw.ftl");
}
```

See any of the tool crates (e.g. [`wlr-draw`](../wlr-draw)) for the full pattern, including
the `tr!` macro that binds lookups to `LOADER`.

## Status

Infrastructure crate for the wlr-utils binaries; the public API is not yet stabilised and
may change between minor versions. It is published so the tools can depend on it from
crates.io.

## License

Licensed under either of [Apache-2.0](../../LICENSE-APACHE) or
[MIT](../../LICENSE-MIT) at your option.
