# wlr-peek

Inspect the screen on wlroots compositors. The "look at the screen and extract
something" companion to [`wlr-shot`](../wlr-shot) (which produces image artifacts),
built on the shared [`wlr-capture`](../wlr-capture) engine.

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
$ wlr-peek mirror <ID>             # mirror a window (ID as printed by wlr-chooser)
$ wlr-peek mirror                  # no ID: launch wlr-chooser to pick one
$ wlr-peek mirror -g "100,200 640x480"        # live magnifier of a fixed region
$ wlr-peek mirror -g "100,200 640x480" --zoom 4
```

Drag to move, the bottom-right grip to resize, the toolbar to collapse to a badge or
close; **Space** freezes, **c** collapses, **+/-** or the wheel set opacity, **r**
re-picks, **Esc** closes. Pair with sway rules `floating enable, sticky enable` for
always-on-top across workspaces.

With `-g "X,Y WxH"` it mirrors a fixed logical region instead of a window — a live,
always-on-top loupe, magnified by `--zoom` (default ×2). Keep the window outside the
region to avoid feedback. Mono-output for now (the region is clipped to the output
its top-left corner sits on); `r` re-pick is disabled in region mode.

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

## License

MIT OR Apache-2.0.
