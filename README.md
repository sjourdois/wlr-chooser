# wlr-chooser

[![CI](https://github.com/sjourdois/wlr-chooser/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-chooser/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/wlr-chooser.svg)](https://crates.io/crates/wlr-chooser)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A graphical window & screen picker for **wlroots** screencast portals
(`xdg-desktop-portal-wlr`) — a rofi-like overlay with **live thumbnails**.

When an application requests screen sharing (e.g. Firefox `getDisplayMedia`, a
video call), the wlroots portal asks an external *chooser* which source to share.
`wlr-chooser` replaces the text-only chooser with a grid of live previews — pick a
window or a monitor with a click.

![wlr-chooser overlay](docs/screenshots/overlay.png)

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

- A wlroots-based compositor exposing `ext-image-copy-capture-v1`,
  `ext-image-capture-source-v1`, `ext-foreign-toplevel-list-v1` and
  `wlr-layer-shell` (Sway ≥ 1.12 / wlroots ≥ 0.20).
- `xdg-desktop-portal-wlr` ≥ 0.8 (for the screencast chooser use).
- For the **GPU path** (default): a working EGL/GLES driver and `libgbm`
  (ships with Mesa). It falls back to CPU automatically if unavailable.
- For the **`--switch`** window-switcher: `zwlr-foreign-toplevel-management-v1`.

## Install

### From a package

- **crates.io:** `cargo install wlr-chooser`.
- **Debian/Ubuntu:** download the `.deb` from the
  [latest release](https://github.com/sjourdois/wlr-chooser/releases/latest) and
  `sudo apt install ./wlr-chooser_*.deb`.
- **Arch (AUR):** _coming soon._

### Prebuilt binary

Download the binary for your platform from the
[releases page](https://github.com/sjourdois/wlr-chooser/releases/latest), or run
the installer script:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/sjourdois/wlr-chooser/releases/latest/download/wlr-chooser-installer.sh | sh
```

### From source

```sh
cargo build --release          # GPU path on by default; needs libgbm-dev to build
install -Dm755 target/release/wlr-chooser ~/.local/bin/wlr-chooser
```

The `gpu` feature (on by default) enables zero-copy dma-buf capture and needs
`libgbm-dev` at build time (`libgbm` at runtime, from Mesa). For a pure-CPU
build with no gbm dependency, use `cargo build --release --no-default-features`.

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
    --switch           Window switcher: focus the picked window (implies --windows)
    --layout <LAYOUT>  Switcher presentation: full (full-screen, default) | compact
    --include-system   Include windows with no app-id (system surfaces)
    --grid COLSxROWS   Fixed grid of that many thumbnails (e.g. 4x3)
-h, --help             Print help
-V, --version          Print version
```

In the overlay: type to filter, arrows to move, Enter/click to pick, Escape or
click-outside to cancel, and the tab bar switches All / Windows / Screens.

## Window switcher

`wlr-chooser --switch` turns the picker into a live alt-tab / exposé: a grid of
**live** window thumbnails (updating in real time, even for windows on other
workspaces); picking one **focuses** it instead of printing to stdout.

Two presentations via `--layout`:

- `full` (default) — a full-screen, mission-control grid that dims the desktop.
- `compact` — the centred rofi-like card.

Bind it in your compositor, e.g. Sway:

```
bindsym $mod+Tab  exec wlr-chooser --switch                  # full-screen
bindsym Alt+Tab   exec wlr-chooser --switch --layout compact # compact card
```

Only one switcher opens at a time (re-pressing the keybind is a no-op).

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
auto-detected. New locales are welcome — copy `i18n/en/wlr_chooser.ftl`.

## Contributing

Bug reports, translations and patches welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md). Please keep `cargo fmt`, `cargo clippy` and
`cargo test` clean.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option.

[egui]: https://github.com/emilk/egui
