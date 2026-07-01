# Compositor compatibility

`wlr-utils` is built on a handful of Wayland protocols. A compositor that
advertises them works; one that doesn't, doesn't (there is no portal fallback).
The quickest way to check your own compositor is the `doctor` command, which
**every tool** exposes — a `doctor` subcommand on `wlr-peek` / `wlr-shot` / `wlr-draw`,
and a `--doctor` flag on `wlr-chooser` / `wlr-switcher`:

```console
$ wlr-peek doctor
```

It prints your tool version, OS, compositor + version, which of the protocols below
the running compositor advertises, and whether screen capture and focus-aware sources
will work — so it doubles as the environment block a bug report needs. Any tool prints
the same report, so a single-tool install can produce it too.

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
protocol plus the **output** source arrived in **wlroots 0.19** (Sway ≥ 1.11), while the
**foreign-toplevel** source (`ext_foreign_toplevel_image_capture_source_manager_v1`) —
which window capture depends on — only arrived in **wlroots 0.20** (Sway ≥ 1.12).

So there are **two floors**:

- **Screen capture** — `ext-image-copy-capture-v1` + the **output** source: **wlroots ≥ 0.19
  / Sway ≥ 1.11**. Screenshots, recording, the loupe/colour picker, region select, and
  wlr-draw's freeze & save work here.
- **Window capture** — additionally the **foreign-toplevel** source + list: **wlroots ≥ 0.20
  / Sway ≥ 1.12**. The Alt-Tab switcher, `-w`/`--pick-window`, and per-window mirror/record
  need this.

The tools **degrade gracefully**: on a Sway 1.11 / wlroots 0.19 compositor the screen
features all work, while window-only paths fail with a clear message (`wlr-switcher` says so
and exits instead of showing an empty overlay; wlr-draw hides freeze/save when even screen
capture is missing). Run `wlr-peek doctor` to see which of the two your compositor offers.

## Compositors

The matrix below tracks the two capture floors plus `wlr-layer-shell` (needed by every
overlay: the switcher, region selector, loupe/colour picker and wlr-draw) and the focus IPC
backend (for `-a` / `--current-output`). Run `wlr-peek doctor` to check your own.

| Compositor | Screen capture | Window capture | Overlays (layer-shell) | Focus IPC |
| --- | --- | --- | --- | --- |
| **Sway** | ✅ ≥ 1.11 (wlroots 0.19) | ✅ ≥ 1.12 (wlroots 0.20) | ✅ | ✅ `swaymsg` |
| **Hyprland** | ✅ ≥ v0.54 | ✅ ≥ v0.54 | ✅ | ✅ `hyprctl` |
| **labwc** | ✅ ≥ 0.9 (wlroots 0.19) | 🟡 ≥ 0.20 (partial) | ✅ | ❌ |
| **cosmic-comp** | ✅ | ✅ | ✅ | ❌ |
| **Wayfire** | ✅ ≥ 0.10 (wlroots 0.19) | ❌ (until its 0.20 branch ships) | ✅ | ❌ |
| **river** | ✅ ≥ 0.3 (wlroots 0.19) | ❌ | ✅ | ❌ |
| **niri** | ❌ (`wlr-screencopy` only) | ❌ | ✅ | 🟡 `niri msg` (`-a` n/a) |
| **dwl** | ❌ (`wlr-screencopy` only) | ❌ | ✅ | ❌ |
| **Mutter** (GNOME) | ✅ (≥ 49) | ❌ | ❌ | ❌ |
| **KWin** (KDE) | ✅ (≥ 6.6) | ❌ | ❌ | ❌ |

✅ full · 🟡 partial · ❌ none. Versions are from each project's release notes / merge
requests (the per-interface numbers on wayland.app are unreliable snapshots).

Only **Sway** (≥ 1.12 / wlroots ≥ 0.20) is **runtime-verified** — it's the development
compositor. The others are inferred from their protocol support and haven't been exercised
end-to-end yet; reports welcome.

Two caveats:

- **Mutter / KWin** now implement `ext-image-copy-capture-v1` (screen capture), but **not
  `wlr-layer-shell`** — so the overlay tools (switcher, region select, loupe, wlr-draw) can't
  run there, and only non-interactive whole-output capture could work. They remain largely
  out of scope; their first-class path is the desktop portal / PipeWire, which this suite
  deliberately doesn't use.
- **niri / dwl** expose only the older `wlr-screencopy-v1`, which this suite doesn't use, so
  they don't work yet (an `ext-image-copy-capture` fallback would be needed).

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
