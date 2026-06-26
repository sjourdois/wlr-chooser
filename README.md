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

- A wlroots compositor exposing `ext-image-copy-capture-v1` with the output **and**
  foreign-toplevel capture sources, plus `ext-foreign-toplevel-list-v1` (and
  `wlr-layer-shell` for the overlays) — **Sway ≥ 1.12 / wlroots ≥ 0.20**, the floor for
  the window source the tools open. See [COMPATIBILITY.md](COMPATIBILITY.md) for the full
  matrix (Hyprland, niri, …), or run `wlr-peek doctor` to check your own compositor.
- **GL stack** — every tool renders overlays through EGL/GLES, so `libegl1` is needed at
  runtime. `wlr-chooser` also builds the zero-copy **GPU path** by default, which adds
  `libgbm` (Mesa); `wlr-shot` and `wlr-peek` capture via CPU shm and need no `libgbm`.
- `wlr-chooser` also needs `xdg-desktop-portal-wlr` ≥ 0.8 (portal use);
  `wlr-switcher` needs `zwlr-foreign-toplevel-management-v1` to focus windows.

## Install

**Everything at once** — the `wlr-utils` crate bundles all five binaries
(`wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`, `wlr-draw`):

```sh
cargo install wlr-utils
```

Or grab the **prebuilt bundle** from the
[latest release](https://github.com/sjourdois/wlr-utils/releases/latest) — one archive
with every binary, plus a one-line installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/sjourdois/wlr-utils/releases/latest/download/wlr-utils-installer.sh | sh
```

**À la carte** — each tool is also its own crate, for a lighter, single-purpose install
(only the system deps that tool needs). Per-tool instructions live in each crate's README:

```sh
cargo install wlr-chooser        # window/screen picker + wlr-switcher (Alt-Tab/exposé)
cargo install wlr-peek           # colour picker, loupe, OCR, live mirror, watch
cargo install wlr-shot           # screenshots + recording
cargo install wlr-draw           # annotation overlay
```

A single `.deb` (the whole suite) is also attached to every release. To build the whole
workspace from source (the `gpu` feature needs `libgbm-dev` at build time):

```sh
cargo build --release            # builds all binaries
```

### Uninstall

`cargo install` drops the binaries in `~/.cargo/bin`. Remove the bundle with
`cargo uninstall wlr-utils`, or an individual tool the same way:

```sh
cargo uninstall wlr-utils        # the whole bundle
cargo uninstall wlr-draw         # …or just one: wlr-chooser / wlr-peek / wlr-shot
```

`wlr-draw` also registers an XDG autostart entry on first run (see its README). Drop the
checkbox in its tray menu, or delete the files by hand (honouring `$XDG_CONFIG_HOME` /
`$XDG_STATE_HOME` if you set them):

```sh
rm -f ~/.config/autostart/wlr-draw.desktop \
      ~/.local/state/wlr-draw/autostart-initialized
# and, if you installed the systemd unit:
systemctl --user disable --now wlr-draw.service
rm -f ~/.config/systemd/user/wlr-draw.service
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
