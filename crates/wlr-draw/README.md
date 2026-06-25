# wlr-draw

Draw and annotate **live on screen** on wlroots compositors — a native, Wayland-first
take on [gromit-mpx](https://github.com/bk138/gromit-mpx). A transparent, always-on-top
overlay floats over every output; toggle draw mode to scribble freehand strokes, lines,
rectangles, ellipses, arrows and text over whatever is on screen, then toggle it off to
go back to clicking through to your apps — the annotations stay visible until you clear
them.

Part of [wlr-utils](../../README.md); built on the shared `wlr-capture` engine (the
egui/EGL overlay toolkit). Each surface is a transparent vector layer the compositor
alpha-blends over the live screen — nothing is captured until you press `Space` to
freeze-frame, which grabs a still backdrop to annotate.

## How it works

A wlroots layer-shell client **cannot grab a global hotkey**, so — like gromit-mpx —
`wlr-draw` runs as a daemon and further invocations drive it over a per-user control
socket. You bind those invocations to compositor keys.

```sh
wlr-draw                 # start the daemon (the overlay; runs in the foreground)
wlr-draw toggle          # enter/leave draw mode (grab input ↔ click-through)
wlr-draw on | off        # force draw mode on/off
wlr-draw clear           # erase everything
wlr-draw undo | redo
wlr-draw visibility      # hide/show the annotations without discarding them
wlr-draw tool  <pen|rect|mask|arrow|text|eraser>   # mask = solid box to redact areas
wlr-draw color <name|#rrggbb[aa]>     # red green blue yellow orange cyan magenta white black
wlr-draw width <px>
wlr-draw quit            # stop the daemon
```

In **draw mode** the overlay grabs the pointer and keyboard; in **click-through** mode
it sets an empty input region so clicks and keys go straight to the apps underneath.
**Caps Lock** toggles a pointer pass-through *while staying in draw mode* — the pointer
reaches the apps below; tap it again to draw. **Hold `Ctrl`** while dragging a shape to
**constrain** it: rectangle → square, ellipse → circle, line/arrow → nearest 45°.
**Hold `Shift`** for a **spotlight**: the screen dims everywhere except a bright zone —
a circle that follows the cursor while idle, or a rectangle/ellipse you drag to place
(see below). The **wheel** (or `i`/`k`) resizes the light; the **tilt/second wheel** (or
`j`/`l`) darkens or lightens it.

### Keyboard shortcuts (while drawing)

The overlay holds keyboard focus in draw mode, so bare keys are local shortcuts — they
don't clash with the compositor's `$mod+…` bindings, so you only need to bind one key
(toggle) in your compositor. Press **`h`** for an on-screen legend.

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| `p` | pen | `c` | colour palette (click a swatch) |
| `r` | rectangle | `u` / `y` | undo / redo |
| `m` | mask (solid box) | `+` / `-` | width up / down |
| `a` | arrow | `Delete` | clear |
| `t` | text | `v` | hide / show |
| `w` | save annotated screenshot | | |
| `e` | eraser | `h` | toggle the help legend |
| `s` | move tool (or right-drag) | `Ctrl` | constrain shape / move axis (hold) |
| `Space` | freeze-frame on/off | `Shift` | spotlight (hold); wheel/`ijkl` size & dim |
| `Esc` | unfreeze / close popup / leave | | |
| `↑↓←→` | nudge selection (`Shift`: 1px, `Ctrl`: big) | | |

Shortcuts are by produced letter, so they follow your keyboard layout. While typing a
text label, keys go to the label (`Enter` commits, `Esc` cancels) instead. The status
chip shows the active tool, a **sample of the current stroke width** and its size, and
the colour — and it **pulses** a few times when you enter draw mode on an empty screen
(or jab repeatedly at one spot) to remind you you're drawing.

## Drawing

- **Pen** — freehand. **Eraser** — deletes whole strokes/shapes the cursor passes over.
- **Rect / Arrow** — press, drag to the far corner, release. The arrowhead is sized by
  the stroke width, not the arrow's length.
- **Mask** — a solid filled rectangle, for hiding/redacting an area (pick black, or any
  colour). Drag the box; the whole area is opaque.
- **Spotlight** ✨ — the inverse of a mask: dim everything *around* a shape to draw the
  eye to it (presenting, screencasts). **Hold `Shift`** and a flashlight follows the
  cursor; drag a rectangle (`r`/`m` tool) or pen-snap a circle while holding `Shift` to
  drop a fixed spotlight. They're ordinary elements (undo, erase by clicking the lit
  area), and several share **one veil** — overlapping lit zones merge with no seam and
  never darken each other. The **wheel**/`i`/`k` resize the light and the **tilt
  wheel**/`j`/`l` dim it (`+`/`-` stay stroke width); `Ctrl` squares/circles the dragged
  one. After dropping one, the cursor flashlight stays off until you release `Shift`.
