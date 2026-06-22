# wlr-capture

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/wlr-capture.svg)](https://crates.io/crates/wlr-capture)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

The shared engine behind the [wlr-utils](https://github.com/sjourdois/wlr-utils)
tools ([`wlr-chooser`], [`wlr-pip`]). Two reusable bricks plus the overlay UI
helpers they share:

- **`wl`** — a native Wayland client that enumerates foreign toplevels and outputs
  (`ext-foreign-toplevel-list-v1`) and captures them at full resolution via
  `ext-image-capture-source-v1` + `ext-image-copy-capture-v1`. It computes the
  format-correct stride (so it works where `grim 1.5` fails with "Invalid
  stride"), and prefers a zero-copy GPU dma-buf path (allocated through `gbm`)
  with an automatic CPU shm fallback. Capture is occlusion-independent and
  damage-driven (windows on other workspaces stream live).
- **`render`** — an egui → `egui_glow` rendering core on an EGL/GLES context bound
  to a `wl_surface`, plus zero-copy dma-buf → GL texture import
  (`EGL_EXT_image_dma_buf_import`). Any windowing host binds a `Gpu` to its
  surface and drives one egui frame per repaint.
- **`theme` / `i18n` / `icons`** — TOML theming, Fluent localisation (13
  languages), and `.desktop`/icon-theme app-icon resolution.

## Status

This is primarily an **internal library** for the wlr-utils binaries; the public
API is not yet stabilised and may change between minor versions. It is published
so the tools can depend on it from crates.io. The `gpu` feature (on by default)
pulls in `gbm` for the dma-buf path; disable it (`--no-default-features`) for a
pure-CPU build.

## License

Licensed under either of [Apache-2.0](../../LICENSE-APACHE) or
[MIT](../../LICENSE-MIT) at your option.

[`wlr-chooser`]: https://crates.io/crates/wlr-chooser
[`wlr-pip`]: https://crates.io/crates/wlr-pip
