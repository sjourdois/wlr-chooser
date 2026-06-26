# wlr-peek

Inspect the screen on wlroots compositors. The "look at the screen and extract
something" companion to [`wlr-shot`](../wlr-shot) (which produces image artifacts),
built on the shared [`wlr-capture`](../wlr-capture) engine.

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-peek/color.gif"
       alt="wlr-peek colour picker: a magnifying loupe and the hex value of the pixel under the crosshair" width="410">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-peek/loupe.gif"
       alt="wlr-peek loupe: a full-screen magnifier panning and zooming" width="410">
</p>
<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-peek/mirror.png"
       alt="wlr-peek mirror: a floating picture-in-picture window zooming a region of the screen" width="410">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-peek/cli.gif"
       alt="wlr-peek CLI: ocr and watch running against a page on screen" width="410">
</p>

<p align="center"><sub>📖 See every tool in action on the <a href="https://sjourdois.github.io/wlr-utils/">showcase</a>.</sub></p>

## Subcommands

### `color` — colour picker (pipette)

Freezes every output, shows a magnifying loupe that follows the cursor with the hex
value of the pixel under the crosshair, and picks the pixel you click.

```console
$ wlr-peek color                 # prints e.g. #4D9AFF
$ wlr-peek color --format rgb     # rgb(77, 154, 255)
$ wlr-peek color --clipboard      # copy to the Wayland clipboard instead
```

- Move the cursor; the loupe magnifies the pixels around it.
- **Click** (or **Enter**) to pick the pixel under the crosshair.
- **Esc** to cancel (exit status 1).

`--format hex|rgb|plain` chooses the output. `--clipboard` runs a small background
daemon that serves the colour as text on the wlroots clipboard until replaced.

### `ocr` — recognise text in a region (Tesseract)

Captures a source and runs it through Tesseract, printing the recognised text.

```console
$ wlr-peek ocr                    # select a region interactively (default)
$ wlr-peek ocr -g "100,200 640x480"
$ wlr-peek ocr --active-window     # OCR the focused window (needs compositor IPC)
$ wlr-peek ocr -l fra+eng -c       # French+English, copy to the clipboard
```

With no source flag it selects a region interactively (the default). Other sources
mirror `wlr-shot`: `-g "X,Y WxH"`, `-o NAME`, `-a/--active-window`, `--current-output`.
`-l/--lang` picks the Tesseract language(s) (default `eng`; the matching
`tesseract-ocr-<lang>` data pack must be installed).

OCR is behind the `ocr` Cargo feature (**on by default**); it links system
`libtesseract`/`libleptonica`. Build `--no-default-features` for an OCR-free binary
with no native OCR dependencies.

### `loupe` — full-screen magnifier

Freezes the screen and magnifies around the cursor; the point under the cursor stays
put as you move, scroll to change the zoom, **Esc** to quit.

```console
$ wlr-peek loupe
```

It is **frozen**, not live: a full-screen *live* magnifier would capture its own
output (a feedback loop), and Wayland does not give a regular client the global
cursor position to follow it from a floating window. For a *live* zoom of a fixed
region, use `mirror -g` (below).

### `mirror` — floating live mirror (picture-in-picture)

A floating, always-on-top window that mirrors live content. This is the former
`wlr-pip`, now a `wlr-peek` subcommand.

```console
$ wlr-peek mirror                  # no source: launch wlr-chooser to pick a window
$ wlr-peek mirror <ID>             # mirror a window (ID as printed by wlr-chooser)
$ wlr-peek mirror -w               # pick a window via the chooser (explicit)
$ wlr-peek mirror -s               # select a region with the mouse, then mirror it
$ wlr-peek mirror -o DP-4          # mirror a whole output / screen
$ wlr-peek mirror --current-output # the focused output
$ wlr-peek mirror -a               # the active window's area (needs focus info)
$ wlr-peek mirror -g "100,200 640x480" --zoom 4   # a fixed region, magnified
```

It takes the **same source flags as the rest of wlr-utils**: a window (`id`,
`-w`/`--pick-window`, `-a`/`--active-window`), or a region/output mirrored as a live
loupe (`-s` interactive, `-g "X,Y WxH"`, `-o NAME`, `--current-output`), magnified by
`--zoom` (default ×2). Region/output mode is mono-output for now (clipped to the output
its top-left corner sits on). Keep the window outside the mirrored region to avoid
feedback.

For a region, **`--follow`** chooses what it tracks: `output` (default — shows whatever
workspace is on that screen) or `window` — captures the **window under the region** and
crops to it, so the loupe follows that window across moves and workspaces (needs
compositor focus info; falls back to `output` if no window is under the region).

```console
$ wlr-peek mirror -s --follow window   # loupe that sticks to the window you picked
```

