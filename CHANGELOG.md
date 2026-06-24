# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- **`wlr-shot record` audio**: a video recording now captures **system audio** (the
  default sink's monitor) as an AAC track by default, via **native PipeWire**. Capture
  is decoupled from the FFmpeg muxer, so an optional **Pulse/ALSA fallback** (the
  `audio-fallback` feature, off by default — PipeWire is universal — via FFmpeg's
  libavdevice, no extra system dep) slots in after it for PipeWire-less hosts.
  `--no-audio` records silently; `--audio-source NODE` picks a specific node (e.g. a
  microphone). Needs the new `audio` build feature (on by default; links libpipewire).
  GIF/WebP carry no audio.
- **`wlr-shot record` animated GIF/WebP**: pick the format by extension — `out.gif` or
  `out.webp` produce an animated image instead of H.264. Frames are downscaled (GIF to
  800 px, WebP to 1280 px on the long side) since per-frame GIF quantization on a full
  output is slow, so this is best on a region. WebP vendors libwebp (no system dependency).
- **`wlr-draw` save**: press `w` (or `wlr-draw save [path]`) to write the annotated
  screen — the output under the cursor — to a PNG in your Pictures directory; works on a
  frozen frame too.
- **`wlr-draw`**: a new tool — draw and annotate **live on screen**, a Wayland-native
  take on gromit-mpx. A transparent always-on-top `wlr-layer-shell` overlay per output
  you scribble on with **pen**, **rectangle**, **arrow**, **text** and a **mask** (solid
  box to redact areas) tool, plus an **eraser** (line and ellipse come from the pen's
  dwell-to-snap rather than dedicated tools); toggle draw mode on to grab input, off to click through to your
  apps while the annotations stay on screen. Because a layer-shell client can't grab a
  global hotkey, it runs as a daemon driven over a per-user **control socket** — bind
  `wlr-draw toggle` / `clear` / `undo` / `tool …` / `color …` to compositor keys. Also
  **dwell-to-snap**: hold the pen still mid-stroke to snap a rough freehand loop to a
  clean circle/ellipse (or a straight line) and resize it live before releasing. While
  drawing, single-key shortcuts switch tool/width, `h` shows an on-screen key legend,
  `c` opens a colour-picker palette, **Ctrl** constrains a shape (square/circle/axis)
  and **Caps Lock** toggles a pointer pass-through to the apps below without leaving
  draw mode. Ships a **StatusNotifierItem tray icon** (the `tray` feature, on by
  default) whose glyph reflects the **current tool** (in the stroke colour while
  drawing), with a click-to-toggle menu including a **Shortcuts** submenu, and a systemd
  `--user` unit (`contrib/wlr-draw.service`, bound to `graphical-session.target`). The
  status chip **pulses** when draw mode is entered on an empty canvas or when the pointer
  is jabbed repeatedly at one spot (a "you're in draw mode" reminder).
- **`wlr-peek region`**: select a region (or a point, `-p`) with the mouse and print
  its geometry as `X,Y WxH` — a native **slurp replacement** (reusing the frozen
  overlay), with a `--format` string (`%x %y %w %h`). Exits 1 if cancelled.
- **`wlr-peek mirror`** now takes the **same source flags as the rest of the suite**:
  besides a window `id` and `-g "X,Y WxH"`, it accepts `-s` (interactive region select,
  no slurp needed), `-o NAME` / `--current-output` (mirror a whole output), `-a`
  (active window) and `-w`/`--pick-window`. For a region, **`--follow window`** captures
  the window under it (cropped to the region) so the loupe follows that window across
  moves/workspaces, instead of the output (`--follow output`, the default).
- **Compositor compatibility**: a [`COMPATIBILITY.md`](COMPATIBILITY.md) matrix
  (which protocols each compositor needs and supports), a new **`wlr-peek doctor`**
  that reports which capture protocols the running compositor advertises (and whether
  capture / focus sources will work), and focus backends for **Hyprland** (`hyprctl`)
  and **niri** (`niri msg`) alongside Sway — so `-a` / `--current-output` work there
  too (niri exposes no per-window geometry, so its `-a` is unavailable; use `-g` /
  `--current-output`).
- **`wlr-peek watch`**: a change monitor. Streams a source (region `-g`/`-s`, output
  `-o`/`--current-output`, or window `-w`/`--pick-window`/`-a`) and fires `--on
  change` or `--on idle` (stable for `--for`). `--threshold PCT` ignores changes
  below that fraction of the watched pixels (skips a blinking cursor/clock); prints
  one line and exits 0 on the first trigger (composes with `&&`), or `--repeat` +
  `--exec CMD` to act every time; `--timeout` gives up (exit 2).
- **`wlr-peek grep`**: a visual grep. OCRs a source and prints where matching text is
  in global logical coordinates (slurp-compatible `X,Y WxH`), `-i` for
  case-insensitive; exits 1 on no match like `grep` (needs the `ocr` feature).
- **`wlr-capture`** engine: a shared live-capture session driver (`stream` — arm /
  poll / reopen / give-up, used by the mirror, recorder and monitor) and a
  frame-difference metric (`diff`) for change detection. The mirror and `wlr-shot
  record` now run on the shared driver instead of bespoke loops.
