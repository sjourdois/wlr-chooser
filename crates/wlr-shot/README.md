# wlr-shot

Screen capture for **wlroots** compositors, built on the shared
[`wlr-capture`](../wlr-capture) engine (`ext-image-copy-capture-v1`, correct
strides, occlusion-independent).

> **Status:** early. It captures an output, a region (interactive `-s` or
> `-g`/slurp), or a window — as a screenshot (PNG/JPEG/PPM) or an H.264 recording /
> timelapse.

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-shot/select.gif"
       alt="wlr-shot's interactive region selector: dragging a bright rectangle over a dimmed, frozen screen" width="820">
</p>

<p align="center"><sub>📖 See every tool in action on the <a href="https://sjourdois.github.io/wlr-utils/">showcase</a>.</sub></p>

## Usage

```sh
wlr-shot screenshot [-o NAME | -g GEOM | -w ID | --pick-window]
                    [-t png|jpeg|ppm] [-q QUALITY] [-c] [FILE|-]
wlr-shot screenshot --list-outputs
wlr-shot record [-o NAME | -g GEOM | -w ID | --pick-window | -s | -a]
                [--encoder auto|nvenc|vaapi|software] [--fps N]
                [--timelapse INTERVAL] [-d SECS] FILE
```

Source (pick one; defaults to the sole output):

- `-s, --select` — **interactively** drag a region on a frozen overlay (spans all
  outputs; `Esc` cancels, `Enter` confirms). No external tool needed.
- `-o, --output NAME` — a whole output (e.g. `DP-4`).
- `--all` — the whole layout: every output combined into one image.
- `-g, --geometry "X,Y WxH"` — a logical region (the format **slurp** prints),
  stitched across every output it covers. Pairs with slurp:
  `wlr-shot screenshot -g "$(slurp)" shot.png`.
- `-w, --window ID` — a window, by its `ext-foreign-toplevel` identifier (as
  printed by `wlr-chooser`).
- `--pick-window` — launch `wlr-chooser` to choose the window interactively.
- `-a, --active-window` — the focused window.
- `--current-output` — the focused output.

The last two need the compositor's focus info. Wayland exposes no portable way to
query focus, so these use compositor IPC: **Sway** (`swaymsg`) is supported today;
Hyprland / niri are natural future additions. Without a supported compositor they
error with a hint (use `--pick-window` / `-o NAME` instead).

Encoding & destination:

- `-t, --type` — `png` (default), `jpeg`, or `ppm`.
- `-q, --quality` — JPEG quality, 1–100 (default 90).
- `-c, --clipboard` — copy to the Wayland clipboard instead of writing a file. A
  small daemon detaches to serve the selection (wlroots `data-control`, the
  protocol `wl-copy` uses) until another client replaces it — the clipboard is
  pull-based, so the data must outlive the command. `--clipboard-foreground` keeps
  it in the foreground (for scripts/debugging). Needs a compositor exposing
  `zwlr_data_control_manager_v1`.
- `FILE` — destination, or `-` for stdout (the default). Ignored with `--clipboard`.
- `--list-outputs` — print `NAME<TAB>WxH+X,Y` (logical geometry) and exit.

Resolution: a whole output, or a region within a **single** output, is captured at
**native (physical) resolution** — so a fractionally-scaled monitor keeps full
pixel detail. A region spanning **several** outputs is composited at logical
resolution.

## Recording (`record`)

Stream a source to a file whose **format follows the extension**: `.mp4`/`.mkv` for
H.264 video, or **`.gif`/`.webp`** for an animated image (downscaled — GIF to 800 px,
WebP to 1280 px on the long side — and best used on a **region**, since per-frame GIF
quantization on a full 4K output is slow). The same source flags as `screenshot` apply — `-o`/sole output,
`--current-output`, `-w ID`/`--pick-window`, `-a`, `-g`, and `-s` — except a region
(`-g`/`-s`) records a **single** output for now (the one its top-left corner sits
on). Recording a **window** (`-w`/`--pick-window`) follows it across workspaces and
even while occluded.

