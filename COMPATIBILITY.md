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

`ext-image-copy-capture-v1` is the linchpin: it arrived in **wlroots 0.18** (Sway
≥ 1.10) and is **not** implemented by GNOME's Mutter or KDE's KWin, which only offer
screen capture through the desktop portal / PipeWire — out of scope here.

## Compositors

Legend: ✅ works · ◐ partial · ❌ unsupported · ❓ unverified. "Runtime-verified"
means actually run by the author; "protocol-verified" means the compositor's binary
is confirmed to advertise the required protocols (but the suite wasn't run on it).
Everything else is inferred — corrections and reports welcome.

| Compositor | Capture | Windows | Overlays | Clipboard | Focus IPC (`-a` / `--current-output`) | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| **Sway** ≥ 1.10 (wlroots ≥ 0.18) | ✅ | ✅ | ✅ | ✅ | ✅ `swaymsg` | **Runtime-verified** (the development compositor). |
| **Hyprland** 0.55 | ✅ | ✅ | ✅ | ✅ | ◐ `hyprctl` | **Protocol-verified**: the Hyprland 0.55 binary advertises every required protocol. The `hyprctl` focus backend is unit-tested against hyprctl's JSON but **not yet run on a live Hyprland**. |
| **niri** | ❓ | ❓ | ✅ | ❓ | ◐ `niri msg` — `--current-output` only | niri's IPC exposes no per-window global rectangle, so `-a` is unavailable; use `-g`/`--current-output`. Backend parsing unit-tested against the documented shape, otherwise **unverified**. |
| **river** | ❓ | ❓ | ✅ | ✅ | ❌ no backend | Capture needs river on wlroots ≥ 0.18. No focus IPC backend: use `-s`/`-g`/`-o`. ❓ |
| **Wayfire** | ❓ | ❓ | ✅ | ✅ | ❌ no backend | As river: depends on the wlroots version. ❓ |
| **COSMIC** (`cosmic-comp`) | ❓ | ❓ | ✅ | ❓ | ❌ no backend | Smithay-based; `ext-image-copy-capture` support unverified. ❓ |
| **Mutter** (GNOME) | ❌ | ❌ | ❌ | ❌ | ❌ | No `ext-image-copy-capture` / `wlr-layer-shell`; capture only via the portal. Out of scope. |
| **KWin** (KDE Plasma) | ❌ | ❌ | ❌ | ❌ | ❌ | Same as Mutter. |

Where the focus IPC is unsupported, only the focus-aware *sources* (`-a`,
`--current-output`) are affected — every other source (`-s` interactive select, `-g`
geometry, `-o NAME`, `-w ID`, `--pick-window`) works regardless.

## Adding a compositor

Focus backends live in [`crates/wlr-capture/src/focus.rs`](crates/wlr-capture/src/focus.rs):
implement `FocusBackend` (a `focused_output()` and an `active_window_rect()`) over
your compositor's IPC and add a detection branch in `detect()`. The Sway, Hyprland
and niri backends are short worked examples.