- **`wlr-shot`**: a new screen-capture binary (built on the shared `wlr-capture`
  engine). `wlr-shot screenshot` captures a full output (`-o`), a logical region
  (`-g "X,Y WxH"`, slurp-compatible, stitched across the outputs it covers), or a
  window (`-w ID` / `--pick-window` via `wlr-chooser`), the whole layout (`--all`),
  the active window (`-a`) or the focused output (`--current-output`), to
  PNG/JPEG/PPM (file or stdout); `--list-outputs` prints names and logical
  geometry. Single-output captures keep native resolution (incl. fractional
  scaling); multi-output regions composite at logical resolution. Focus-aware modes
  use compositor IPC via a small extensible backend (Sway today; Hyprland/niri
  next). `-s` opens an **interactive frozen region selector** — a layer-shell
  overlay per output showing the frozen screen; drag to select (spanning outputs),
  `Esc` cancels, `Enter` confirms — then crops from the same frozen pixels.
  `-c`/`--clipboard` copies the shot to the Wayland clipboard (wlroots
  `data-control`) instead of writing a file: a tiny daemon detaches to serve the
  selection until another client replaces it (`--clipboard-foreground` keeps it in
  the foreground).
- **`wlr-shot record`**: record a source to an H.264 video file (container from the
  extension, e.g. `.mp4`). Same sources as `screenshot` — output (`-o`/sole),
  `--current-output`, window (`-w`/`--pick-window`, follows it across workspaces and
  while occluded), `-a`, and a single-output region (`-g`/`-s`). Pluggable encoder
  (`--encoder auto|nvenc|vaapi|software`): `auto` prefers hardware (NVENC, then
  VAAPI on a `--device` render node) and falls back to software `libx264`. Capture is
  damage-driven, so a normal recording emits a constant frame rate at `--fps` (default
  30), repeating the last frame through static stretches; `--timelapse INTERVAL`
  instead samples one frame per interval and plays them at `--fps`, so the footage is
  sped up. Stops on `--duration SECS`, Ctrl-C (the file is finalised cleanly), or the
  window closing. Built on a new `wlr_capture::video` `FrameSink` (feature `video`,
  on by default; `--no-default-features` builds screenshots-only with no FFmpeg
  dependency).
- **`wlr-capture`** engine additions (foundations for the capture suite): accurate
  output geometry via `xdg-output` with a `wl_output` fallback (multi-monitor
  positions and fractional-scale logical sizes), one-shot capture
  (`capture_output_once` / `capture_toplevel_once`), a `Region` type with
  `CapturedImage::pixel` / `crop` / `blit_into` for cropping and multi-output
  compositing, and a `clipboard` module that puts a captured blob on the wlroots
  clipboard via `zwlr_data_control_v1`. New `gl::GpuReadback` reads a capture
  dma-buf back to CPU RGBA8 through a headless offscreen EGL context (1×1 pbuffer +
  `glReadPixels`), so the GPU path can feed CPU encoders. New `sink::FrameSink` is
  the common output seam for screenshot/record/timelapse, with `sink::pump` routing
  shm frames straight through and reading dma-buf frames back via `GpuReadback`
  (built lazily, so pure-shm streams never spin up EGL). The crate now splits into
  an always-on core (`wl`, `clipboard`, `gl`, `sink`) and a `toolkit` feature (on by
  default) gating the egui/EGL overlay UI (`render` + `theme`/`i18n`/`icons`) — a
  headless build (`--no-default-features`) drops egui, resvg, fontdb and the i18n
  stack for a ~6× smaller dependency tree. `wlr-shot` gains an off-by-default `gpu`
  feature that builds the dma-buf capture path and reads frames back before
  encoding (the shipped binary stays shm-only, no `libgbm`).

- **`wlr-switcher`**: a new binary — a live window switcher / Alt-Tab / exposé that
  **focuses** the picked window (via `zwlr-foreign-toplevel-management-v1`). Bind it
  to a held modifier for a true Alt-Tab (`bindsym Mod1+Tab exec wlr-switcher`): the
  overlay appears while the modifier (Alt **or** Super) is held, `Tab`/`Shift+Tab`
  cycle, and **releasing the modifier** confirms and switches. Three presentations
  via `--layout strip|grid|card`:
  - `strip` (default) — a macOS-style single row of tiles, highlighted window's
    name above; each tile shows a **live preview** with the app icon as a badge
    (live capture being the differentiator);
  - `grid` — the full-screen mission-control exposé;
  - `card` — the centred rofi-like card.
  Live previews are tunable with `--live none|current|all` (default `all`).
  Hold-to-switch is on by default for `strip`, off for `grid`/`card`; force it with
  `--hold` / `--no-hold`. Mouse click and `Esc` still work; only one switcher opens
  at a time. It uses the `keyboard-shortcuts-inhibit` protocol so the compositor
  forwards the full chord (e.g. `Mod1+Tab`) to the overlay instead of re-running its
  binding — this is what lets `Tab` cycle forward under Sway and other wlroots
  compositors. App icons are loaded at higher resolution to stay crisp, and the
  exposé intro animation is skipped while hold-to-switch is armed for an instant
  first frame.

### Changed

- **`wlr-chooser`** is now strictly the xdg-desktop-portal-wlr picker (prints the
  chosen source to stdout). The window-switcher modes (`--switch` / `--alt-tab`)
  moved to the new `wlr-switcher` binary. Both binaries ship from the same package
  and share the capture engine.
- The overlay now starts capturing before building itself, so thumbnails stream in
  sooner. Set `WLR_CHOOSER_TIMING=1` to print cold-start timing milestones to
  stderr.

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