- **Text** — pick the text tool, click to place a caret, type, `Enter` to commit
  (`Esc` cancels). Click again to start another label.
- **Move** — **right-drag** any element to move it without leaving the current tool, or
  press `s` for the move tool and **click an element to grab it** (a faint accent box
  marks the selection). Hold `Ctrl` while dragging to lock the move to one axis. In the
  move tool the **arrow keys** then nudge it — held to glide (key-repeat), `Shift` for
  1px-precise, `Ctrl` for a big step. Switching tools, undo/redo, clear or leaving draw
  mode deselects.
- **Save** — press `w` (or `wlr-draw save [path]`) to write the **annotated screen** (the
  output under the cursor) to a PNG in your Pictures directory. The capture is the
  composited output, so your strokes are baked in — works on a frozen frame too.
- **Freeze-frame** — press `Space` to **freeze the screen**: each output is captured and
  shown as a still backdrop so you can annotate (and spotlight) a frozen moment while
  everything keeps running underneath. `Space` again or `Esc` returns to live. Freeze a
  clean screen *before* drawing — existing strokes get baked into the capture.
- **Text size follows the stroke width** — `+`/`-` size both the strokes and the next
  text label (each label keeps the size it was placed at).
- **Dwell-to-snap** ✨ — there are no separate line/ellipse tools: with the **pen**,
  sketch a rough circle (or a straight line) and *hold the cursor still for a moment
  without releasing the button*. The freehand blob snaps to a clean ellipse (a perfect
  circle when roughly round) or a straight line, which you then **resize live** by
  moving the mouse. Release to commit.

The tray icon shows the **current tool** as a glyph (in the stroke colour while drawing,
grey when idle).

## Running the daemon

### systemd `--user` (recommended)

The daemon is a session service. A unit is provided in
[`contrib/wlr-draw.service`](contrib/wlr-draw.service) (bound to
`graphical-session.target`, so it starts/stops with the Wayland session — works with
uwsm, which imports `WAYLAND_DISPLAY` into the user manager):

```sh
install -Dm644 contrib/wlr-draw.service ~/.config/systemd/user/wlr-draw.service
# If wlr-draw is in ~/.local/bin (not on the user manager's PATH), point at it:
#   sed -i 's|^ExecStart=wlr-draw$|ExecStart=%h/.local/bin/wlr-draw|' \
#       ~/.config/systemd/user/wlr-draw.service
systemctl --user enable --now wlr-draw.service
```

### Or from the compositor

For a non-systemd session, launch it from the compositor instead — sway: `exec wlr-draw`.

### Tray icon

With the `tray` feature (on by default) the daemon shows a StatusNotifierItem tray icon
(e.g. in waybar's `tray` module): a hollow ring when idle, a filled disc in the current
stroke colour while drawing. Left-click toggles draw mode; the menu offers toggle /
clear / undo / quit, a **Shortcuts** submenu with the full key legend, and a
**Start on login** checkbox (see below). `--no-default-features` drops it (and the D-Bus
dependency).

### Start on login

The tray's **Start on login** entry writes (or removes) an XDG autostart desktop entry at
`~/.config/autostart/wlr-draw.desktop`, picked up by any XDG-compliant session — including
the systemd xdg-autostart generator under uwsm. The daemon also registers itself there
**once**, on its first ever run (tracked by a sentinel under `$XDG_STATE_HOME`), so it
starts with the session out of the box. After that the checkbox is the sole source of
truth: unchecking it is permanent, and a later manual launch won't re-create the entry.
This is an alternative to the hand-written unit above — use one or the other, not both.

## Example sway bindings

Bind the toggle (everything else is a key shortcut while drawing, so one bind is enough
— add more if you like driving it from outside draw mode):

```
bindsym $mod+d       exec wlr-draw toggle
bindsym $mod+Shift+d exec wlr-draw clear
bindsym $mod+z       exec wlr-draw undo
```

The protocol is plain text, one command per line on the socket
(`$XDG_RUNTIME_DIR/wlr-draw.sock`), so you can also drive it from scripts:
`echo 'tool arrow' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/wlr-draw.sock`.

## Build

```sh
cargo build --release -p wlr-draw
```

Needs a working EGL/GL stack (`libegl1`) and a wlroots compositor advertising
`wlr-layer-shell` (sway, Hyprland, niri, …). `--no-default-features` drops Fluent and
builds the UI hints English-only.

## Limitations

- Overlays are built for the outputs present at start-up; hot-plugged monitors are not
  picked up until the daemon is restarted.
- One daemon per session (a second `wlr-draw` exits with "already running").