```sh
wlr-shot record -o DP-4 out.mp4                 # an output, until Ctrl-C
wlr-shot record --pick-window -d 30 clip.mp4    # a window, 30 seconds
wlr-shot record -g "$(slurp)" region.mp4        # a region (single output)
wlr-shot record -g "$(slurp)" --fps 15 demo.gif # a region as an animated GIF
wlr-shot record -g "$(slurp)" demo.webp         # …or animated WebP (smaller)
wlr-shot record -o DP-4 --timelapse 2s day.mp4  # a frame every 2s, played at --fps
wlr-shot record -o DP-4 --no-audio clip.mp4     # video only (audio is on by default)
```

- `--encoder` — `auto` (default) prefers hardware (NVENC, then VAAPI) and falls back
  to software `libx264`. Force one with `nvenc`/`vaapi`/`software`.
- `--device PATH` — DRM render node for VAAPI (default `/dev/dri/renderD128`).
- `--fps N` — frame rate (default 30). Capture is damage-driven (a frame only arrives
  when the screen changes), so a normal recording emits a **constant** `--fps`,
  repeating the last frame through static stretches; `--timelapse` instead samples one
  frame per interval and plays them back at `--fps`, so the footage is sped up.
- `--timelapse INTERVAL` — sample one frame every `INTERVAL` (`2s`, `500ms`, `1m`).
- `-d, --duration SECS` — stop automatically; otherwise **Ctrl-C** ends and finalises
  the file. (Recording a window also ends when the window closes.)
- **Audio** — a video recording captures **system audio** (the default sink's monitor)
  as an AAC track by default, via **native PipeWire**. `--no-audio` records silently;
  `--audio-source NODE` captures a specific node instead (e.g. a microphone). Needs the
  `audio` build feature (on by default; links libpipewire). GIF/WebP carry no audio.
  For a PipeWire-less host, build with `--features audio-fallback` to add a **Pulse/ALSA**
  path (via FFmpeg's libavdevice, no extra system dep), tried after PipeWire.

Recording needs the `video` build feature (on by default), which links the system
FFmpeg libraries. A screenshots-only build drops it: `cargo build -p wlr-shot
--no-default-features --features i18n`.

## Install

> **Want the whole suite?** Install the bundle instead — `cargo install wlr-utils` gets
> every tool (`wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`, `wlr-draw`) in one
> go. The single-tool install below is the lighter, à-la-carte option.

```sh
cargo install wlr-shot
```

Or build just this binary from the [wlr-utils](../../README.md) workspace:

```sh
cargo build --release -p wlr-shot
```

The default build records video and links the system FFmpeg libraries (see
**Requirements** below). For a screenshots-only binary with no FFmpeg dependency:
`cargo build -p wlr-shot --no-default-features --features i18n`.

## Requirements

A wlroots compositor exposing `ext-image-copy-capture-v1` with both the
`ext-output-image-capture-source-manager-v1` (outputs) and
`ext-foreign-toplevel-image-capture-source-manager-v1` (windows) sources — **Sway ≥ 1.12 /
wlroots ≥ 0.20**, the floor for the window source. `xdg-output` is used for accurate
logical geometry when present, and the clipboard (`-c`) needs
`zwlr_data_control_manager_v1`. Run `wlr-peek doctor` to see what your compositor exposes.

The interactive region selector (`-s`) renders a frozen overlay through EGL/GLES, so
**every build** needs a working GL stack (`libegl1`) at runtime. Screenshots use CPU shm
capture, so no `libgbm` is required.

The default build (with `record`) additionally links the system **FFmpeg** libraries, so
it needs their development packages at build time — on Debian/Ubuntu: `libavcodec-dev
libavformat-dev libavutil-dev libavfilter-dev libavdevice-dev libswscale-dev
libswresample-dev libva-dev` (and `clang` for the bindings). Hardware encoding needs
the matching runtime: NVIDIA's `libnvidia-encode` for NVENC, or a VAAPI driver for
your GPU. The default `audio` feature records system sound through **PipeWire**, linking
`libpipewire-0.3` (and `clang` to build); drop it with `--features video` (video only) or
build screenshots-only (`--no-default-features --features i18n`) to need none of this.

## Uninstall

```sh
cargo uninstall wlr-shot          # crates.io install; or: rm -f ~/.local/bin/wlr-shot
```

wlr-shot writes no config or state files — removing the binary is enough.

## License

MIT OR Apache-2.0.
