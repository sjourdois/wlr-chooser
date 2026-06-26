# wlr-utils

The **[wlr-utils](https://github.com/sjourdois/wlr-utils)** suite in a single install —
five sharp screen tools for **wlroots** compositors, all sharing one capture engine.

```sh
cargo install wlr-utils
```

This installs every binary at once:

| Binary | What it does |
| --- | --- |
| `wlr-chooser` | Window & screen picker for `xdg-desktop-portal-wlr` (rofi-like, live thumbnails). |
| `wlr-switcher` | Alt-Tab / exposé window switcher with live previews. |
| `wlr-peek` | Inspect the screen — colour picker, loupe, OCR, live mirror, change monitor, visual grep. |
| `wlr-shot` | Screenshots (PNG/JPEG/PPM) and recording (H.264, GIF/WebP) with system audio. |
| `wlr-draw` | Draw on screen — annotation overlay with shapes, text, spotlight, freeze-frame. |

This crate is just a **bundle**: it ships no library and no logic of its own, only thin
binaries that re-export each tool. Because it pulls every tool with its default features,
the build needs all of their system dependencies (GPU/`libgbm`, FFmpeg, PipeWire,
Tesseract, D-Bus). For a lighter, single-purpose install, install a tool on its own —
`cargo install wlr-shot` — and read that crate's README for its exact requirements.

## Prebuilt bundle

Every [release](https://github.com/sjourdois/wlr-utils/releases/latest) ships one archive
containing all the binaries, plus a one-line installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/sjourdois/wlr-utils/releases/latest/download/wlr-utils-installer.sh | sh
```

## Uninstall

```sh
cargo uninstall wlr-utils
```

`wlr-draw` registers an XDG autostart entry on first run — see the
[main README](https://github.com/sjourdois/wlr-utils#uninstall) for the leftover files to
remove.

## Documentation

Per-tool docs, requirements and options live in each crate's README:
[wlr-chooser](https://github.com/sjourdois/wlr-utils/tree/main/crates/wlr-chooser) ·
[wlr-peek](https://github.com/sjourdois/wlr-utils/tree/main/crates/wlr-peek) ·
[wlr-shot](https://github.com/sjourdois/wlr-utils/tree/main/crates/wlr-shot) ·
[wlr-draw](https://github.com/sjourdois/wlr-utils/tree/main/crates/wlr-draw).

## License

MIT OR Apache-2.0.
