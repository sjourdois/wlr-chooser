# wlr-utils

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A small set of **wlroots** desktop utilities built around one zero-copy GPU
capture engine: enumerate and capture toplevels/outputs (even occluded or on other
workspaces), live and near-free.

| Tool | What it does | crate |
| --- | --- | --- |
| **[wlr-chooser](crates/wlr-chooser)** | Window & screen picker for screencast portals (`xdg-desktop-portal-wlr`) — a rofi-like overlay with live thumbnails. | [![v](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser) |
| **[wlr-switcher](crates/wlr-chooser)** | Live **Alt-Tab / exposé** window switcher (macOS-style strip, full-screen grid, or card) with hold-to-switch and live previews. Ships with `wlr-chooser`. | [![v](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser) |
| **[wlr-peek](crates/wlr-peek)** | **Inspect the screen** — colour picker, loupe, OCR, live picture-in-picture **mirror** (window or region), **change monitor** (`watch`), and **visual grep**. | [![v](https://img.shields.io/crates/v/wlr-peek.svg)](https://crates.io/crates/wlr-peek) |
| **[wlr-shot](crates/wlr-shot)** | **Screen capture** — screenshots of an output/region/window (PNG/JPEG/PPM), copy to clipboard; plus H.264 **recording** & **timelapse** (NVENC/VAAPI/libx264). | [![v](https://img.shields.io/crates/v/wlr-shot.svg)](https://crates.io/crates/wlr-shot) |
| **[wlr-pip](crates/wlr-pip)** | _Deprecated_ — the live mirror moved to `wlr-peek mirror`; this is a stub pointing there. | [![v](https://img.shields.io/crates/v/wlr-pip.svg)](https://crates.io/crates/wlr-pip) |

Both share **[wlr-capture](crates/wlr-capture)**, a library with the wlroots
capture engine (`ext-image-copy-capture-v1`, full-resolution dma-buf zero-copy
with a CPU shm fallback) and an egui/EGL rendering + dma-buf-import toolkit.

![wlr-chooser overlay](docs/screenshots/overlay.png)

## Requirements

- A wlroots compositor exposing `ext-image-copy-capture-v1`,
  `ext-image-capture-source-v1`, `ext-foreign-toplevel-list-v1` (and
  `wlr-layer-shell` for `wlr-chooser`) — Sway ≥ 1.12 / wlroots ≥ 0.20. See
  [COMPATIBILITY.md](COMPATIBILITY.md) for the full matrix (Hyprland, niri, …), or
  run `wlr-peek doctor` to check your own compositor.
- For the **GPU path** (default): a working EGL/GLES driver and `libgbm` (Mesa).
  Falls back to CPU shm automatically.
- `wlr-chooser` also needs `xdg-desktop-portal-wlr` ≥ 0.8 (portal use);
  `wlr-switcher` needs `zwlr-foreign-toplevel-management-v1` to focus windows.

## Install

Per-tool instructions live in each crate's README. In short:

```sh
cargo install wlr-chooser        # the picker
cargo install wlr-pip            # the PiP mirror
```

Prebuilt binaries, installer scripts and `.deb` packages are attached to every
[release](https://github.com/sjourdois/wlr-utils/releases/latest). To build the
whole workspace from source (the `gpu` feature needs `libgbm-dev` at build time):

```sh
cargo build --release            # builds all binaries
```

## Documentation

- **[wlr-chooser README](crates/wlr-chooser/README.md)** — portal setup, options,
  the `wlr-switcher` Alt-Tab/exposé, theming and localisation.
- **[wlr-pip README](crates/wlr-pip/README.md)** — usage, Sway rules, controls and
  keyboard shortcuts.
- **[wlr-capture README](crates/wlr-capture/README.md)** — the shared engine.

## Contributing

Bug reports, translations and patches welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md). Please keep `cargo fmt`, `cargo clippy` and
`cargo test` clean.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.
