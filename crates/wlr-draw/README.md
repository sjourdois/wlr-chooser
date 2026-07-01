# wlr-draw

Draw and annotate **live on screen** on wlroots compositors — a native, Wayland-first
take on [gromit-mpx](https://github.com/bk138/gromit-mpx). A transparent, always-on-top
overlay floats over every output; toggle draw mode to scribble freehand strokes, lines,
rectangles, ellipses, arrows and text over whatever is on screen, then toggle it off to
go back to clicking through to your apps — the annotations stay visible until you clear
them.

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-draw/annotate.gif"
       alt="wlr-draw annotating a screen with an arrow, a highlight box, a circled line and a text label" width="820">
</p>

Presenter **spotlight** — hold Shift to dim the screen except a flashlight that
follows the cursor, or pose a fixed spotlight on a window:

<p align="center">
  <img src="https://raw.githubusercontent.com/sjourdois/wlr-utils/main/docs/assets/wlr-draw/spotlight.gif"
       alt="wlr-draw presenter spotlight darkening the screen around a flashlight that follows the cursor" width="820">
</p>

Part of [wlr-utils](../../README.md); built on the shared `wlr-capture` engine (the
egui/EGL overlay toolkit). Each surface is a transparent vector layer the compositor
alpha-blends over the live screen — nothing is captured until you press `Space` to
freeze-frame, which grabs a still backdrop to annotate.

<p align="center"><sub>📖 See every tool in action on the <a href="https://sjourdois.github.io/wlr-utils/">showcase</a>.</sub></p>

## How it works

A wlroots layer-shell client **cannot grab a global hotkey**, so — like gromit-mpx —
`wlr-draw` runs as a daemon and further invocations drive it over a per-user control
socket — driven from a compositor key bind, the tray icon, or a script.

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
wlr-draw save [path]     # write the annotated screen to a PNG (Pictures dir by default)
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

### Customising shortcuts

Every shortcut above is rebindable from **`~/.config/wlr-draw/keys.toml`** (honours
`$XDG_CONFIG_HOME`). Key names are the same **XKB keysym names** sway/Hyprland use in
`bindsym` (`a`, `space`, `Caps_Lock`, `plus`, `F5`…), matched case-insensitively. Each
binding is a single name or a list; missing entries keep their default, so a partial file
is fine and no config at all means the defaults below.

The three held controls — `passthrough` (click-through), `constrain`, `spotlight` — take
**either a modifier** (`caps`, `ctrl`, `shift`, `alt`, `super`) **or a regular key**. This
is the fix for keyboards without a usable Caps Lock (e.g. HHKB): point `passthrough` at
`alt`, `super`, or any key. A modifier engages while held (Caps Lock latches); a regular
key bound to `passthrough` toggles, and to `constrain`/`spotlight` engages while held.

A commented example listing every binding with its default is at
[`docs/wlr-draw-keys.toml`](../../docs/wlr-draw-keys.toml) — copy it to
`~/.config/wlr-draw/keys.toml` and edit.

Fixed (not rebindable): `Esc` (always backs out), the arrow-key nudge, and the spotlight
size/dim cluster (`i`/`j`/`k`/`l` + wheel, live only while spotlighting). The nudge step
size still reads the physical `Shift` (1px) / `Ctrl` (big) keys. A bad name or a key bound
to two things is reported on stderr and the default is kept. The on-screen `h` legend and
the tray's Shortcuts menu reflect your bindings.

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

### Start on login (default)

With the `tray` feature (on by default) **nothing needs installing**: on its very first
run the daemon registers an XDG autostart entry at `~/.config/autostart/wlr-draw.desktop`
(tracked by a sentinel under `$XDG_STATE_HOME`), so it comes up with the session out of
the box — picked up by any XDG-compliant session, including the systemd xdg-autostart
generator under uwsm.

After that first run the desktop file's presence is the sole source of truth: the tray's
**Start on login** checkbox writes or removes it, and unchecking it is permanent — a later
manual launch won't recreate an entry you deliberately dropped.

### Logs

The daemon logs to stderr, which the session journals. Filter by the **binary name**, not
the unit — it's clean and works however the daemon was started:

```sh
journalctl --user -t wlr-draw -f
```

(A default XDG-autostart launch shows up under the systemd unit `app-wlr\x2ddraw@…` — the
`\x2d` is just systemd escaping the dash in the desktop-file name, which is awkward to
type. `-t wlr-draw` sidesteps it. If you want a tidy unit name in the journal too, run the
daemon from the systemd user unit below instead, and it appears as `wlr-draw.service`.)

Restarting the autostart daemon (e.g. after installing a new build) uses that same escaped
unit name — quote it so the shell keeps the backslash:

```sh
systemctl --user restart 'app-wlr\x2ddraw@autostart.service'
```

### Tray icon

With the `tray` feature (on by default) the daemon shows a StatusNotifierItem tray icon
(e.g. in waybar's `tray` module): a hollow ring when idle, a filled disc in the current
stroke colour while drawing. Left-click toggles draw mode; the menu offers toggle /
clear / undo / quit, a **Shortcuts** submenu with the full key legend, and the **Start on
login** checkbox above. `--no-default-features` drops it (and the D-Bus dependency).

### Without the tray: systemd or the compositor

A `--no-default-features` build has no tray and so no self-registering autostart — start
the daemon yourself. Either drop in the provided systemd `--user` unit
([`contrib/wlr-draw.service`](contrib/wlr-draw.service), bound to
`graphical-session.target` so it tracks the Wayland session — works with uwsm, which
imports `WAYLAND_DISPLAY` into the user manager):

```sh
install -Dm644 contrib/wlr-draw.service ~/.config/systemd/user/wlr-draw.service
# If wlr-draw is in ~/.local/bin (not on the user manager's PATH), point at it:
#   sed -i 's|^ExecStart=wlr-draw$|ExecStart=%h/.local/bin/wlr-draw|' \
#       ~/.config/systemd/user/wlr-draw.service
systemctl --user enable --now wlr-draw.service
```

…or launch it straight from the compositor — sway: `exec wlr-draw`. Use one mechanism, not
several.

## Example sway bindings

**The only binding you need is `wlr-draw toggle`.** Once draw mode is on, the overlay holds
keyboard focus, so every other tool is a bare key shortcut while drawing (see the table
above) — no extra compositor binds required. And if you have the tray, **left-clicking its
icon toggles draw mode too**, so even that one binding is optional.

The bindings below are just conveniences for driving the daemon from *outside* draw mode:

```
bindsym $mod+d       exec wlr-draw toggle
bindsym $mod+Shift+d exec wlr-draw clear
bindsym $mod+z       exec wlr-draw undo
```

The protocol is plain text, one command per line on the socket
(`$XDG_RUNTIME_DIR/wlr-draw.sock`), so you can also drive it from scripts:
`echo 'tool arrow' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/wlr-draw.sock`.

## Install

> **Want the whole suite?** Install the bundle instead — `cargo install wlr-utils` gets
> every tool (`wlr-chooser`, `wlr-switcher`, `wlr-peek`, `wlr-shot`, `wlr-draw`) in one
> go. The single-tool install below is the lighter, à-la-carte option.

```sh
cargo install wlr-draw
```

Or build just this binary from the [wlr-utils](../../README.md) workspace:

```sh
cargo build --release -p wlr-draw
```

`--no-default-features` drops Fluent (English-only hints) **and** the tray.

## Requirements

- **GL stack** — `libegl1` at runtime; the overlay renders through EGL/GLES.
- **Compositor** — a wlroots one advertising `wlr-layer-shell` (sway, Hyprland, niri, …)
  for the always-on-top overlay. Plain annotation needs only that, at any version.
- **Screen capture** (freeze-frame `Space`, save `w`) — additionally needs
  `ext-image-copy-capture-v1` with the **output** source, i.e. **Sway ≥ 1.11 /
  wlroots ≥ 0.19**. Where it's missing, freeze and save are hidden from the help/tray and
  plain annotation still works. See [COMPATIBILITY.md](../../COMPATIBILITY.md).
- **Tray** (`tray` feature, on by default) — a StatusNotifierItem host and `libdbus`.
  `--no-default-features` drops the tray and its D-Bus dependency.

## Uninstall

Remove the binary, then the autostart entry it registers on first run (and the systemd
unit if you installed one — paths honour `$XDG_CONFIG_HOME` / `$XDG_STATE_HOME`):

```sh
cargo uninstall wlr-draw      # if installed from crates.io
rm -f ~/.config/autostart/wlr-draw.desktop \
      ~/.local/state/wlr-draw/autostart-initialized
# optional systemd unit:
systemctl --user disable --now wlr-draw.service
rm -f ~/.config/systemd/user/wlr-draw.service
```

## Limitations

- Overlays are built for the outputs present at start-up; hot-plugged monitors are not
  picked up until the daemon is restarted.
- One daemon per session (a second `wlr-draw` exits with "already running").
