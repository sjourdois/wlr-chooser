# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

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
