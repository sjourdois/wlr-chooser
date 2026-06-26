# Compositor compatibility

`wlr-utils` is built on a handful of Wayland protocols. A compositor that
advertises them works; one that doesn't, doesn't (there is no portal fallback).
The quickest way to check your own compositor is:

```console
$ wlr-peek doctor
```

It prints which of the protocols below the running compositor advertises, and
whether screen capture and focus-aware sources will work.

## Protocols used

| Protocol | Used for | Needed by |
| --- | --- | --- |
| `ext-image-copy-capture-v1` + `ext-image-capture-source-v1` + the output / foreign-toplevel source managers | the **capture engine** (frames of an output or a window) | everything |
| `ext-foreign-toplevel-list-v1` | enumerating windows | `wlr-chooser`, `-w`, `wlr-peek mirror`, window record/watch |
| `wlr-layer-shell` (`zwlr_layer_shell_v1`) | full-screen overlays | the region selector (`-s`), `wlr-peek loupe`/`color`, `wlr-switcher`, `wlr-chooser` |
| `wlr-data-control` (`zwlr_data_control_manager_v1`) | clipboard copy | `-c`/`--clipboard` |
| `keyboard-shortcuts-inhibit` (`zwp_keyboard_shortcuts_inhibit_manager_v1`) | grabbing keys under a layer-shell grab | `wlr-switcher` (so `Alt+Tab` reaches it) |
| `linux-dmabuf` (`zwp_linux_dmabuf_v1`) | zero-copy GPU capture (optional; CPU `wl_shm` is the fallback) | the optional `gpu` build |
| `xdg-output` (`zxdg_output_manager_v1`) | accurate logical geometry (fractional scale, positions) | recommended; falls back to `wl_output` |
| compositor IPC | "the active window" / "the current output" (`-a`, `--current-output`) | a per-compositor focus backend |

`ext-image-capture-source-v1` is the linchpin, and it landed in two steps: the base
protocol plus the **output** source arrived in **wlroots 0.19** (Sway ≥ 1.11), but the
**foreign-toplevel** source (`ext_foreign_toplevel_image_capture_source_manager_v1`) —
which window capture depends on — only arrived in **wlroots 0.20** (Sway ≥ 1.12). Since
every tool here opens the window source at start-up (even for a screen capture), the
effective floor is **wlroots 0.20 / Sway ≥ 1.12**. A Sway 1.11 compositor advertises
screen capture, but the tools still abort with
`ext_foreign_toplevel_image_capture_source_manager_v1 missing`. None of this is
implemented by GNOME's Mutter or KDE's KWin, which only offer screen capture through the
desktop portal / PipeWire — out of scope here.

## Compositors

Any **wlroots-based** compositor that advertises the protocols above should work —
**Sway**, **Hyprland**, **niri**, **river**, **Wayfire**, **cosmic-comp** and the like.
Run `wlr-peek doctor` to check yours.

Only **Sway** (≥ 1.12 / wlroots ≥ 0.20) is **runtime-verified** — it's the development
compositor. The others are *expected* to work from their protocol support, but haven't
been exercised end-to-end yet.

**Mutter** (GNOME) and **KWin** (KDE Plasma) are **out of scope**: they don't implement
`ext-image-copy-capture-v1` (or `wlr-layer-shell`), offering screen capture only through
the desktop portal / PipeWire, which this suite deliberately doesn't use.

Two things vary by compositor:

- **Focus-aware sources** — `-a` (active window) and `--current-output` need a
  per-compositor IPC backend (see below). Backends ship for **Sway** (`swaymsg`),
  **Hyprland** (`hyprctl`) and **niri** (`niri msg`). Without a backend, every *other*
  source still works: `-s` interactive select, `-g` geometry, `-o NAME`, `-w ID`,
  `--pick-window`. (niri exposes no per-window global rectangle, so its `-a` is
  unavailable — use `-g` / `--current-output`.)
- **Zero-copy GPU capture** (`linux-dmabuf`) is optional; the CPU `wl_shm` path is the
  universal fallback.

> [!NOTE]
> **Help wanted.** If you run wlr-utils on Hyprland, niri, river, Wayfire, cosmic-comp or
> any other wlroots compositor, please report how it goes — run `wlr-peek doctor` and
> open an issue with the output. Validation reports (and focus backends for more
> compositors) are very welcome.

## Adding a compositor

Focus backends live in [`crates/wlr-capture/src/focus.rs`](crates/wlr-capture/src/focus.rs):
implement `FocusBackend` (a `focused_output()` and an `active_window_rect()`) over
your compositor's IPC and add a detection branch in `detect()`. The Sway, Hyprland
and niri backends are short worked examples.