Drag to move, the bottom-right grip to resize, the toolbar to collapse to a badge or
close; **Space** freezes, **c** collapses, **+/-** or the wheel set opacity, **r**
re-picks (window mode), **Esc** closes. Pair with sway rules `floating enable, sticky
enable` for always-on-top across workspaces.

### `region` — select a region/point, print its geometry (slurp replacement)

Reuses the same frozen overlay as `wlr-shot -s` to select a region with the mouse and
print it as `X,Y WxH` (slurp's format) — a native, dependency-free slurp replacement.
Exits 1 if cancelled (Esc).

```console
$ wlr-peek region                  # drag a region → "X,Y WxH"
$ wlr-peek region -p               # pick a point → "X,Y"
$ wlr-peek region -f '%x %y %w %h' # custom format
$ grim -g "$(wlr-peek region)" shot.png        # feed any slurp-compatible tool
```

### `watch` — change monitor

Streams a source and fires when its content **changes**, or once it goes **idle**
(stops changing). Same sources as the other tools: `-s` (interactive region), `-g`,
`-o`, `--current-output`, `-w`/`--pick-window`, `-a` (a region is single-output, like
`mirror`/`record`).

```console
$ wlr-peek watch -g "$(slurp)" && notify-send "it changed"   # fire once, then notify
$ wlr-peek watch -w "$ID" --on idle --for 5s                 # wake when the window settles
$ wlr-peek watch -o DP-4 --on change --threshold 2 --repeat --exec 'mpc next'
```

- `--on change` (default) fires when the content changes; `--on idle` fires once it
  has been stable for `--for` (e.g. `5s`).
- `--threshold PCT` ignores changes smaller than that percentage of the watched
  pixels (default 0 = any change) — raise it to skip a blinking cursor or clock.
- By default it prints one line and exits 0 on the first trigger (composes with
  `&&`); `--repeat` keeps watching and fires every time. `--exec CMD` runs a shell
  command on each trigger. `--timeout DUR` gives up (exit 2) if nothing fires.

Capture is damage-driven, so `watch` is cheap: a static source delivers no frames.

### `grep` — find text on screen (visual grep)

OCRs a source and prints where matching text is, in global logical coordinates
(slurp-compatible `X,Y WxH`, so it feeds `mirror`/`shot`/other tools). Needs the
`ocr` feature (Tesseract).

```console
$ wlr-peek grep --current-output "TODO"
2741,318 58x19	TODO
$ wlr-peek grep -g "$(slurp)" -i error      # case-insensitive, in a region
```

Sources: `-g`, `-o`, `-a`, `--current-output`, or (default) an interactive region.
Matching is a substring (`-i` for case-insensitive); coordinates map back to a single
output. Exits 1 when nothing matches, like `grep`.

## Requirements

A wlroots compositor exposing `ext-image-copy-capture-v1` with the output **and**
foreign-toplevel capture sources — **Sway ≥ 1.12 / wlroots ≥ 0.20** (the floor for the
window source; the tools open it even for screen captures). The frozen overlays (`color`,
`loupe`, `region`) use `zwlr-layer-shell`, `mirror` uses `xdg-shell`, and `color
--clipboard` needs `zwlr_data_control_manager_v1`. `wlr-peek doctor` prints exactly what
your compositor advertises.

- **GL stack** — `libegl1` at runtime (the overlays render through EGL/GLES). Capture is
  CPU shm, so **no `libgbm`** is needed (the `gpu` feature is off by default).
- **OCR** (`ocr`, `grep`; on by default) — links the system `libtesseract`/`libleptonica`
  (`-dev` packages, plus `clang` for the bindings); the matching `tesseract-ocr-<lang>`
  data pack must be installed at runtime (default `eng`). `--no-default-features` builds an
  OCR-free binary with none of these.
- **Focus IPC** — `--active-window` / `--current-output` query the compositor (e.g.
  `swaymsg` on Sway); without a supported compositor, use an explicit source instead.

## Install

> **Want the whole suite?** Install the bundle instead — `cargo install wlr-utils` gets
> every tool (`wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`, `wlr-draw`) in one
> go. The single-tool install below is the lighter, à-la-carte option.

```sh
cargo install wlr-peek
```

Or build just this binary from the [wlr-utils](../../README.md) workspace:

```sh
cargo build --release -p wlr-peek
```

`--no-default-features` drops OCR (and Fluent) for a lighter binary with no native OCR
dependencies — see **Requirements** above for what each feature pulls in.

## Uninstall

```sh
cargo uninstall wlr-peek          # crates.io install; or: rm -f ~/.local/bin/wlr-peek
```

wlr-peek writes no config or state files, so removing the binary is all it takes.

## License

MIT OR Apache-2.0.
