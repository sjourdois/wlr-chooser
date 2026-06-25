# wlr-utils

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Showcase](https://img.shields.io/badge/▶_showcase-sjourdois.github.io%2Fwlr--utils-8aadf4)](https://sjourdois.github.io/wlr-utils/)

### Capture · switch · inspect · annotate your screen — the native Wayland way.

⚡ Zero-copy GPU capture &nbsp;·&nbsp; 👁️ Sees occluded & off-workspace windows
&nbsp;·&nbsp; 🦀 Rust, no XWayland &nbsp;·&nbsp; 🎨 Themeable &nbsp;·&nbsp; 🌍 13 languages

Five sharp tools for **wlroots** compositors, all sharing one capture engine.

| Tool | What it does | crate |
| --- | --- | --- |
| **[wlr-chooser](crates/wlr-chooser)** | Window & screen picker for screencast portals (`xdg-desktop-portal-wlr`) — a rofi-like overlay with live thumbnails. | [![v](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser) |
| **[wlr-switcher](crates/wlr-chooser)** | Live **Alt-Tab / exposé** window switcher (macOS-style strip, full-screen grid, or card) with hold-to-switch and live previews. Ships with `wlr-chooser`. | [![v](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser) |
| **[wlr-peek](crates/wlr-peek)** | **Inspect the screen** — colour picker, loupe, OCR, live picture-in-picture **mirror** (window or region), **change monitor** (`watch`), and **visual grep**. | [![v](https://img.shields.io/crates/v/wlr-peek.svg)](https://crates.io/crates/wlr-peek) |
| **[wlr-shot](crates/wlr-shot)** | **Screen capture** — screenshots of an output/region/window (PNG/JPEG/PPM), copy to clipboard; plus **recording** (H.264, or animated GIF/WebP) with **system audio** & **timelapse** (NVENC/VAAPI/libx264). | [![v](https://img.shields.io/crates/v/wlr-shot.svg)](https://crates.io/crates/wlr-shot) |
| **[wlr-draw](crates/wlr-draw)** | **Draw on screen** — a transparent annotation overlay (gromit-mpx-style): freehand, shapes, arrows, text, dwell-to-snap, element move, plus presenter **spotlight**, **freeze-frame** and **save**. Daemon + control socket. | [![v](https://img.shields.io/crates/v/wlr-draw.svg)](https://crates.io/crates/wlr-draw) |
| **[wlr-pip](crates/wlr-pip)** | _Deprecated_ — the live mirror moved to `wlr-peek mirror`; this is a stub pointing there. | [![v](https://img.shields.io/crates/v/wlr-pip.svg)](https://crates.io/crates/wlr-pip) |

They all share **[wlr-capture](crates/wlr-capture)**, a library with the wlroots
capture engine (`ext-image-copy-capture-v1`, full-resolution dma-buf zero-copy
with a CPU shm fallback) and an egui/EGL rendering + dma-buf-import toolkit.

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-draw/annotate.gif" width="49%" alt="wlr-draw — annotate live on screen">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-switcher/altab.gif" width="49%" alt="wlr-switcher — Alt-Tab with live previews">
</p>
<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-shot/select.gif" width="49%" alt="wlr-shot — frozen region selector">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-peek/color.gif" width="49%" alt="wlr-peek — colour picker with loupe">
</p>
<p align="center"><sub>wlr-draw · wlr-switcher · wlr-shot · wlr-peek — see the <a href="https://sjourdois.github.io/wlr-utils/">showcase</a></sub></p>

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

Each tool is its own crate — install the ones you want. Per-tool instructions
live in each crate's README. In short:

```sh
cargo install wlr-chooser        # window/screen picker + wlr-switcher (Alt-Tab/exposé)
cargo install wlr-peek           # colour picker, loupe, OCR, live mirror, watch
cargo install wlr-shot           # screenshots + recording
cargo install wlr-draw           # annotation overlay

# …or all at once:
cargo install wlr-chooser wlr-peek wlr-shot wlr-draw
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
- **[wlr-peek README](crates/wlr-peek/README.md)** — colour picker, loupe, OCR, live
  mirror, change monitor and visual grep.
- **[wlr-shot README](crates/wlr-shot/README.md)** — screenshots, recording and
  timelapse, with system audio and hardware encoding.
- **[wlr-draw README](crates/wlr-draw/README.md)** — the annotation overlay: daemon,
  control socket, tools and example key bindings.
- **[wlr-capture README](crates/wlr-capture/README.md)** — the shared engine.

## Contributing

Bug reports, translations and patches welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md). Please keep `cargo fmt`, `cargo clippy` and
`cargo test` clean.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.
