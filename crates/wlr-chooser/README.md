# wlr-chooser

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Part of the [wlr-utils](https://github.com/sjourdois/wlr-utils) workspace.

A graphical window & screen picker for **wlroots** screencast portals
(`xdg-desktop-portal-wlr`) — a rofi-like overlay with **live thumbnails**.

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-chooser/picker.png"
       alt="wlr-chooser: a grid of live window and screen thumbnails to pick a screen-share source" width="820">
</p>

The same crate ships **`wlr-switcher`**, a live Alt-Tab / exposé window switcher.
The strip cycles with Tab; the exposé **reveals windows from other workspaces**:

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-switcher/altab.gif"
       alt="wlr-switcher: a macOS-style strip of live window previews, cycling with Tab" width="410">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-switcher/expose.gif"
       alt="wlr-switcher: the full-screen exposé grid revealing windows from every workspace" width="410">
</p>

When an application requests screen sharing (e.g. Firefox `getDisplayMedia`, a
video call), the wlroots portal asks an external *chooser* which source to share.
`wlr-chooser` replaces the text-only chooser with a grid of live previews — pick a
window or a monitor with a click.

<p align="center"><sub>📖 See every tool in action on the <a href="https://sjourdois.github.io/wlr-utils/">showcase</a>.</sub></p>

## Why

- **Real overlay**, like rofi: a `wlr-layer-shell` surface that grabs the keyboard,
  dims the desktop behind a centred card, and cancels on click-outside or Escape.
- **Captures any window** — including ones on other workspaces/outputs — via the
  compositor's native toplevel capture (`ext-image-copy-capture-v1`), not
  screen-region grabs. Off-screen windows are real previews, not icons.
- **Live thumbnails that actually move**: previews refresh in real time, and on
  the GPU path (default) the dma-buf is imported straight as a texture — no
  read-back, near-zero CPU. Falls back to CPU shm where the GPU path isn't usable.
- **Doubles as a window switcher** (`--switch`): pick a window to focus it.
- **Native Wayland** (no XWayland), built in Rust with [egui]; opens near-instantly.
- **Themeable** (8 ready palettes incl. Catppuccin), **localised** (13 languages,
  with CJK font fallback), and a configurable thumbnail grid.

## Requirements

- A wlroots-based compositor exposing `ext-image-copy-capture-v1` with the output **and**
  foreign-toplevel capture sources (the latter feeds the live window thumbnails),
  `ext-foreign-toplevel-list-v1` and `wlr-layer-shell` (Sway ≥ 1.12 / wlroots ≥ 0.20).
- `xdg-desktop-portal-wlr` ≥ 0.8 (for the screencast chooser use).
- For the **GPU path** (default): a working EGL/GLES driver and `libgbm`
  (ships with Mesa). It falls back to CPU automatically if unavailable.
- For the **`--switch`** window-switcher: `zwlr-foreign-toplevel-management-v1`.

## Install

> **Want the whole suite?** Install the bundle instead — `cargo install wlr-utils` gets
> every tool (`wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`, `wlr-draw`) in one
> go. The single-tool install below is the lighter, à-la-carte option.

```sh
cargo install wlr-chooser        # installs both wlr-chooser and wlr-switcher
```

Or build just these binaries from the [wlr-utils](../../README.md) workspace:

```sh
cargo build --release -p wlr-chooser
```

The `gpu` feature (on by default) enables zero-copy dma-buf capture and needs `libgbm-dev`
at build time (`libgbm` at runtime, from Mesa). `--no-default-features` builds a pure-CPU
binary with no gbm dependency. The whole suite also ships as a single `wlr-utils` `.deb`
on every [release](https://github.com/sjourdois/wlr-utils/releases/latest).

## Uninstall

The crate ships two binaries — `wlr-chooser` and `wlr-switcher`. Remove both the way you
installed them:

```sh
cargo uninstall wlr-chooser                       # crates.io install (~/.cargo/bin)
rm -f ~/.local/bin/wlr-chooser ~/.local/bin/wlr-switcher   # manual `install` from source
sudo apt remove wlr-chooser                        # the .deb package
```

## Set up the portal

Point the screencast chooser at the binary:

```ini
# ~/.config/xdg-desktop-portal-wlr/config
[screencast]
chooser_type=simple
chooser_cmd=wlr-chooser
```

Then restart the portal: `systemctl --user restart xdg-desktop-portal-wlr`.

Now any screen-share prompt opens `wlr-chooser` as a dimmed modal overlay on the
focused output. You can pass options in `chooser_cmd`, e.g.
`chooser_cmd=wlr-chooser --windows --grid 4x3`.

## Options

```
-w, --windows          Show only windows
-o, --outputs          Show only screens          (alias: --screens)
    --both             Show both (default)
    --include-system   Include windows with no app-id (system surfaces)
    --grid COLSxROWS   Fixed grid of that many thumbnails (e.g. 4x3)
-h, --help             Print help
-V, --version          Print version
```

In the overlay: type to filter, arrows to move, Enter/click to pick, Escape or
click-outside to cancel, and the tab bar switches All / Windows / Screens.

> **Looking for an Alt-Tab / window switcher?** That is now a separate binary,
> **`wlr-switcher`** (shipped alongside this one) — see [its section](#window-switcher--wlr-switcher) below.

## Window switcher — `wlr-switcher`

The same crate ships a second binary, **`wlr-switcher`**: a live Alt-Tab / exposé
that **focuses** the picked window (via `zwlr-foreign-toplevel-management-v1`)
instead of printing to stdout. It reuses this engine, so previews are **live** —
even for windows on other workspaces — which is what sets it apart from a plain
Cmd-Tab.

Three presentations via `--layout`:

- `strip` (default) — a macOS-style single row of tiles, the highlighted window's
  name above the row;
- `grid` — a full-screen, mission-control exposé;
- `card` — the centred rofi-like card.

Each tile shows a live preview with the app icon as a badge; tune it with
`--live none|current|all` (default `all`): `current` previews only the highlighted
window, `none` shows app icons only.

### True Alt-Tab (hold-to-switch)

Bind `wlr-switcher` to a **held** modifier and it behaves like a classic Alt-Tab:

```
bindsym Mod1+Tab exec wlr-switcher                 # hold Alt, Tab cycles, release switches
bindsym $mod+Tab exec wlr-switcher --layout grid   # full-screen exposé
```

- The overlay appears while the modifier (Alt **or** Super) is held.
- **`Tab`** moves to the next window, **`Shift+Tab`** to the previous one.
- **Releasing the modifier** confirms the highlighted window and switches to it.
- Mouse click and `Esc` (cancel) still work.

Hold-to-switch is **on by default for `strip`** and **off for `grid`/`card`**;
force it either way with `--hold` / `--no-hold`. With it off, the overlay stays
open after release — confirm with Enter or a click. Only one switcher opens at a
time (re-pressing the keybind is a no-op).

> **Tip:** set `WLR_CHOOSER_TIMING=1` to print cold-start timing milestones to
> stderr if you want to profile how fast the overlay appears.

> **Looking for a floating live mirror?** **`wlr-peek mirror`** keeps a picture-in-picture
> of a window (or a magnified region) always on top — see the
> [wlr-peek README](https://github.com/sjourdois/wlr-utils/tree/main/crates/wlr-peek).

## Output contract

`wlr-chooser` writes the selected source to stdout and exits `0`:

```text
Window: <foreign-toplevel-identifier>
Monitor: <output-name>
```

On cancel it writes nothing and exits non-zero.

## Theming

Colours and fonts come from `~/.config/wlr-chooser/theme.toml`
(`$XDG_CONFIG_HOME` is honoured) with sensible dark defaults. Colour keys are
`#rrggbb` / `#rrggbbaa`:

```toml
accent        = "#89b4fa"
screen-accent = "#74c7ec"   # outline for screens
window-accent = "#cba6f7"   # outline for windows
backdrop      = "#11111baa" # dimmed overlay

font      = "JetBrains Mono" # UI font family (via fontconfig)
# font-path = "/path/to/Font.ttf"
# cjk-font = "Noto Sans CJK JP"
font-size = 15.0
```

Screens are outlined in `screen-accent`, windows in `window-accent`, so the two
can't be confused. Ready-made themes live in [`docs/themes/`](docs/themes/):
Catppuccin (Mocha, Macchiato, Frappé, Latte), Nord, Gruvbox, Dracula, Tokyo Night.
Symlink one so it tracks updates:

```sh
mkdir -p ~/.config/wlr-chooser
ln -sf "$PWD/docs/themes/catppuccin-mocha.toml" ~/.config/wlr-chooser/theme.toml
```

## Localisation

The UI ships in 13 languages (English, French, German, Spanish, Italian,
Brazilian Portuguese, Dutch, Polish, Russian, Ukrainian, Japanese, Korean,
Simplified Chinese), translated with [Fluent](https://projectfluent.org/). It
**follows your desktop locale** (`LANG` / `LC_*`) and falls back to English when no
catalog matches. Override it any time with `LANGUAGE`:

```sh
LANGUAGE=ja wlr-chooser
```

Rendering CJK text needs a CJK font installed (e.g. Noto Sans CJK); one is
auto-detected. New locales are welcome — copy
`crates/wlr-capture/i18n/en/wlr_capture.ftl`.

## Contributing

Bug reports, translations and patches welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md). Please keep `cargo fmt`, `cargo clippy` and
`cargo test` clean.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.

[egui]: https://github.com/emilk/egui
