# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## 1.4.0 — 2026-07-01

### Added

- **`wlr-draw` configurable keys** — bindings now live in `~/.config/wlr-draw/keys.toml`;
  the click-through key (Caps Lock by default) and every tool key can be rebound, so
  keyboards without a Caps Lock (e.g. HHKB) are no longer stuck (#2).
- **`wlr-draw save [path]`** — save the annotated screen to a chosen path from the command
  line (#3).
- **Screen-only compositors** — tools degrade gracefully on wlroots 0.19 / Sway 1.11:
  screen capture (screenshots, recording, loupe, `wlr-draw` freeze/save) works; window-only
  features stay off with a clear notice, and `wlr-switcher` reports the missing window
  capture and exits instead of showing an empty overlay (#1).
- **`doctor` on every tool** — the compositor report is now a `doctor` subcommand on
  `wlr-peek`/`wlr-shot`/`wlr-draw` and a `--doctor` flag on `wlr-chooser`/`wlr-switcher`
  (previously only `wlr-peek doctor`), so any single-tool install can produce it. It also
  reports the run environment — tool version, OS, compositor + version, install hint — so
  it doubles as the environment block a bug report needs.

### Fixed

- **`wlr-draw save`** — paths containing spaces are no longer truncated (#3).
- **`wlr-draw`** — a very short arrow no longer panics while sizing its head.
- **Debian / Ubuntu `.deb`** — now built per distro (Debian 12–sid, Ubuntu 22.04–26.04) so
  it links against that distro's FFmpeg / Leptonica; fixes `liblept.so.5` / `libavutil.so.58`
  load failures on Debian 13 (#1).
- **`wlr-peek doctor`** — verdicts reflect the two capture floors (screen 0.19/1.11,
  window 0.20/1.12).

### Security

- Control socket and single-instance locks stay in a private directory even when
  `$XDG_RUNTIME_DIR` is unset, instead of a predictable name in world-readable `/tmp`.

### Changed

- **`wlr-capture`** — the engine returns a typed `CaptureError` instead of `anyhow`
  (breaking for library users); each tool now owns its own translation catalog.
- **Docs** — added a compositor compatibility matrix and per-distro `.deb` install notes.
- Dependency refresh (egui 0.35, xkbcommon 0.9).

## 1.3.2 — 2026-06-26

### Added

- **`wlr-utils`** — a new bundle crate that installs the whole suite at once:
  `cargo install wlr-utils` provides `wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`
  and `wlr-draw`. The prebuilt release now ships a **single archive** with every binary
  plus a one-line `wlr-utils-installer.sh`, and a **single `wlr-utils` `.deb`** (it
  `Replaces`/`Conflicts` the old per-tool packages), instead of one download per tool. Each
  tool stays its own crate for lighter, à-la-carte installs.

### Removed

- **`wlr-pip`** — the deprecated stub (superseded by `wlr-peek mirror`) is no longer built
  or published.

### Fixed

- **`wlr-peek doctor`** — on a screen-capture-only compositor (Sway 1.11 / wlroots 0.19)
  the report no longer claims a bare "screen capture: supported"; it now notes the tools
  also open the window source at start-up, so the effective floor stays Sway 1.12 /
  wlroots 0.20.
- **Docs** — aligned the **Install / Uninstall / Requirements** sections across every
  README (per-tool system dependencies, the unified bundle, and removing `wlr-draw`'s
  autostart entry).

## 1.3.1 — 2026-06-26

### Added

- **`wlr-draw`** — the tray menu gained a **Start on login** toggle that writes / removes
  an XDG autostart entry (`~/.config/autostart/wlr-draw.desktop`). The daemon also
  auto-registers itself on its first ever run (tracked by a sentinel under
  `$XDG_STATE_HOME`), so it starts with the session out of the box; unchecking it from the
  tray is then permanent — a later manual launch won't bring it back.

### Fixed

- **Compositor requirements** were documented wrong (`wlroots 0.18` / `Sway 1.10`). The
  base protocol plus output capture landed in **wlroots 0.19 / Sway 1.11**, and the
  foreign-toplevel source that *window* capture needs only in **wlroots 0.20 / Sway 1.12**
  — the suite's real floor. `COMPATIBILITY.md` is corrected, the
  `ext_foreign_toplevel_image_capture_source_manager_v1 missing` error now names the
  required version and points at `wlr-peek doctor`, and `doctor` reports **screen** and
  **window** capture as separate verdicts.
- **`wlr-draw`** — the daemon never called `i18n::init()` (unlike every other binary), so
  the tray menu and on-screen hints stayed English regardless of `$LANG`/`$LANGUAGE`. It
  now negotiates the desktop locale at startup. The autostart entry also carries its state
  as a `☑` / `☐` glyph in the label rather than a dbusmenu `toggle-type=checkmark`, which
  several SNI hosts (e.g. waybar) don't render reliably.

### Changed

- Replaced the unmaintained `khronos-egl` (last released ~5 years ago) with the
  API-compatible, actively maintained `edgefirst-egl` fork, which lets the EGL bindings
  track **`libloading` 0.9**. Refreshed the rest of the dependency tree (`image` 0.25.10,
  `resvg` 0.47, …) and dropped the pinned `rust-version` from the workspace manifest.
- **`wlr-capture`** — docs.rs now documents the full public API. It built with default
  features only, so the feature-gated modules (`capture`/`focus`/`overlay`/`mirror`/
  `video`) were missing; a `[package.metadata.docs.rs]` block enables them (with
  `doc(cfg)` feature badges). Documented every remaining public item (100 % coverage,
  enforced by `#![warn(missing_docs)]`). `audio` stays off on docs.rs — its `pipewire`
  dep fails to build there.

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
