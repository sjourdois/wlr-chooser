# wlr-pip

[![CI](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml/badge.svg)](https://github.com/sjourdois/wlr-utils/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/wlr-pip.svg)](https://crates.io/crates/wlr-pip)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A floating, always-on-top **live picture-in-picture mirror** of a single
**wlroots** window. Keep an eye on a build log, a video, or a dashboard while it
sits on another workspace or behind other windows — the mirror updates in real
time, using the same zero-copy GPU capture as [`wlr-chooser`].

Part of the [wlr-utils](https://github.com/sjourdois/wlr-utils) workspace.

## Why

- **Mirrors any window**, even occluded or on another workspace, via the
  compositor's native toplevel capture (`ext-image-copy-capture-v1`) — not a
  screen-region grab.
- **Live and cheap**: on the GPU path (default) the dma-buf is imported straight
  as a texture (no read-back), and repaints are driven by capture damage, so a
  static window costs almost nothing.
- **Stays out of the way**: a normal `xdg-toplevel` you can move, resize, make
  translucent, or shrink to an icon badge that pops back open when its window
  changes.

## Requirements

- A wlroots compositor exposing `ext-image-copy-capture-v1`,
  `ext-image-capture-source-v1` and `ext-foreign-toplevel-list-v1`
  (Sway ≥ 1.12 / wlroots ≥ 0.20).
- For the **GPU path** (default): a working EGL/GLES driver and `libgbm` (ships
  with Mesa). Falls back to CPU shm automatically.
- `wlr-chooser` on `PATH` if you launch `wlr-pip` with no argument (it shells out
  to it to pick a window).

## Install

```sh
# crates.io
cargo install wlr-pip
# from the workspace
cargo build --release -p wlr-pip && install -Dm755 target/release/wlr-pip ~/.local/bin/wlr-pip
```

Prebuilt binaries and a `.deb` are attached to each
[release](https://github.com/sjourdois/wlr-utils/releases/latest).

## Usage

```sh
wlr-pip                 # pick a window via wlr-chooser, then mirror it
wlr-pip <identifier>    # mirror directly (identifier as printed by wlr-chooser)
```

`wlr-pip` is a normal `xdg-toplevel`, so the compositor manages stacking. Add a
couple of Sway rules to make it behave like a PiP (floating, on every workspace,
above others), and optionally a keybind:

```
for_window [app_id="wlr-pip"] floating enable, sticky enable
bindsym $mod+p exec wlr-pip
```

Mirror several windows at once by launching `wlr-pip` more than once; only one
mirror per window is allowed (a second launch for the same window is a no-op).

## Controls

In the tile: **drag** the body to move it, drag the **bottom-right grip** to
resize (the source aspect ratio is kept), and hover to reveal the **collapse**
(to an icon badge) and **close** buttons. Collapsed, the tile pops back open the
moment its window changes — a lightweight "tell me when this changes" watcher.

Keyboard shortcuts (the tile must have focus — click it first):

| Key | Action |
| --- | --- |
| `Esc` / `q` | Close |
| `Space` | Freeze / unfreeze the live feed |
| `c` | Collapse to / from the icon badge |
| `+` / `-` (or scroll wheel) | More / less opaque |
| `r` | Re-pick another window (opens the chooser) |

## License

Licensed under either of [Apache-2.0](../../LICENSE-APACHE) or
[MIT](../../LICENSE-MIT) at your option.

[`wlr-chooser`]: https://crates.io/crates/wlr-chooser
