# Screenshot generator

Reproducible, hands-off screenshots and short animations of every wlr-utils
tool, used in the project READMEs and the GitHub Pages showcase.

Each scene spins up an **isolated, headless nested sway** on its own
`WAYLAND_DISPLAY`, populates a small desktop, drives the tool with a synthetic
pointer + keyboard, and captures the result. The nested compositor uses the
headless wlroots backend (virtual in-memory outputs, no DRM master), so it runs
safely **alongside a live session and never touches your real screens**.

## Usage

```sh
cd tools/screenshots
./capture.sh              # build the tools and regenerate every asset
./capture.sh draw peek    # only the named scenes
SKIP_BUILD=1 ./capture.sh # reuse target/release binaries
```

Output lands in `../../docs/assets/<tool>/` as `*.png` (still) plus `*.apng`,
`*.webp` and `*.gif` (animations) where the scene is dynamic.

To render the overlays in French (or any locale): `SHOTS_LANG=fr_FR.UTF-8 ./capture.sh`.

## Requirements

System tools: `sway`, `grim`, `wtype`, `foot`, `ffmpeg`, ImageMagick, plus
`batcat`/`tree` for the demo windows. The first run also builds a tiny
virtual-pointer injector:

```sh
( cd pointer && cargo build --release )
```

## Layout

| Path | Role |
|------|------|
| `lib.sh` | nested-compositor lifecycle, input injection, capture helpers |
| `nested-sway.conf` | the isolated compositor's config (one virtual output) |
| `foot.ini` | dark theme for the demo terminals |
| `pointer/` | `shots-pointer`, a `zwlr_virtual_pointer_v1` injector (standalone crate, **not** in the workspace) |
| `scenes/*.sh` | one scene per tool |
| `capture.sh` | orchestrator: build + run every scene |

## Notes

- **Why a virtual pointer?** A headless seat has no input devices, so it has no
  pointer capability and sway's `seat cursor` IPC delivers nothing to clients.
  `shots-pointer` creates a real virtual pointer, which the overlays then see.
- **Why isolated per-crate builds?** Building the workspace together lets Cargo
  feature-unification enable `wlr-capture/gpu`, which routes capture through
  dma-buf + a GPU readback; the second EGL connection then hits
  `eglCreateWindowSurface: BadAlloc`. Building each tool with its own
  `cargo build -p` keeps it on the shm capture path.
- Run scenes one at a time — they share a fixed nested IPC socket path and must
  not overlap. `capture.sh` already serialises them.
