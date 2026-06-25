# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- **`wlr-draw`** — the tray menu gained a **Start on login** toggle that writes / removes
  an XDG autostart entry (`~/.config/autostart/wlr-draw.desktop`). The daemon also
  auto-registers itself on its first ever run (tracked by a sentinel under
  `$XDG_STATE_HOME`), so it starts with the session out of the box; unchecking it from the
  tray is then permanent — a later manual launch won't bring it back.

### Fixed

- **`wlr-draw`** — the daemon never called `i18n::init()` (unlike every other binary), so
  the tray menu and on-screen hints stayed English regardless of `$LANG`/`$LANGUAGE`. It
  now negotiates the desktop locale at startup. The autostart entry also carries its state
  as a `☑` / `☐` glyph in the label rather than a dbusmenu `toggle-type=checkmark`, which
  several SNI hosts (e.g. waybar) don't render reliably.

## 1.3.0 — 2026-06-25

### Added

- **`wlr-shot`** — a new screen-capture tool. `screenshot` captures an output (`-o`), a
  slurp-style logical region (`-g`, stitched across outputs), a window (`-w` /
  `--pick-window`), the whole layout (`--all`), the active window (`-a`) or focused
  output (`--current-output`) to PNG/JPEG/PPM (file, stdout, or `-c` clipboard), with an
  interactive **frozen region selector** (`-s`).
- **`wlr-shot record`** — record to **H.264** (`.mp4`/`.mkv`), animated **GIF/WebP**
  (downscaled; best on a region), with **system audio** as an AAC track (native
  PipeWire; `--no-audio`, `--audio-source`; an optional Pulse/ALSA fallback lives behind
  the off-by-default `audio-fallback` feature). Pluggable encoder
  (`--encoder auto|nvenc|vaapi|software`), constant-`--fps` capture, `--timelapse`,
  `--duration`/Ctrl-C.
- **`wlr-switcher`** — a live Alt-Tab / exposé that **focuses** the picked window; held
  modifier to switch, `--layout strip|grid|card`, live previews (`--live`).
- **`wlr-draw`** — a transparent on-screen annotation overlay (gromit-mpx-style): pen,
  rectangle, arrow, text, mask, eraser, **move** (right-drag or the move tool + arrow
  nudge), and dwell-to-snap circles/lines. Runs as a daemon driven by a key-bound
  control socket, with a colour palette, tray icon and systemd unit. Plus presenter
  tools: **spotlight** (hold Shift to dim the screen around a shape or the cursor),
  **freeze-frame** (`Space`), and **save** the annotated screen to PNG (`w`).
- **`wlr-peek`** — `watch` (change/idle monitor), `grep` (OCR then locate text),
  `region` (a native slurp replacement); and `mirror` now takes the suite's common
  source flags plus `--follow window`.
- **Compositor compatibility** — `wlr-peek doctor` reports the protocols the running
  compositor advertises; focus backends for Hyprland and niri join Sway; see
  [`COMPATIBILITY.md`](COMPATIBILITY.md).
- **`wlr-capture`** engine — a shared capture-session driver (`stream`), a frame-diff
  metric (`diff`), one-shot capture, a `Region` type with cropping/multi-output
  compositing, GPU dma-buf readback (`gl`), the `FrameSink` output seam, a wlroots
  clipboard (`data-control`), and a native-PipeWire `audio` module. Split into a lean
  always-on core plus optional features (`toolkit`, `video`, `audio`, `overlay`, …).
- **i18n** — the shared Fluent catalog is now complete in 13 languages (de, en, es, fr,
  it, ja, ko, nl, pl, pt-BR, ru, uk, zh-CN). The command line stays English.

### Changed

- **`wlr-chooser`** is now strictly the xdg-desktop-portal picker (prints the chosen
  source); the switcher modes moved to `wlr-switcher`. Both ship from one package and
  share the engine.
- The chooser/switcher overlay starts capturing before building itself, so thumbnails
  stream in sooner (`WLR_CHOOSER_TIMING=1` prints cold-start timing).
- Upgraded to the latest major dependencies (egui 0.34, glow 0.17, pipewire 0.10,
  ksni 0.3); the **minimum supported Rust version is now 1.92**.

## 1.2.0

### Added

- **`wlr-pip`**: a new companion binary — a floating, always-on-top live mirror
  (picture-in-picture) of a single window, sharing the same zero-copy GPU capture
  engine. Pick a window via `wlr-chooser` (run `wlr-pip` with no argument) or pass
  its identifier (`wlr-pip <id>`). It is an `xdg-toplevel` (pair with Sway
  `floating enable, sticky enable` for always-on-top across workspaces): drag to
  move, corner grip to resize (source aspect ratio kept), hover for collapse/close,
  `Esc` to quit. Collapsed to an icon badge, it pops back open when its window
  changes. One mirror per window (single-instance lock per identifier). Keyboard
  shortcuts: `Space` freeze/unfreeze, `c` collapse, `+`/`-` or wheel for opacity,
  `r` re-pick another window, `q`/`Esc` close.

### Changed

- The project is now a Cargo **workspace**: a shared `wlr-capture` library (the
  wlroots capture engine + the egui/EGL rendering & dma-buf-import toolkit, both
  extracted from the previous single crate) plus the `wlr-chooser` and `wlr-pip`
  binaries. No behaviour change for `wlr-chooser`.

## 1.1.0

### Added

- **Live thumbnails**: previews now refresh continuously (damage-driven), so the
  grid shows windows updating in real time, including on other workspaces.
- **GPU zero-copy capture** behind the `gpu` Cargo feature (on by default):
  dma-bufs are allocated via gbm and imported as GL textures (EGLImage), with no
  CPU read-back. Falls back to the CPU shm path automatically when unavailable.
  Build without it (no gbm/`libgbm` dependency) via `--no-default-features`.
- **`--switch`** window switcher: a live alt-tab / exposé that **focuses** the
  picked window (via `zwlr-foreign-toplevel-management-v1`) instead of printing.
  Two presentations via `--layout`: `full` (full-screen mission-control grid that
  dims the desktop, with an intro animation — default) or `compact` (the centred
  card). Identical windows are disambiguated by creation order so the right one
  is focused. Only one switcher opens at a time (re-pressing the keybind is a
  no-op, via a single-instance lock).

## 1.0.0

Initial release.
