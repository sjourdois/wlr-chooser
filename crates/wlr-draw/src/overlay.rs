//! The annotation daemon: a transparent `wlr-layer-shell` overlay per output that you
//! draw on, in the spirit of gromit-mpx.
//!
//! Unlike the frozen [`wlr_capture::overlay`], nothing is captured — each surface is a
//! transparent vector layer cleared to `[0,0,0,0]` and alpha-blended over the live
//! screen by the compositor. Strokes live in global logical coordinates (so they span
//! outputs); each surface paints the portion in its own area.
//!
//! Draw mode is toggled over the control socket ([`crate::ipc`]), because a layer-shell
//! client cannot grab a global hotkey. Entering draw mode sets a full input region and
//! `Exclusive` keyboard so the overlay receives the pointer and keystrokes; leaving it
//! sets an *empty* input region and `None` keyboard, so clicks and keys pass straight
//! through to the apps below while the annotations stay on screen. While drawing, Caps
//! Lock toggles a pointer pass-through (reach the apps below without leaving draw mode)
//! and Ctrl constrains the shape being drawn (square / circle / axis-locked line).
//! Control also arrives over the socket (clear/undo/tool…); while drawing, the overlay
//! additionally takes single-key shortcuts (incl. `h` help, `c` colour picker) and text.

use crate::ipc;
use crate::keymap::{Action, Keymap, ModKind, Trigger};
use crate::model;
use crate::model::{Color, Document, Element, Recognized, ShapeKind, Tool, constrain, recognize};
use crate::proto::Cmd;
use crate::tr;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    reexports::calloop::channel::{Channel, Event as ChannelEvent, channel},
    reexports::calloop::{EventLoop, LoopHandle},
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{AxisScroll, PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wlr_capture::render::Gpu;
use wlr_capture::theme::Theme;
use wlr_capture::{capture, wl};

/// Default stroke colour and width before the user changes them.
const DEFAULT_COLOR: Color = [0xff, 0x3b, 0x30, 0xff]; // red
const DEFAULT_WIDTH: f32 = 4.0;
/// Text label size is derived from the current stroke width, so one size control drives
/// both. The ratio keeps the default width (4) at the long-standing 28 px font.
const TEXT_SIZE_RATIO: f32 = 7.0;
const TEXT_SIZE_MIN: f32 = 14.0;
const TEXT_SIZE_MAX: f32 = 220.0;
/// Eraser hit radius around the cursor (logical px).
const ERASE_RADIUS: f32 = 10.0;
/// How close (logical px) a click must be to grab an element (Move tool / right-drag).
const SELECT_RADIUS: f32 = 8.0;
/// Pointer buttons we act on: left draws, right grabs-and-moves.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// Time budget for the one-shot capture behind freeze-frame.
const FREEZE_BUDGET: Duration = Duration::from_secs(2);

/// Font size for a text label at the given stroke width.
fn text_size(width: f32) -> f32 {
    (width * TEXT_SIZE_RATIO).clamp(TEXT_SIZE_MIN, TEXT_SIZE_MAX)
}
/// How long the pen must hold still mid-stroke to snap the freehand shape to a clean
/// one (dwell-to-snap).
const DWELL: Duration = Duration::from_millis(650);
/// Pointer travel under this (logical px) counts as "still" for the dwell timer.
const PEN_STILL_EPS: f32 = 3.0;
/// How long the status chip pulses to draw the eye when draw mode is entered on an
/// empty canvas, and how many on/off blinks fit in that window.
const FLASH_DURATION: Duration = Duration::from_millis(1500);
const FLASH_CYCLES: f32 = 3.0;
/// Repeated clicks within this radius (logical px) and gap re-pulse the chip — the user
/// is likely jabbing at the same spot wondering why their app won't respond.
const CLICK_CLUSTER_RADIUS: f32 = 60.0;
const CLICK_CLUSTER_GAP: Duration = Duration::from_millis(900);
const CLICK_CLUSTER_N: u32 = 4;
/// Default spotlight veil opacity over the dimmed area (0..255 alpha of black). ~65%.
const DEFAULT_SPOTLIGHT_DIM: f32 = 165.0;
const SPOTLIGHT_DIM_MIN: f32 = 70.0;
const SPOTLIGHT_DIM_MAX: f32 = 240.0;
/// Default radius of the cursor "flashlight" hole (logical px); resized with the wheel
/// or i/k while Shift is held.
const DEFAULT_SPOTLIGHT_RADIUS: f32 = 160.0;
const SPOTLIGHT_RADIUS_MIN: f32 = 40.0;
const SPOTLIGHT_RADIUS_MAX: f32 = 600.0;
/// Per-notch steps for the spotlight wheel/key controls.
const SPOTLIGHT_RADIUS_STEP: f32 = 20.0;
const SPOTLIGHT_DIM_STEP: f32 = 14.0;
/// Scanline height (logical px) for painting the veil minus its holes: small enough that
/// elliptical holes read as smooth.
const VEIL_SCAN: f32 = 2.0;

/// One output's transparent overlay surface and its GL context.
struct Surface {
    layer: LayerSurface,
    /// Output top-left in the global logical space (maps pointer ↔ stroke coords).
    logical_x: i32,
    logical_y: i32,
    egui_ctx: egui::Context,
    gpu: Option<Gpu>,
    width: u32,
    height: u32,
    scale: u32,
    /// This output's frozen screenshot (freeze-frame), uploaded as an egui texture.
    frozen_tex: Option<egui::TextureHandle>,
}

impl Surface {
    fn ensure_gpu(&mut self, conn: &Connection) {
        if self.gpu.is_some() || self.width == 0 {
            return;
        }
        let (pw, ph) = (
            (self.width * self.scale) as i32,
            (self.height * self.scale) as i32,
        );
        self.gpu = Some(Gpu::new(conn, self.layer.wl_surface(), pw, ph));
    }

    /// Repaint: clear transparent, then the document and any in-progress gesture, all
    /// translated into this surface's local coordinates, plus the draw-mode HUD.
    fn render(&mut self, conn: &Connection, frame: &Frame) {
        self.ensure_gpu(conn);
        if self.width == 0 {
            return;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width as f32, self.height as f32),
            )),
            ..Default::default()
        };
        let (lx, ly) = (self.logical_x as f32, self.logical_y as f32);
        let frozen_tex = self.frozen_tex.as_ref();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.render(
            &self.egui_ctx,
            raw_input,
            self.scale as f32,
            (pw, ph),
            [0.0, 0.0, 0.0, 0.0], // transparent: the live screen shows through
            |ui, _imp| paint(ui, lx, ly, frame, frozen_tex),
        );
        self.layer.commit();
    }
}

/// What's in progress under the cursor between press and release.
enum Gesture {
    None,
    /// Freehand: accumulated points (global logical).
    Pen(Vec<(f32, f32)>),
    /// A two-corner shape anchored at `start`; the far corner follows the cursor.
    Shape(ShapeKind, (f32, f32)),
    /// A freehand loop the pen dwelled on, snapped to a clean ellipse centred at
    /// `center` and resized live by the cursor (a perfect circle when `circle`).
    SnapEllipse {
        center: (f32, f32),
        circle: bool,
    },
    /// An eraser drag is active (deletions happen on motion).
    Erase,
    /// Dragging the selected element. `start` is the pointer at grab time and `applied`
    /// the offset already added to the element, so each motion re-derives the target
    /// offset from the cursor — which lets Ctrl axis-constrain the whole move.
    Move {
        start: (f32, f32),
        applied: (f32, f32),
    },
}

/// An immutable view of the drawing state handed to each surface's painter.
struct Frame<'a> {
    elements: &'a [Element],
    gesture: &'a Gesture,
    /// Text being typed, if any: `(pos, buffer)` in global logical coordinates.
    text_edit: &'a Option<((f32, f32), String)>,
    theme: &'a Theme,
    /// Resolved keybindings, for the help legend's key column.
    keymap: &'a Keymap,
    tool: Tool,
    color: Color,
    width: f32,
    pointer: Option<(f64, f64)>,
    draw_mode: bool,
    /// Pointer click-through active (the pass-through bind).
    passthrough: bool,
    /// Constrain role active (the constrain bind): square/circle/axis-locked shapes.
    constrain: bool,
    /// Spotlight role active (the spotlight bind): dim around a shape / a cursor flashlight.
    spotlight: bool,
    /// Radius of the cursor flashlight hole (logical px).
    spotlight_radius: f32,
    /// Veil opacity (0..255 alpha of black).
    spotlight_dim: u8,
    /// Whether the idle cursor flashlight may show (false right after placing a spotlight).
    flashlight: bool,
    visible: bool,
    show_help: bool,
    show_palette: bool,
    /// Whether screen capture is available (drives whether freeze/save are listed).
    capture_available: bool,
    /// Status-chip attention pulse (0 = none … 1 = peak).
    flash: f32,
    /// Index of the selected element (drawn with a handle outline), if any.
    selected: Option<usize>,
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /// Calloop handle, needed to wire up keyboard repeat when the seat appears.
    loop_handle: LoopHandle<'static, State>,
    surfaces: Vec<Surface>,
    /// A pre-made empty region: set as the input region for click-through.
    empty_region: Region,
    theme: Theme,
    /// Resolved keybindings (`~/.config/wlr-draw/keys.toml`, or the built-in defaults).
    keymap: Keymap,

    doc: Document,
    tool: Tool,
    color: Color,
    width: f32,
    /// Draw mode (input grabbed) vs click-through.
    draw_mode: bool,
    /// Pointer click-through active (the pass-through bind), in draw mode.
    passthrough: bool,
    /// Constrain role active (the constrain bind): square/circle/axis-locked shapes.
    constrain_active: bool,
    /// Spotlight role active (the spotlight bind): dim everything around the shape being
    /// drawn, or a flashlight hole around the cursor while idle.
    spotlight_active: bool,
    /// Physical Ctrl / Shift state, read only for the arrow-nudge step granularity
    /// (Shift = 1px, Ctrl = big) — kept on the real modifiers regardless of how the
    /// constrain / spotlight roles are rebound.
    ctrl_held: bool,
    shift_held: bool,
    /// Radius of the cursor flashlight hole (logical px), tuned with the wheel / i / k.
    spotlight_radius: f32,
    /// Veil opacity (0..255 alpha), tuned with the tilt wheel while Shift held.
    spotlight_dim: f32,
    /// Set when a spotlight shape was just placed: suppresses the cursor flashlight until
    /// Shift is released, so releasing the drag doesn't snap straight back to the torch.
    spotlight_latched: bool,
    /// Freeze-frame active: each surface shows a frozen screenshot backdrop.
    frozen: bool,
    /// Lazily-opened capture connection (separate from the overlay's), reused across
    /// freezes. Uses shm, so it never touches the overlay's EGL contexts.
    capture_client: Option<wl::Client>,
    /// Whether this compositor can capture the screen (ext-image-copy-capture + an
    /// output source, wlroots >= 0.19). Freeze and save need it; when `false` they are
    /// hidden from the help/tray and no-op with a clear message (issue #1).
    capture_available: bool,
    /// Whether annotations are shown (visibility toggle).
    visible: bool,
    /// On-screen key legend (the `h` shortcut).
    show_help: bool,
    /// Colour-picker popup (the `c` shortcut).
    show_palette: bool,

    pointer_pos: Option<(f64, f64)>,
    gesture: Gesture,
    text_edit: Option<((f32, f32), String)>,
    /// The element to nudge with the arrow keys: the last one placed, until another
    /// action deselects it. `moving` coalesces a run of nudges into one undo step.
    selected: Option<usize>,
    moving: bool,
    /// Time of the last significant pen movement, for dwell-to-snap.
    pen_dwell: Instant,
    /// When the status-chip attention pulse started (draw mode entered on an empty
    /// canvas); `None` once it has run its course.
    flash_start: Option<Instant>,
    /// Repeated-click ("am I stuck?") detector: anchor, count and time of the cluster.
    click_anchor: (f32, f32),
    click_count: u32,
    click_last: Instant,

    /// Tray icon handle, for pushing status changes (the `tray` feature).
    #[cfg(feature = "tray")]
    tray: Option<ksni::blocking::Handle<crate::tray::DrawTray>>,

    dirty: bool,
    quit: bool,
}

impl State {
    fn redraw_all(&mut self, conn: &Connection) {
        let flash = self.flash_value();
        // Disjoint borrows: the frame view reads `doc`/`gesture`/… while we iterate
        // `surfaces` mutably.
        let frame = Frame {
            elements: self.doc.elements(),
            gesture: &self.gesture,
            text_edit: &self.text_edit,
            theme: &self.theme,
            keymap: &self.keymap,
            tool: self.tool,
            color: self.color,
            width: self.width,
            pointer: self.pointer_pos,
            draw_mode: self.draw_mode,
            passthrough: self.passthrough,
            constrain: self.constrain_active,
            spotlight: self.spotlight_active,
            spotlight_radius: self.spotlight_radius,
            spotlight_dim: self.spotlight_dim.round() as u8,
            flashlight: !self.spotlight_latched,
            visible: self.visible,
            show_help: self.show_help,
            show_palette: self.show_palette,
            capture_available: self.capture_available,
            flash,
            selected: self.selected,
        };
        for s in &mut self.surfaces {
            s.render(conn, &frame);
        }
    }

    /// Map a surface-local pointer position to global logical coordinates.
    fn to_global(&self, surface: &wl_surface::WlSurface, pos: (f64, f64)) -> Option<(f64, f64)> {
        self.surfaces
            .iter()
            .find(|s| s.layer.wl_surface() == surface)
            .map(|s| (s.logical_x as f64 + pos.0, s.logical_y as f64 + pos.1))
    }

    /// Enter or leave draw mode: switch every surface between a full input region +
    /// `Exclusive` keyboard (drawing) and an empty input region + no keyboard
    /// (click-through). Leaving commits any in-flight text and drops the gesture.
    ///
    /// Keyboard is `Exclusive` (not `OnDemand`) in draw mode so the overlay keeps
    /// keyboard focus even when the pointer momentarily passes through — otherwise a
    /// click on a window below would steal focus and we'd miss the modifier release
    /// that ends [`set_passthrough`], leaving pass-through stuck on.
    fn set_draw_mode(&mut self, on: bool) {
        if on == self.draw_mode {
            return;
        }
        if !on {
            self.commit_text();
            self.deselect();
            self.unfreeze();
            self.gesture = Gesture::None;
            self.flash_start = None;
        } else if self.doc.elements().is_empty() {
            // Entering draw mode on an empty canvas: pulse the chip to draw the eye.
            self.flash_start = Some(Instant::now());
        }
        self.draw_mode = on;
        self.passthrough = false;
        for s in &self.surfaces {
            let surf = s.layer.wl_surface();
            if on {
                surf.set_input_region(None); // whole surface receives input
                s.layer
                    .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            } else {
                surf.set_input_region(Some(self.empty_region.wl_region()));
                s.layer
                    .set_keyboard_interactivity(KeyboardInteractivity::None);
            }
            s.layer.commit();
        }
        self.dirty = true;
        self.sync_tray();
    }

    /// Momentary click-through while drawing: hold a modifier (Ctrl) to let the pointer
    /// reach the apps below without leaving draw mode — release it to go back to
    /// drawing. Only the *pointer* input region is toggled; keyboard stays `Exclusive`
    /// so we keep receiving the modifier release. A no-op outside draw mode.
    fn set_passthrough(&mut self, on: bool) {
        if !self.draw_mode || on == self.passthrough {
            return;
        }
        self.passthrough = on;
        if on {
            self.cancel_gesture(); // don't resume a stroke across the gap
            self.deselect();
        }
        for s in &self.surfaces {
            let surf = s.layer.wl_surface();
            if on {
                surf.set_input_region(Some(self.empty_region.wl_region()));
            } else {
                surf.set_input_region(None);
            }
            s.layer.commit();
        }
        self.dirty = true;
    }

    /// Toggle the constrain role (square/circle/axis-locked shapes). Driven by the
    /// configured trigger — a held modifier, or a held key.
    fn set_constrain(&mut self, on: bool) {
        if on == self.constrain_active {
            return;
        }
        self.constrain_active = on;
        if !matches!(self.gesture, Gesture::None) {
            self.dirty = true; // refresh the constrained preview in place
        }
    }

    /// Toggle the spotlight role (cursor flashlight / dim around a shape). Driven by the
    /// configured trigger. Leaving it re-arms the idle torch, like releasing Shift did.
    fn set_spotlight(&mut self, on: bool) {
        if on == self.spotlight_active {
            return;
        }
        self.spotlight_active = on;
        if !on {
            self.spotlight_latched = false; // a fresh activation re-arms the torch
        }
        self.dirty = true;
    }

    /// Toggle the freeze-frame backdrop: capture every output once and show it frozen so
    /// the user can annotate a still image; toggling again (or Esc) returns to live.
    fn toggle_freeze(&mut self) {
        if !self.capture_available {
            eprintln!("wlr-draw: {}", tr!("draw-capture-unavailable"));
            return;
        }
        if self.frozen {
            self.unfreeze();
        } else if let Err(e) = self.freeze() {
            eprintln!("wlr-draw: freeze failed: {e}");
        }
    }

    /// Drop the frozen backdrop (back to the transparent live overlay).
    fn unfreeze(&mut self) {
        if !self.frozen {
            return;
        }
        self.frozen = false;
        for s in &mut self.surfaces {
            s.frozen_tex = None;
        }
        self.dirty = true;
    }

    /// Capture every output and upload each as its surface's frozen backdrop texture.
    fn freeze(&mut self) -> anyhow::Result<()> {
        if self.capture_client.is_none() {
            self.capture_client = Some(wl::Client::connect()?);
        }
        let client = self.capture_client.as_mut().unwrap();
        let caps = capture::capture_all(client, FREEZE_BUDGET)?;
        for s in &mut self.surfaces {
            // Match each capture to its surface by output position.
            let Some(cap) = caps
                .iter()
                .find(|c| c.output.logical_x == s.logical_x && c.output.logical_y == s.logical_y)
            else {
                continue;
            };
            let img = &cap.image;
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [img.width as usize, img.height as usize],
                &img.rgba,
            );
            s.frozen_tex = Some(s.egui_ctx.load_texture(
                "frozen",
                color,
                egui::TextureOptions::default(),
            ));
        }
        self.frozen = true;
        // Grab input so clicks annotate the still frame rather than passing through.
        self.set_draw_mode(true);
        self.dirty = true;
        Ok(())
    }

    /// Save the annotated screen to a PNG — the output under the cursor (or the whole
    /// layout). The capture grabs the composited output, so the overlay's strokes are
    /// baked into the image.
    fn save_screenshot(&mut self, path: Option<String>) {
        if !self.capture_available {
            eprintln!("wlr-draw: {}", tr!("draw-capture-unavailable"));
            return;
        }
        match self.do_save(path) {
            Ok(p) => eprintln!("wlr-draw: saved {}", p.display()),
            Err(e) => eprintln!("wlr-draw: save failed: {e}"),
        }
    }

    fn do_save(&mut self, path: Option<String>) -> anyhow::Result<std::path::PathBuf> {
        if self.capture_client.is_none() {
            self.capture_client = Some(wl::Client::connect()?);
        }
        let pointer = self.pointer_pos;
        let client = self.capture_client.as_mut().unwrap();
        let caps = capture::capture_all(client, FREEZE_BUDGET)?;
        let under_cursor = pointer.and_then(|(px, py)| {
            caps.iter().find(|c| {
                let o = &c.output;
                let (w, h) = o.logical_size();
                (px as i32) >= o.logical_x
                    && (px as i32) < o.logical_x + w
                    && (py as i32) >= o.logical_y
                    && (py as i32) < o.logical_y + h
            })
        });
        // The output under the cursor, else the whole-layout composite.
        let bytes = match under_cursor {
            Some(c) => capture::encode_png(&c.image)?,
            None => {
                let whole = capture::composite(&caps, capture::whole_layout(client)?)?;
                capture::encode_png(&whole)?
            }
        };
        let path = path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(default_save_path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Note a press for the "stuck user" detector: several clicks in quick succession
    /// near the same spot re-pulse the chip to remind them they're in draw mode.
    fn note_click(&mut self, g: (f32, f32)) {
        let near =
            (g.0 - self.click_anchor.0).hypot(g.1 - self.click_anchor.1) <= CLICK_CLUSTER_RADIUS;
        if near && self.click_last.elapsed() <= CLICK_CLUSTER_GAP {
            self.click_count += 1;
        } else {
            self.click_anchor = g;
            self.click_count = 1;
        }
        self.click_last = Instant::now();
        if self.click_count >= CLICK_CLUSTER_N {
            self.flash_start = Some(Instant::now());
            self.click_count = 0;
        }
    }

    /// Current attention-pulse intensity (0 = none): a few decaying blinks over
    /// [`FLASH_DURATION`]. Clears the timer once it has elapsed.
    fn flash_value(&mut self) -> f32 {
        let Some(start) = self.flash_start else {
            return 0.0;
        };
        let e = start.elapsed().as_secs_f32() / FLASH_DURATION.as_secs_f32();
        if e >= 1.0 {
            self.flash_start = None;
            return 0.0;
        }
        // `FLASH_CYCLES` on/off humps, fading out towards the end.
        (e * FLASH_CYCLES * std::f32::consts::PI).sin().abs() * (1.0 - e)
    }

    /// Push the current draw-mode / colour / tool to the tray icon (no-op without it).
    fn sync_tray(&self) {
        #[cfg(feature = "tray")]
        if let Some(h) = &self.tray {
            let (active, color, tool) = (self.draw_mode, self.color, self.tool);
            h.update(move |t| {
                t.active = active;
                t.color = color;
                t.tool = tool;
            });
        }
    }

    /// Grow/shrink the cursor flashlight, clamped to a sensible range.
    fn adjust_spotlight_radius(&mut self, delta: f32) {
        self.spotlight_radius =
            (self.spotlight_radius + delta).clamp(SPOTLIGHT_RADIUS_MIN, SPOTLIGHT_RADIUS_MAX);
        self.dirty = true;
    }

    /// Darken/lighten the spotlight veil, clamped to a sensible range.
    fn adjust_spotlight_dim(&mut self, delta: f32) {
        self.spotlight_dim =
            (self.spotlight_dim + delta).clamp(SPOTLIGHT_DIM_MIN, SPOTLIGHT_DIM_MAX);
        self.dirty = true;
    }

    /// The topmost element under `g` (last drawn wins), for the Move tool's click-select.
    fn element_at(&self, g: (f32, f32)) -> Option<usize> {
        self.doc
            .elements()
            .iter()
            .enumerate()
            .rev()
            .find(|(_, el)| el.hit(g, SELECT_RADIUS))
            .map(|(i, _)| i)
    }

    /// Finalize a run of arrow-key nudges into a single undo step.
    fn end_move_session(&mut self) {
        if self.moving {
            self.doc.end_gesture();
            self.moving = false;
        }
    }

    /// Clear the selection (and finalize any move in progress).
    fn deselect(&mut self) {
        self.end_move_session();
        if self.selected.take().is_some() {
            self.dirty = true;
        }
    }

    /// Nudge the selected element by `(dx, dy)`; the first nudge of a run snapshots once.
    fn nudge_selected(&mut self, dx: f32, dy: f32) {
        let Some(idx) = self.selected else {
            return;
        };
        if !self.moving {
            self.doc.begin_move();
            self.moving = true;
        }
        if self.doc.translate(idx, dx, dy) {
            self.dirty = true;
        }
    }

    /// Drop the in-flight gesture without committing it (e.g. on entering pass-through).
    fn cancel_gesture(&mut self) {
        if matches!(self.gesture, Gesture::Erase) {
            self.doc.end_gesture();
        }
        self.gesture = Gesture::None;
    }

    /// Apply a control command from the socket.
    fn apply_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Toggle => self.set_draw_mode(!self.draw_mode),
            Cmd::On => self.set_draw_mode(true),
            Cmd::Off => self.set_draw_mode(false),
            Cmd::Clear => {
                self.deselect();
                self.text_edit = None;
                self.gesture = Gesture::None;
                self.doc.clear();
                self.dirty = true;
            }
            Cmd::Undo => {
                // The selection index would no longer be meaningful after a history jump.
                self.deselect();
                if self.doc.undo() {
                    self.dirty = true;
                }
            }
            Cmd::Redo => {
                self.deselect();
                if self.doc.redo() {
                    self.dirty = true;
                }
            }
            Cmd::Visibility => {
                self.visible = !self.visible;
                self.dirty = true;
            }
            Cmd::Tool(t) => {
                if t != Tool::Text {
                    self.commit_text();
                }
                // Selection belongs to the Move tool; leaving it drops the selection.
                if t != Tool::Move {
                    self.deselect();
                }
                self.tool = t;
                self.dirty = true;
                self.sync_tray();
            }
            Cmd::Color(c) => {
                self.color = c;
                self.dirty = true;
                self.sync_tray();
            }
            Cmd::Width(w) => {
                self.width = w.clamp(1.0, 200.0);
                self.dirty = true;
            }
            Cmd::Save(path) => self.save_screenshot(path),
            Cmd::Quit => self.quit = true,
        }
    }

    /// Commit the text being typed (if any and non-empty) as a [`Element::Text`].
    fn commit_text(&mut self) {
        if let Some((pos, text)) = self.text_edit.take() {
            let text = text.trim_end().to_string();
            if !text.is_empty() {
                self.doc.commit(Element::Text {
                    pos,
                    text,
                    color: self.color,
                    size: text_size(self.width),
                });
            }
            self.dirty = true;
        }
    }

    /// Left-button press at global logical `g`: start the gesture for the current tool.
    fn on_press(&mut self, g: (f32, f32)) {
        self.deselect();
        self.note_click(g);
        match self.tool {
            Tool::Pen => {
                self.gesture = Gesture::Pen(vec![g]);
                self.pen_dwell = Instant::now();
            }
            Tool::Rect | Tool::Mask | Tool::Arrow => {
                let kind = self.tool.shape_kind().expect("shape tool");
                self.gesture = Gesture::Shape(kind, g);
            }
            Tool::Eraser => {
                self.doc.begin_erase();
                self.doc.erase_at(g, ERASE_RADIUS);
                self.gesture = Gesture::Erase;
            }
            Tool::Move => {
                // Grab the topmost element under the cursor; clicking empty space just
                // leaves nothing selected (deselect already ran at the top).
                if let Some(idx) = self.element_at(g) {
                    self.selected = Some(idx);
                    self.doc.begin_move();
                    self.moving = true;
                    self.gesture = Gesture::Move {
                        start: g,
                        applied: (0.0, 0.0),
                    };
                }
            }
            Tool::Text => {
                // A second click while typing commits the first label, then starts a new
                // one at the new spot.
                self.commit_text();
                self.text_edit = Some((g, String::new()));
            }
        }
        self.dirty = true;
    }

    /// Pointer motion at global logical `g`: extend the active gesture.
    fn on_motion(&mut self, g: (f32, f32)) {
        match &mut self.gesture {
            Gesture::Pen(points) => {
                // Reset the dwell timer only on real movement, so micro-jitter while
                // holding still still triggers the snap.
                let moved = points
                    .last()
                    .is_none_or(|&p| (p.0 - g.0).hypot(p.1 - g.1) > PEN_STILL_EPS);
                points.push(g);
                if moved {
                    self.pen_dwell = Instant::now();
                }
            }
            Gesture::Erase => {
                self.doc.erase_at(g, ERASE_RADIUS);
            }
            Gesture::Move { start, applied } => {
                if let Some(idx) = self.selected {
                    let raw = (g.0 - start.0, g.1 - start.1);
                    // Constrain locks the move to the dominant axis (horizontal or vertical).
                    let target = if self.constrain_active {
                        if raw.0.abs() >= raw.1.abs() {
                            (raw.0, 0.0)
                        } else {
                            (0.0, raw.1)
                        }
                    } else {
                        raw
                    };
                    self.doc
                        .translate(idx, target.0 - applied.0, target.1 - applied.1);
                    *applied = target;
                }
            }
            // Shapes (manual and snapped) track the live pointer; nothing to store.
            Gesture::Shape(..) | Gesture::SnapEllipse { .. } | Gesture::None => {}
        }
        self.dirty = true;
    }

    /// Called each loop tick while a pen stroke is in flight: if the pen has held still
    /// long enough and the freehand path reads as a clean shape, snap to it. The snapped
    /// shape then resizes live until the button is released.
    fn check_dwell(&mut self) {
        let recognized = match &self.gesture {
            Gesture::Pen(points) if self.pen_dwell.elapsed() >= DWELL => recognize(points),
            _ => None,
        };
        if let Some(r) = recognized {
            self.gesture = match r {
                Recognized::Ellipse { center, circle } => Gesture::SnapEllipse { center, circle },
                Recognized::Line { anchor } => Gesture::Shape(ShapeKind::Line, anchor),
            };
            self.dirty = true;
        }
    }

    /// Left-button release at global logical `g`: finish the gesture.
    fn on_release(&mut self, g: (f32, f32)) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Pen(mut points) => {
                if points.len() < 2 {
                    // A click without a drag: a dot (two coincident points draw a cap).
                    points.push(points.first().copied().unwrap_or(g));
                }
                self.doc.commit(Element::Stroke {
                    points,
                    color: self.color,
                    width: self.width,
                });
            }
            Gesture::Shape(kind, start) => {
                let b = if self.constrain_active {
                    constrain(kind, start, g)
                } else {
                    g
                };
                // The spotlight role turns a closed shape into its spotlight (dim around it).
                let kind = match self.spotlight_active.then(|| kind.spotlight()).flatten() {
                    Some(spot) => spot,
                    None => kind,
                };
                if (start.0 - b.0).abs() > 1.0 || (start.1 - b.1).abs() > 1.0 {
                    self.doc.commit(Element::Shape {
                        kind,
                        a: start,
                        b,
                        color: self.color,
                        width: self.width,
                    });
                    self.spotlight_latched |= kind.is_spotlight();
                }
            }
            Gesture::SnapEllipse { center, circle } => {
                let (rx, ry) = snap_radii(center, g, circle || self.constrain_active);
                let kind = if self.spotlight_active {
                    ShapeKind::SpotlightEllipse
                } else {
                    ShapeKind::Ellipse
                };
                self.doc.commit(Element::Shape {
                    kind,
                    a: (center.0 - rx, center.1 - ry),
                    b: (center.0 + rx, center.1 + ry),
                    color: self.color,
                    width: self.width,
                });
                self.spotlight_latched |= kind.is_spotlight();
            }
            Gesture::Erase => self.doc.end_gesture(),
            Gesture::Move { .. } => self.end_move_session(),
            Gesture::None => {}
        }
        self.dirty = true;
    }

    /// Right-button press: grab the topmost element under the cursor and move it with the
    /// drag, whatever the current tool. A transient grab — the release drops the
    /// selection, so no outline lingers afterwards.
    fn on_right_press(&mut self, g: (f32, f32)) {
        self.deselect();
        if let Some(idx) = self.element_at(g) {
            self.selected = Some(idx);
            self.doc.begin_move();
            self.moving = true;
            self.gesture = Gesture::Move {
                start: g,
                applied: (0.0, 0.0),
            };
        }
        self.dirty = true;
    }

    /// Right-button release: finish the move and drop the transient selection.
    fn on_right_release(&mut self) {
        if matches!(self.gesture, Gesture::Move { .. }) {
            self.gesture = Gesture::None;
        }
        self.deselect();
        self.dirty = true;
    }

    /// A keystroke while drawing. When a text label is being typed, keys feed the
    /// buffer; otherwise the overlay holds keyboard focus, so bare keys are local
    /// shortcuts (they don't clash with the compositor's `$mod+…` bindings). Control
    /// also arrives over the socket, so these are a convenience, not the only way.
    fn on_key(&mut self, event: &KeyEvent) {
        if self.text_edit.is_some() {
            match event.keysym {
                Keysym::Escape => {
                    self.text_edit = None; // discard the in-progress label
                    self.dirty = true;
                }
                Keysym::Return | Keysym::KP_Enter => self.commit_text(),
                Keysym::BackSpace => {
                    if let Some((_, buf)) = self.text_edit.as_mut() {
                        buf.pop();
                        self.dirty = true;
                    }
                }
                _ => {
                    if let Some(txt) = &event.utf8 {
                        // Printable input only (skip control characters).
                        if txt.chars().any(|c| !c.is_control())
                            && let Some((_, buf)) = self.text_edit.as_mut()
                        {
                            buf.push_str(txt);
                            self.dirty = true;
                        }
                    }
                }
            }
            return;
        }
        // Any key other than an arrow ends a nudge run, so undo groups the whole move.
        let is_arrow = matches!(
            event.keysym,
            Keysym::Left | Keysym::Right | Keysym::Up | Keysym::Down
        );
        if !is_arrow {
            self.end_move_session();
        }
        // --- Reserved controls (not rebindable) ---
        // Arrow keys nudge the selected element. Shift = 1px (pixel-precise), Ctrl = big
        // step, plain = medium — read from the *physical* modifiers, independent of how
        // the constrain / spotlight roles are bound.
        if is_arrow {
            if self.selected.is_some() {
                let step = if self.shift_held {
                    1.0
                } else if self.ctrl_held {
                    25.0
                } else {
                    6.0
                };
                let (dx, dy) = match event.keysym {
                    Keysym::Left => (-step, 0.0),
                    Keysym::Right => (step, 0.0),
                    Keysym::Up => (0.0, -step),
                    _ => (0.0, step),
                };
                self.nudge_selected(dx, dy);
            }
            return;
        }
        // Esc peels back one layer at a time (popup → frozen → draw mode).
        if event.keysym == Keysym::Escape {
            if self.show_palette {
                self.show_palette = false;
                self.dirty = true;
            } else if self.show_help {
                self.show_help = false;
                self.dirty = true;
            } else if self.frozen {
                self.unfreeze();
            } else {
                self.set_draw_mode(false);
            }
            return;
        }
        // Spotlight tuning cluster (i/j/k/l), live only while the spotlight role is active:
        // an ijkl cluster, layout-proof unlike +/-. The wheel does the same.
        if self.spotlight_active {
            match event.keysym {
                Keysym::i | Keysym::I => {
                    self.adjust_spotlight_radius(SPOTLIGHT_RADIUS_STEP);
                    return;
                }
                Keysym::k | Keysym::K => {
                    self.adjust_spotlight_radius(-SPOTLIGHT_RADIUS_STEP);
                    return;
                }
                Keysym::l | Keysym::L => {
                    self.adjust_spotlight_dim(-SPOTLIGHT_DIM_STEP); // lighter
                    return;
                }
                Keysym::j | Keysym::J => {
                    self.adjust_spotlight_dim(SPOTLIGHT_DIM_STEP); // darker
                    return;
                }
                _ => {}
            }
        }

        // --- Configurable bindings ---
        let ks = event.keysym;
        // Held roles bound to a key: pass-through toggles (parity with the Caps latch),
        // constrain / spotlight engage while held (released in `on_key_release`).
        if self.keymap.passthrough == Trigger::Key(ks) {
            self.set_passthrough(!self.passthrough);
            return;
        }
        if self.keymap.constrain == Trigger::Key(ks) {
            self.set_constrain(true);
            return;
        }
        if self.keymap.spotlight == Trigger::Key(ks) {
            self.set_spotlight(true);
            return;
        }
        // Discrete actions.
        if let Some(action) = self.keymap.action_for_key(ks) {
            self.do_action(action);
        }
    }

    /// Release handler for held roles bound to a regular key. A no-op for everything else
    /// (discrete actions only fire on press); pass-through is a toggle, so it ignores
    /// release too.
    fn on_key_release(&mut self, ks: Keysym) {
        if self.keymap.constrain == Trigger::Key(ks) {
            self.set_constrain(false);
        }
        if self.keymap.spotlight == Trigger::Key(ks) {
            self.set_spotlight(false);
        }
    }

    /// Dispatch a discrete keymap action.
    fn do_action(&mut self, action: Action) {
        match action {
            Action::Pen => self.apply_cmd(Cmd::Tool(Tool::Pen)),
            Action::Rect => self.apply_cmd(Cmd::Tool(Tool::Rect)),
            Action::Mask => self.apply_cmd(Cmd::Tool(Tool::Mask)),
            Action::Arrow => self.apply_cmd(Cmd::Tool(Tool::Arrow)),
            Action::Text => self.apply_cmd(Cmd::Tool(Tool::Text)),
            Action::Move => self.apply_cmd(Cmd::Tool(Tool::Move)),
            Action::Eraser => self.apply_cmd(Cmd::Tool(Tool::Eraser)),
            Action::Palette => {
                self.show_palette = !self.show_palette;
                self.dirty = true;
            }
            Action::Undo => self.apply_cmd(Cmd::Undo),
            Action::Redo => self.apply_cmd(Cmd::Redo),
            Action::Visibility => self.apply_cmd(Cmd::Visibility),
            Action::Save => self.apply_cmd(Cmd::Save(None)),
            Action::Help => {
                self.show_help = !self.show_help;
                self.dirty = true;
            }
            Action::Clear => self.apply_cmd(Cmd::Clear),
            Action::WidthInc => self.apply_cmd(Cmd::Width(self.width + 2.0)),
            Action::WidthDec => self.apply_cmd(Cmd::Width((self.width - 2.0).max(1.0))),
            Action::Freeze => self.toggle_freeze(),
        }
    }

    /// Surface-local size of the surface backing `surface`, for popup hit-testing.
    fn surface_dims(&self, surface: &wl_surface::WlSurface) -> Option<(f32, f32)> {
        self.surfaces
            .iter()
            .find(|s| s.layer.wl_surface() == surface)
            .map(|s| (s.width as f32, s.height as f32))
    }

    /// Handle a left click while the colour palette is open: pick the swatch under the
    /// cursor (closing the popup), or close it on a click outside the panel.
    fn palette_click(&mut self, surface: &wl_surface::WlSurface, pos: (f64, f64)) {
        let p = egui::pos2(pos.0 as f32, pos.1 as f32);
        if let Some((w, h)) = self.surface_dims(surface) {
            let (panel, cells) = palette_cells(w, h);
            if let Some((_, c)) = cells.iter().find(|(r, _)| r.contains(p)) {
                self.color = *c;
                self.show_palette = false;
            } else if !panel.contains(p) {
                self.show_palette = false; // click outside dismisses
            }
        } else {
            self.show_palette = false;
        }
        self.dirty = true;
        self.sync_tray();
    }
}

/// Run the annotation daemon until `quit`.
pub fn run() -> anyhow::Result<()> {
    // A clean no-op exit when one is already running, so a service manager
    // (systemd `--user`) doesn't treat it as a failure and restart-loop.
    if ipc::daemon_running() {
        eprintln!("wlr-draw: a daemon is already running");
        return Ok(());
    }
    let listener = ipc::bind()?;

    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|e| anyhow::anyhow!("wl_compositor: {e}"))?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|e| anyhow::anyhow!("layer-shell missing: {e}"))?;
    let empty_region = Region::new(&compositor).map_err(|e| anyhow::anyhow!("wl_region: {e}"))?;

    // The calloop loop is created up front so its handle can be handed to the keyboard
    // (for repeat) when the seat capability appears during the roundtrips below.
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let lh = event_loop.handle();

    let theme = Theme::load();
    let keymap = Keymap::load();
    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        keyboard: None,
        pointer: None,
        loop_handle: lh.clone(),
        surfaces: Vec::new(),
        empty_region,
        theme,
        keymap,
        doc: Document::new(),
        tool: Tool::Pen,
        color: DEFAULT_COLOR,
        width: DEFAULT_WIDTH,
        draw_mode: false,
        passthrough: false,
        constrain_active: false,
        spotlight_active: false,
        ctrl_held: false,
        shift_held: false,
        spotlight_radius: DEFAULT_SPOTLIGHT_RADIUS,
        spotlight_dim: DEFAULT_SPOTLIGHT_DIM,
        spotlight_latched: false,
        frozen: false,
        capture_client: None,
        // Probe once whether the screen can be captured (ext-image-copy-capture + an
        // output source). Freeze/save need it; on compositors without it we hide those
        // from the help/tray rather than failing at first use (issue #1). Drawing and
        // click-through need no capture and always work.
        capture_available: wl::Client::connect()
            .map(|c| !c.outputs().is_empty())
            .unwrap_or(false),
        visible: true,
        show_help: false,
        show_palette: false,
        pointer_pos: None,
        gesture: Gesture::None,
        text_edit: None,
        selected: None,
        moving: false,
        pen_dwell: Instant::now(),
        flash_start: None,
        click_anchor: (f32::MIN, f32::MIN),
        click_count: 0,
        click_last: Instant::now(),
        #[cfg(feature = "tray")]
        tray: None,
        dirty: false,
        quit: false,
    };

    // Enumerate outputs, then build one click-through overlay surface per output.
    event_queue.roundtrip(&mut state)?;
    let outputs: Vec<_> = state.output_state.outputs().collect();
    for wl_out in outputs {
        let Some(info) = state.output_state.info(&wl_out) else {
            continue;
        };
        let (lx, ly) = info.logical_position.unwrap_or((0, 0));
        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("wlr-draw"),
            Some(&wl_out),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        // Start click-through: an empty input region lets everything pass to the apps
        // below until the user toggles draw mode.
        layer
            .wl_surface()
            .set_input_region(Some(state.empty_region.wl_region()));
        layer.commit();

        state.surfaces.push(Surface {
            layer,
            logical_x: lx,
            logical_y: ly,
            egui_ctx: egui::Context::default(),
            gpu: None,
            width: 0,
            height: 0,
            scale: 1,
            frozen_tex: None,
        });
    }
    if state.surfaces.is_empty() {
        anyhow::bail!("no outputs to draw on");
    }
    event_queue.roundtrip(&mut state)?;

    // Drive Wayland events and socket commands from the calloop loop created above.
    WaylandSource::new(conn.clone(), event_queue)
        .insert(lh.clone())
        .map_err(|e| anyhow::anyhow!("calloop wayland source: {e}"))?;

    let (tx, ch): (_, Channel<Cmd>) = channel();
    // The tray sends menu actions over the same channel as the socket, and reflects the
    // daemon's status (icon + colour) back through its handle.
    #[cfg(feature = "tray")]
    {
        ipc::serve(listener, tx.clone());
        let shortcuts = shortcut_rows(&state.keymap, state.capture_available);
        state.tray = crate::tray::spawn(tx, state.color, state.tool, shortcuts);
    }
    #[cfg(not(feature = "tray"))]
    ipc::serve(listener, tx);
    lh.insert_source(ch, |event, _, state: &mut State| {
        if let ChannelEvent::Msg(cmd) = event {
            state.apply_cmd(cmd);
        }
    })
    .map_err(|e| anyhow::anyhow!("calloop channel source: {e}"))?;

    // A startup line so `journalctl --user -t wlr-draw` shows the daemon came up (and its
    // capture verdict) — otherwise the log stays silent until the first error.
    eprintln!(
        "wlr-draw: daemon ready on {} output{} (screen capture {})",
        state.surfaces.len(),
        if state.surfaces.len() == 1 { "" } else { "s" },
        if state.capture_available {
            "available"
        } else {
            "unavailable"
        },
    );

    while !state.quit {
        // Tick fast enough to animate: while a freehand stroke is in flight (dwell to
        // snap) or the status chip is pulsing, even when no input events arrive.
        let animating = matches!(state.gesture, Gesture::Pen(_)) || state.flash_start.is_some();
        let timeout = if animating {
            Duration::from_millis(60)
        } else {
            Duration::from_millis(1000)
        };
        event_loop.dispatch(timeout, &mut state)?;
        state.check_dwell();
        if state.flash_start.is_some() {
            state.dirty = true; // keep redrawing through the pulse
        }
        if state.dirty {
            state.dirty = false;
            state.redraw_all(&conn);
        }
    }

    ipc::cleanup();
    Ok(())
}

// ---------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------

fn col(c: Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Paint one surface: every element, the in-progress gesture and the live text, all
/// offset by `(lx, ly)` into local coordinates, plus the draw-mode HUD.
fn paint(
    ui: &mut egui::Ui,
    lx: f32,
    ly: f32,
    frame: &Frame,
    frozen_tex: Option<&egui::TextureHandle>,
) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let p = ui.painter();
            let off = egui::vec2(-lx, -ly);

            // Freeze-frame: an opaque screenshot backdrop under everything, so the live
            // screen is held still while the annotations (and spotlight veil) sit on top.
            if let Some(tex) = frozen_tex {
                p.image(
                    tex.id(),
                    ui.max_rect(),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            if frame.visible {
                // Spotlight veil: dim the whole surface except the bright "holes" — placed
                // spotlight elements, plus the live one (a shape being dragged, or the
                // cursor flashlight) while Shift is held. One veil for all holes, so two
                // lit zones never darken each other.
                let mut holes: Vec<Hole> = frame
                    .elements
                    .iter()
                    .filter_map(|el| spotlight_hole(el, off))
                    .collect();
                holes.extend(live_spotlight_hole(frame, off));
                if !holes.is_empty() {
                    paint_veil(p, ui.max_rect(), &holes, frame.spotlight_dim);
                }
                // Annotations on top of the veil; spotlight shapes are the veil, not drawn.
                for el in frame.elements {
                    if !is_spotlight_element(el) {
                        paint_element(p, off, el);
                    }
                }
                paint_gesture(p, off, frame);
                paint_text_edit(p, off, frame);
                // Selection only exists in the Move tool or during a right-drag, so this
                // no-ops the rest of the time (it returns early when nothing is selected).
                paint_selection(p, off, frame);
            }

            if frame.draw_mode {
                paint_hud(p, ui, frame);
                if frame.show_help {
                    paint_help(p, ui, frame);
                }
                if frame.show_palette {
                    paint_palette(p, ui, frame);
                }
            }
        });
}

/// Paint a finished element.
fn paint_element(p: &egui::Painter, off: egui::Vec2, el: &Element) {
    match el {
        Element::Stroke {
            points,
            color,
            width,
        } => {
            let pts: Vec<egui::Pos2> = points.iter().map(|&(x, y)| local(x, y, off)).collect();
            p.add(egui::Shape::line(
                pts,
                egui::Stroke::new(*width, col(*color)),
            ));
        }
        Element::Shape {
            kind,
            a,
            b,
            color,
            width,
        } => paint_shape(p, off, *kind, *a, *b, col(*color), *width),
        Element::Text {
            pos,
            text,
            color,
            size,
        } => {
            p.text(
                local(pos.0, pos.1, off),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(*size),
                col(*color),
            );
        }
    }
}

fn local(x: f32, y: f32, off: egui::Vec2) -> egui::Pos2 {
    egui::pos2(x + off.x, y + off.y)
}

fn paint_shape(
    p: &egui::Painter,
    off: egui::Vec2,
    kind: ShapeKind,
    a: (f32, f32),
    b: (f32, f32),
    color: egui::Color32,
    width: f32,
) {
    let pa = local(a.0, a.1, off);
    let pb = local(b.0, b.1, off);
    let stroke = egui::Stroke::new(width, color);
    match kind {
        ShapeKind::Line => {
            p.line_segment([pa, pb], stroke);
        }
        ShapeKind::Arrow => draw_arrow(p, pa, pb, color, width),
        ShapeKind::Rect => {
            p.rect_stroke(
                egui::Rect::from_two_pos(pa, pb),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        ShapeKind::FilledRect => {
            p.rect_filled(egui::Rect::from_two_pos(pa, pb), 0.0, color);
        }
        ShapeKind::Ellipse => {
            let r = egui::Rect::from_two_pos(pa, pb);
            p.add(egui::Shape::Ellipse(egui::epaint::EllipseShape::stroke(
                r.center(),
                r.size() * 0.5,
                stroke,
            )));
        }
        // Spotlights are rendered as the veil (see `paint_veil`), never as an outline.
        ShapeKind::SpotlightRect | ShapeKind::SpotlightEllipse => {}
    }
}

/// Arrowhead length for a shaft of `len` px drawn at stroke `width`: tied to the stroke
/// width (so heads stay consistent), at least 10px for readability, but never more than
/// 60% of the shaft so it can't dwarf a short arrow. On a very short arrow the 60% cap
/// wins over the 10px floor — hence `cap.min(10.0)` as the lower bound, so the clamp never
/// gets `min > max` (which panics).
fn arrowhead_len(width: f32, len: f32) -> f32 {
    let cap = 0.6 * len;
    (width * 4.5).clamp(cap.min(10.0), cap)
}

/// Draw an arrow from `pa` to `pb`: a shaft plus an arrowhead whose size scales with the
/// stroke `width` (not the arrow's length), so short and long arrows keep a consistent
/// head.
fn draw_arrow(p: &egui::Painter, pa: egui::Pos2, pb: egui::Pos2, color: egui::Color32, width: f32) {
    let stroke = egui::Stroke::new(width, color);
    p.line_segment([pa, pb], stroke);
    let v = pb - pa;
    let len = v.length();
    if len < 1.0 {
        return;
    }
    let dir = v / len;
    let head = arrowhead_len(width, len);
    let spread = 0.45_f32; // ~26° half-angle
    let (sin, cos) = spread.sin_cos();
    // Two barbs: -dir rotated by ±spread.
    let b1 = egui::vec2(-dir.x * cos + dir.y * sin, -dir.x * sin - dir.y * cos);
    let b2 = egui::vec2(-dir.x * cos - dir.y * sin, dir.x * sin - dir.y * cos);
    p.line_segment([pb, pb + b1 * head], stroke);
    p.line_segment([pb, pb + b2 * head], stroke);
}

/// Paint the gesture currently under the cursor (pen points so far, or the shape
/// rubber-banding to the live pointer).
fn paint_gesture(p: &egui::Painter, off: egui::Vec2, frame: &Frame) {
    // A shape drawn with Shift held is previewed as the veil hole, not as an outline.
    if gesture_is_live_spotlight(frame) {
        return;
    }
    let color = col(frame.color);
    match frame.gesture {
        Gesture::Pen(points) if !points.is_empty() => {
            let mut pts: Vec<egui::Pos2> = points.iter().map(|&(x, y)| local(x, y, off)).collect();
            if pts.len() == 1 {
                pts.push(pts[0]);
            }
            p.add(egui::Shape::line(
                pts,
                egui::Stroke::new(frame.width, color),
            ));
        }
        Gesture::Shape(kind, start) => {
            if let Some((gx, gy)) = frame.pointer {
                let b = if frame.constrain {
                    constrain(*kind, *start, (gx as f32, gy as f32))
                } else {
                    (gx as f32, gy as f32)
                };
                paint_shape(p, off, *kind, *start, b, color, frame.width);
            }
        }
        Gesture::SnapEllipse { center, circle } => {
            if let Some((gx, gy)) = frame.pointer {
                let (rx, ry) =
                    snap_radii(*center, (gx as f32, gy as f32), *circle || frame.constrain);
                paint_shape(
                    p,
                    off,
                    ShapeKind::Ellipse,
                    (center.0 - rx, center.1 - ry),
                    (center.0 + rx, center.1 + ry),
                    color,
                    frame.width,
                );
            }
        }
        _ => {}
    }
}

/// Half-extents of a snapped ellipse whose centre is `center` and whose far point is the
/// cursor `g`. A circle uses the centre→cursor distance for both radii; an ellipse uses
/// the per-axis offset. A small floor keeps it visible.
fn snap_radii(center: (f32, f32), g: (f32, f32), circle: bool) -> (f32, f32) {
    let dx = (g.0 - center.0).abs();
    let dy = (g.1 - center.1).abs();
    if circle {
        let r = (dx * dx + dy * dy).sqrt().max(2.0);
        (r, r)
    } else {
        (dx.max(2.0), dy.max(2.0))
    }
}

// ---------------------------------------------------------------------------------
// Spotlight veil
// ---------------------------------------------------------------------------------

/// A bright region punched out of the spotlight veil, in surface-local coordinates.
enum Hole {
    Rect(egui::Rect),
    Ellipse {
        center: egui::Pos2,
        rx: f32,
        ry: f32,
    },
}

impl Hole {
    /// The x-interval this hole covers at scanline height `y`, or `None` if it doesn't
    /// reach that line.
    fn x_span_at(&self, y: f32) -> Option<(f32, f32)> {
        match self {
            Hole::Rect(r) => (y >= r.top() && y <= r.bottom()).then_some((r.left(), r.right())),
            Hole::Ellipse { center, rx, ry } => {
                if *rx <= f32::EPSILON || *ry <= f32::EPSILON {
                    return None;
                }
                let dy = (y - center.y) / ry;
                if dy.abs() > 1.0 {
                    return None;
                }
                let half = rx * (1.0 - dy * dy).sqrt();
                Some((center.x - half, center.x + half))
            }
        }
    }

    /// The hole's axis-aligned bounding box (for limiting the scanned band).
    fn bounds(&self) -> egui::Rect {
        match self {
            Hole::Rect(r) => *r,
            Hole::Ellipse { center, rx, ry } => {
                egui::Rect::from_center_size(*center, egui::vec2(rx * 2.0, ry * 2.0))
            }
        }
    }

    /// True if `p` is inside the hole, inset by `margin` (so a rim running exactly along a
    /// shared edge isn't counted as "inside the other hole").
    fn contains(&self, p: egui::Pos2, margin: f32) -> bool {
        match self {
            Hole::Rect(r) => r.shrink(margin).contains(p),
            Hole::Ellipse { center, rx, ry } => {
                let (ax, ay) = ((rx - margin).max(0.1), (ry - margin).max(0.1));
                let nx = (p.x - center.x) / ax;
                let ny = (p.y - center.y) / ay;
                nx * nx + ny * ny < 1.0
            }
        }
    }

    /// The hole's boundary as a closed polyline (for drawing the rim), sampled finely
    /// enough that segment midpoints are a fair test of "inside another hole".
    fn outline_points(&self) -> Vec<egui::Pos2> {
        match self {
            Hole::Rect(r) => {
                let c = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                ];
                let mut v = Vec::new();
                for i in 0..4 {
                    let (a, b) = (c[i], c[(i + 1) % 4]);
                    let n = ((b - a).length() / 8.0).ceil().max(1.0) as usize;
                    for k in 0..n {
                        v.push(a + (b - a) * (k as f32 / n as f32));
                    }
                }
                v.push(c[0]);
                v
            }
            Hole::Ellipse { center, rx, ry } => {
                let n = 72;
                (0..=n)
                    .map(|i| {
                        let a = i as f32 / n as f32 * std::f32::consts::TAU;
                        egui::pos2(center.x + rx * a.cos(), center.y + ry * a.sin())
                    })
                    .collect()
            }
        }
    }
}

/// A faint rim along the *union* boundary of all holes: each hole's outline is drawn only
/// where it isn't inside another hole, so overlapping spotlights merge into one bright
/// zone with no seam between them.
fn paint_union_rim(p: &egui::Painter, holes: &[Hole]) {
    let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(45));
    for (i, h) in holes.iter().enumerate() {
        let pts = h.outline_points();
        for seg in pts.windows(2) {
            let mid = seg[0] + (seg[1] - seg[0]) * 0.5;
            let buried = holes
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.contains(mid, 1.0));
            if !buried {
                p.line_segment([seg[0], seg[1]], stroke);
            }
        }
    }
}

/// True if Shift-spotlight is currently active: draw mode, pointer grabbed, not typing,
/// and a drawing tool is selected — so Shift stays free as a nudge modifier in the Move
/// tool (and for the eraser).
fn spotlight_active(frame: &Frame) -> bool {
    frame.draw_mode
        && frame.spotlight
        && !frame.passthrough
        && frame.text_edit.is_none()
        && matches!(
            frame.tool,
            Tool::Pen | Tool::Rect | Tool::Mask | Tool::Arrow
        )
}

/// True if the in-flight gesture is being previewed as a spotlight (so its normal outline
/// must be suppressed in favour of the veil hole).
fn gesture_is_live_spotlight(frame: &Frame) -> bool {
    spotlight_active(frame)
        && match frame.gesture {
            Gesture::SnapEllipse { .. } => true,
            Gesture::Shape(kind, _) => kind.spotlight().is_some(),
            _ => false,
        }
}

/// The hole for a placed spotlight element (`None` for anything else), in local coords.
fn spotlight_hole(el: &Element, off: egui::Vec2) -> Option<Hole> {
    let Element::Shape { kind, a, b, .. } = el else {
        return None;
    };
    let r = egui::Rect::from_two_pos(local(a.0, a.1, off), local(b.0, b.1, off));
    match kind {
        ShapeKind::SpotlightRect => Some(Hole::Rect(r)),
        ShapeKind::SpotlightEllipse => Some(Hole::Ellipse {
            center: r.center(),
            rx: r.width() * 0.5,
            ry: r.height() * 0.5,
        }),
        _ => None,
    }
}

fn is_spotlight_element(el: &Element) -> bool {
    matches!(el, Element::Shape { kind, .. } if kind.is_spotlight())
}

/// The live spotlight hole while Shift is held: the shape being dragged, the pen's snapped
/// ellipse, or — when idle — a flashlight circle around the cursor.
fn live_spotlight_hole(frame: &Frame, off: egui::Vec2) -> Option<Hole> {
    if !spotlight_active(frame) {
        return None;
    }
    let (gx, gy) = frame.pointer?;
    let (gx, gy) = (gx as f32, gy as f32);
    match frame.gesture {
        Gesture::Shape(kind, start) if kind.spotlight().is_some() => {
            let b = if frame.constrain {
                constrain(*kind, *start, (gx, gy))
            } else {
                (gx, gy)
            };
            Some(Hole::Rect(egui::Rect::from_two_pos(
                local(start.0, start.1, off),
                local(b.0, b.1, off),
            )))
        }
        Gesture::SnapEllipse { center, circle } => {
            let (rx, ry) = snap_radii(*center, (gx, gy), *circle || frame.constrain);
            Some(Hole::Ellipse {
                center: local(center.0, center.1, off),
                rx,
                ry,
            })
        }
        // Idle: a flashlight that follows the cursor, unless just-placed (latched) so the
        // drag's release doesn't snap straight back to it. (Pen/eraser/arrow: no spotlight.)
        Gesture::None if frame.flashlight => Some(Hole::Ellipse {
            center: local(gx, gy, off),
            rx: frame.spotlight_radius,
            ry: frame.spotlight_radius,
        }),
        _ => None,
    }
}

/// Paint the dark veil over `screen`, leaving the `holes` bright. The veil is one uniform
/// layer (no double-darkening where holes' surroundings overlap): full-width above and
/// below the holes' band, and scanlined through the band so the gaps between merged hole
/// spans are filled exactly once.
///
/// All bands go into **one raw [`egui::Mesh`]** rather than many `rect_filled` shapes:
/// egui feathers (anti-aliases) the edge of each filled shape, so abutting translucent
/// rectangles leave a faint seam — visible as a flickering horizontal line where the
/// scanlines meet the full-width band. Raw mesh quads aren't feathered, so they tile
/// seamlessly (and it's a single draw call).
fn paint_veil(p: &egui::Painter, screen: egui::Rect, holes: &[Hole], dim_alpha: u8) {
    let dim = egui::Color32::from_black_alpha(dim_alpha);
    let mut mesh = egui::Mesh::default();
    {
        let mut quad = |x0: f32, y0: f32, x1: f32, y1: f32| {
            if x1 > x0 && y1 > y0 {
                mesh.add_colored_rect(
                    egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
                    dim,
                );
            }
        };

        // Restrict the scanline pass to the band that actually contains holes.
        let mut ytop = screen.bottom();
        let mut ybot = screen.top();
        for h in holes {
            let b = h.bounds();
            ytop = ytop.min(b.top());
            ybot = ybot.max(b.bottom());
        }
        ytop = ytop.clamp(screen.top(), screen.bottom());
        ybot = ybot.clamp(screen.top(), screen.bottom());

        quad(screen.left(), screen.top(), screen.right(), ytop);
        quad(screen.left(), ybot, screen.right(), screen.bottom());

        let mut y = ytop;
        while y < ybot {
            let y1 = (y + VEIL_SCAN).min(ybot);
            let ymid = (y + y1) * 0.5;
            let mut spans: Vec<(f32, f32)> = holes
                .iter()
                .filter_map(|h| h.x_span_at(ymid))
                .map(|(s, e)| (s.max(screen.left()), e.min(screen.right())))
                .filter(|(s, e)| e > s)
                .collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut x = screen.left();
            for (s, e) in spans {
                quad(x, y, s, y1);
                x = x.max(e);
            }
            quad(x, y, screen.right(), y1);
            y = y1;
        }
    }
    if !mesh.vertices.is_empty() {
        p.add(egui::Shape::mesh(mesh));
    }

    paint_union_rim(p, holes);
}

/// Outline the selected element with a handled box, so it's clear what the arrow keys
/// will nudge.
fn paint_selection(p: &egui::Painter, off: egui::Vec2, frame: &Frame) {
    let Some(el) = frame.selected.and_then(|i| frame.elements.get(i)) else {
        return;
    };
    let ((x0, y0), (x1, y1)) = el.bounds();
    let r = egui::Rect::from_two_pos(local(x0, y0, off), local(x1, y1, off)).expand(4.0);
    let accent = frame.theme.accent;
    p.rect_stroke(
        r,
        2.0,
        egui::Stroke::new(1.5, accent),
        egui::StrokeKind::Outside,
    );
    for c in [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ] {
        p.rect_filled(
            egui::Rect::from_center_size(c, egui::vec2(5.0, 5.0)),
            1.0,
            accent,
        );
    }
}

/// Paint the text label being typed, with a caret.
fn paint_text_edit(p: &egui::Painter, off: egui::Vec2, frame: &Frame) {
    let Some((pos, buf)) = frame.text_edit else {
        return;
    };
    let at = local(pos.0, pos.1, off);
    let ts = text_size(frame.width);
    let font = egui::FontId::proportional(ts);
    let galley = p.layout_no_wrap(buf.clone(), font.clone(), col(frame.color));
    let size = galley.size();
    p.galley(at, galley, col(frame.color));
    // Caret just past the text.
    let cx = at.x + size.x + 1.0;
    p.line_segment(
        [egui::pos2(cx, at.y), egui::pos2(cx, at.y + ts)],
        egui::Stroke::new(2.0, col(frame.color)),
    );
}

/// A localised tool name for the HUD.
fn tool_label(tool: Tool) -> String {
    match tool {
        Tool::Pen => tr!("draw-tool-pen"),
        Tool::Rect => tr!("draw-tool-rect"),
        Tool::Mask => tr!("draw-tool-mask"),
        Tool::Arrow => tr!("draw-tool-arrow"),
        Tool::Text => tr!("draw-tool-text"),
        Tool::Eraser => tr!("draw-tool-eraser"),
        Tool::Move => tr!("draw-tool-move"),
    }
}

/// Paint the bottom-centre status chip shown while drawing: a colour swatch, the tool
/// name, a sample of the current stroke width (drawn at that width) next to its size in
/// pixels, and a short hint.
fn paint_hud(p: &egui::Painter, ui: &egui::Ui, frame: &Frame) {
    let t = frame.theme;
    let screen = ui.max_rect();
    let in_spotlight = spotlight_active(frame);
    let hint = if frame.passthrough {
        tr!("draw-passthrough-hint")
    } else if frame.text_edit.is_some() {
        tr!("draw-text-hint")
    } else if in_spotlight {
        tr!("draw-spotlight-hint")
    } else {
        tr!("draw-hint")
    };
    let font = egui::FontId::proportional(13.0);
    let tool = p.layout_no_wrap(
        format!("{}  ·", tool_label(frame.tool)),
        font.clone(),
        t.text,
    );
    let info_text = if in_spotlight {
        let pct = (frame.spotlight_dim as f32 / 255.0 * 100.0).round();
        format!("◯ {:.0}px · {pct:.0}%  ·  {hint}", frame.spotlight_radius)
    } else {
        format!("{:.0}px  ·  {hint}", frame.width)
    };
    let info = p.layout_no_wrap(info_text, font, t.text);

    const SW: f32 = 16.0; // colour swatch
    const SAMPLE: f32 = 42.0; // width-sample line length
    let gap = 8.0;
    let pad = egui::vec2(10.0, 7.0);
    let inner_w = SW + gap + tool.size().x + gap + SAMPLE + gap + info.size().x;
    let inner_h = tool.size().y.max(info.size().y).max(SW);
    let box_size = egui::vec2(inner_w, inner_h) + pad * 2.0;
    let origin = egui::pos2(
        screen.center().x - box_size.x * 0.5,
        screen.bottom() - box_size.y - 24.0,
    );
    let bg = egui::Rect::from_min_size(origin, box_size);
    p.rect_filled(bg, 8.0, egui::Color32::from_black_alpha(190));
    // Attention pulse on draw-mode entry: a glowing accent halo + border that blinks a
    // few times, so the chip catches the eye on an otherwise empty screen.
    if frame.flash > 0.0 {
        let a = frame.theme.accent;
        let halo =
            egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), (frame.flash * 70.0) as u8);
        p.rect_filled(bg.expand(6.0 * frame.flash), 12.0, halo);
        let border =
            egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), (frame.flash * 255.0) as u8);
        p.rect_stroke(
            bg,
            8.0,
            egui::Stroke::new(2.5, border),
            egui::StrokeKind::Outside,
        );
    }

    let mid = origin.y + pad.y + inner_h * 0.5;
    let mut x = origin.x + pad.x;
    // Colour swatch.
    let swatch = egui::Rect::from_min_size(egui::pos2(x, mid - SW * 0.5), egui::vec2(SW, SW));
    p.rect_filled(swatch, 3.0, col(frame.color));
    p.rect_stroke(
        swatch,
        3.0,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(120)),
        egui::StrokeKind::Inside,
    );
    x += SW + gap;
    // Tool name.
    p.galley(
        egui::pos2(x, mid - tool.size().y * 0.5),
        tool.clone(),
        t.text,
    );
    x += tool.size().x + gap;
    // A sample of the current stroke width, drawn at that width and colour.
    p.line_segment(
        [egui::pos2(x, mid), egui::pos2(x + SAMPLE, mid)],
        egui::Stroke::new(frame.width.clamp(1.0, inner_h), col(frame.color)),
    );
    x += SAMPLE + gap;
    // Width in pixels + hint.
    p.galley(egui::pos2(x, mid - info.size().y * 0.5), info, t.text);
}

/// Layout for the colour-picker popup: the panel rect plus each swatch rect and its
/// colour, in surface-local coordinates. Shared by [`paint_palette`] and the click
/// hit-test ([`State::palette_click`]) so they always agree.
fn palette_cells(w: f32, h: f32) -> (egui::Rect, Vec<(egui::Rect, Color)>) {
    let colors = model::palette();
    let cols = model::PALETTE_COLS;
    let rows = colors.len() / cols;
    const CELL: f32 = 30.0;
    const GAP: f32 = 3.0;
    const PAD: f32 = 12.0;
    const TITLE: f32 = 22.0;
    let grid_w = cols as f32 * CELL + (cols - 1) as f32 * GAP;
    let grid_h = rows as f32 * CELL + (rows - 1) as f32 * GAP;
    let panel = egui::Rect::from_center_size(
        egui::pos2(w * 0.5, h * 0.5),
        egui::vec2(grid_w + 2.0 * PAD, grid_h + TITLE + 2.0 * PAD),
    );
    let origin = egui::pos2(panel.min.x + PAD, panel.min.y + PAD + TITLE);
    let cells = colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (row, col) = (i / cols, i % cols);
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    origin.x + col as f32 * (CELL + GAP),
                    origin.y + row as f32 * (CELL + GAP),
                ),
                egui::vec2(CELL, CELL),
            );
            (rect, *c)
        })
        .collect();
    (panel, cells)
}

/// Paint the colour-picker popup: a grid of swatches with the current colour outlined.
fn paint_palette(p: &egui::Painter, ui: &egui::Ui, frame: &Frame) {
    let screen = ui.max_rect();
    let (panel, cells) = palette_cells(screen.width(), screen.height());
    p.rect_filled(panel, 10.0, egui::Color32::from_black_alpha(225));
    p.text(
        egui::pos2(panel.min.x + 12.0, panel.min.y + 8.0),
        egui::Align2::LEFT_TOP,
        tr!("draw-palette-title"),
        egui::FontId::proportional(14.0),
        frame.theme.text,
    );
    for (rect, c) in &cells {
        p.rect_filled(*rect, 3.0, col(*c));
        if *c == frame.color {
            p.rect_stroke(
                rect.expand(1.5),
                3.0,
                egui::Stroke::new(2.5, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
    }
}

/// One line of the cheat-sheet: a section header, or a `(key, description)` entry.
#[derive(Clone)]
pub(crate) enum HelpRow {
    Group(String),
    Entry(String, String),
}

/// The full key/gesture cheat-sheet, grouped into sections. Key labels are derived from the
/// live [`Keymap`] so they reflect the user's bindings, and descriptions are localised and
/// key-free (the key lives in its own column). Shared by the on-screen [`paint_help`] legend
/// and the tray's Shortcuts submenu, so they never drift apart.
pub(crate) fn shortcut_rows(km: &Keymap, capture_available: bool) -> Vec<HelpRow> {
    use crate::keymap::trigger_label;
    let key = |a| km.label_for(a);
    let mut rows = vec![HelpRow::Group(tr!("draw-help-group-tools"))];
    let mut entry = |k: String, d: String| rows.push(HelpRow::Entry(k, d));
    entry(key(Action::Pen), tool_label(Tool::Pen));
    entry(key(Action::Rect), tool_label(Tool::Rect));
    entry(key(Action::Mask), tool_label(Tool::Mask));
    entry(key(Action::Arrow), tool_label(Tool::Arrow));
    entry(key(Action::Text), tool_label(Tool::Text));
    entry(key(Action::Move), tool_label(Tool::Move));
    entry(key(Action::Eraser), tool_label(Tool::Eraser));

    rows.push(HelpRow::Group(tr!("draw-help-group-edit")));
    rows.push(HelpRow::Entry(key(Action::Palette), tr!("draw-help-color")));
    rows.push(HelpRow::Entry(
        format!("{} / {}", key(Action::Undo), key(Action::Redo)),
        tr!("draw-help-undo"),
    ));
    rows.push(HelpRow::Entry(
        format!("{} / {}", key(Action::WidthInc), key(Action::WidthDec)),
        tr!("draw-help-width"),
    ));
    rows.push(HelpRow::Entry(key(Action::Clear), tr!("draw-help-clear")));
    rows.push(HelpRow::Entry(
        key(Action::Visibility),
        tr!("draw-help-visibility"),
    ));

    rows.push(HelpRow::Group(tr!("draw-help-group-screen")));
    // Save and Freeze need screen capture; list them only where they work (issue #1).
    if capture_available {
        rows.push(HelpRow::Entry(key(Action::Save), tr!("draw-help-save")));
        rows.push(HelpRow::Entry(key(Action::Freeze), tr!("draw-help-freeze")));
    }
    rows.push(HelpRow::Entry(key(Action::Help), tr!("draw-help-help")));
    rows.push(HelpRow::Entry("Esc".into(), tr!("draw-help-leave")));

    rows.push(HelpRow::Group(tr!("draw-help-group-hold")));
    rows.push(HelpRow::Entry(
        trigger_label(km.constrain),
        tr!("draw-help-constrain"),
    ));
    rows.push(HelpRow::Entry(
        trigger_label(km.spotlight),
        tr!("draw-help-spotlight"),
    ));
    rows.push(HelpRow::Entry(
        tr!("draw-help-key-wheel"),
        tr!("draw-help-spotlight-tune"),
    ));
    rows.push(HelpRow::Entry(
        trigger_label(km.passthrough),
        tr!("draw-help-passthrough"),
    ));

    rows.push(HelpRow::Group(tr!("draw-help-group-pointer")));
    rows.push(HelpRow::Entry(
        tr!("draw-help-key-drag"),
        tr!("draw-help-draw"),
    ));
    rows.push(HelpRow::Entry(
        tr!("draw-help-key-hold"),
        tr!("draw-help-snap"),
    ));
    rows.push(HelpRow::Entry(
        tr!("draw-help-key-rdrag"),
        tr!("draw-help-rightmove"),
    ));
    rows.push(HelpRow::Entry("↑↓←→".into(), tr!("draw-help-move")));
    rows.push(HelpRow::Entry(
        tr!("draw-help-key-type"),
        tr!("draw-help-text"),
    ));
    rows
}

/// Paint the top-left key legend (the `h` shortcut): grouped sections with an accent header,
/// a monospace key column and localised descriptions.
fn paint_help(p: &egui::Painter, _ui: &egui::Ui, frame: &Frame) {
    let t = frame.theme;
    let rows = shortcut_rows(frame.keymap, frame.capture_available);
    let key_font = egui::FontId::monospace(13.0);
    let desc_font = egui::FontId::proportional(13.0);
    let group_font = egui::FontId::proportional(12.0);
    let line_h = 19.0;
    let group_gap = 8.0; // extra space above each section header
    let key_col = 60.0; // width reserved for the key column
    let origin = egui::pos2(28.0, 28.0);
    let content_h: f32 = rows
        .iter()
        .map(|r| match r {
            HelpRow::Group(_) => line_h + group_gap,
            HelpRow::Entry(..) => line_h,
        })
        .sum();
    let panel = egui::Rect::from_min_size(
        origin - egui::vec2(12.0, 12.0),
        egui::vec2(key_col + 280.0, content_h + 40.0),
    );
    p.rect_filled(panel, 10.0, egui::Color32::from_black_alpha(220));
    p.text(
        origin,
        egui::Align2::LEFT_TOP,
        tr!("draw-help-title"),
        egui::FontId::proportional(15.0),
        t.accent,
    );
    let mut y = origin.y + 26.0;
    for row in &rows {
        match row {
            HelpRow::Group(name) => {
                y += group_gap;
                p.text(
                    egui::pos2(origin.x, y),
                    egui::Align2::LEFT_TOP,
                    name,
                    group_font.clone(),
                    t.accent,
                );
                y += line_h;
            }
            HelpRow::Entry(key, desc) => {
                p.text(
                    egui::pos2(origin.x + 10.0, y),
                    egui::Align2::LEFT_TOP,
                    key,
                    key_font.clone(),
                    egui::Color32::from_white_alpha(230),
                );
                p.text(
                    egui::pos2(origin.x + key_col, y),
                    egui::Align2::LEFT_TOP,
                    desc,
                    desc_font.clone(),
                    t.text,
                );
                y += line_h;
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// Wayland handlers
// ---------------------------------------------------------------------------------

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(s) = self
            .surfaces
            .iter_mut()
            .find(|s| s.layer.wl_surface() == surface)
        {
            s.scale = new_factor.max(1) as u32;
            s.layer.wl_surface().set_buffer_scale(new_factor.max(1));
            if let (Some(gpu), true) = (s.gpu.as_ref(), s.width > 0) {
                gpu.resize((s.width * s.scale) as i32, (s.height * s.scale) as i32);
            }
        }
        self.dirty = true;
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.quit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if let Some(s) = self.surfaces.iter_mut().find(|s| &s.layer == layer) {
            if w > 0 && h > 0 {
                s.width = w;
                s.height = h;
            }
            if let Some(gpu) = s.gpu.as_ref() {
                gpu.resize((s.width * s.scale) as i32, (s.height * s.scale) as i32);
            }
        }
        // Attach the (transparent) buffer on the next loop tick.
        self.dirty = true;
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        cap: Capability,
    ) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            // With-repeat so held keys (arrow nudges, text input) auto-repeat: sctk runs
            // its own repeat timer on the calloop loop and fires this callback, which we
            // route through the same `on_key` path as a fresh press.
            let lh = self.loop_handle.clone();
            self.keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    lh,
                    Box::new(|state: &mut State, _kbd, event| state.on_key(&event)),
                )
                .ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.on_key(&event);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        // Only held roles bound to a regular key care about release (to disengage); for
        // everything else this is a no-op, so discrete actions never double-step.
        self.on_key_release(event.keysym);
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
        // Repeat is driven by sctk's own timer via the `get_keyboard_with_repeat`
        // callback (which calls `on_key`); this trait method only fires for
        // compositor-sent repeats, so leaving it empty avoids double-stepping.
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        // Physical Ctrl / Shift drive only the arrow-nudge step granularity, kept on the
        // real modifiers regardless of how the constrain / spotlight roles are bound.
        self.ctrl_held = modifiers.ctrl;
        self.shift_held = modifiers.shift;
        // Roles whose trigger is a modifier follow that modifier's state; roles bound to a
        // key are driven from `on_key` / `on_key_release` instead. `set_*` no-op when the
        // value is unchanged, so this is cheap to call on every modifier event.
        if let Trigger::Mod(m) = self.keymap.passthrough {
            self.set_passthrough(mod_active(&modifiers, m));
        }
        if let Trigger::Mod(m) = self.keymap.constrain {
            self.set_constrain(mod_active(&modifiers, m));
        }
        if let Trigger::Mod(m) = self.keymap.spotlight {
            self.set_spotlight(mod_active(&modifiers, m));
        }
    }
}

/// Read one modifier's state from the xkb modifier set.
fn mod_active(m: &Modifiers, kind: ModKind) -> bool {
    match kind {
        ModKind::Ctrl => m.ctrl,
        ModKind::Shift => m.shift,
        ModKind::Alt => m.alt,
        ModKind::Logo => m.logo,
        ModKind::Caps => m.caps_lock,
    }
}

/// Default screenshot path: `<Pictures>/wlr-draw-<timestamp>.png`. The Pictures dir comes
/// from `xdg-user-dir` (falling back to `~/Pictures`), the stamp from `date`.
fn default_save_path() -> std::path::PathBuf {
    let dir = run_cmd("xdg-user-dir", &["PICTURES"])
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home).join("Pictures")
        });
    let stamp = run_cmd("date", &["+%Y%m%d-%H%M%S"]).unwrap_or_else(|| "shot".into());
    dir.join(format!("wlr-draw-{stamp}.png"))
}

/// Run a command and return its trimmed stdout, or `None` on any failure/empty output.
fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// A scroll event's direction as ±1 (or 0): the click count when discrete, else the sign
/// of the smooth value. Lets one wheel notch be one step regardless of high-res scrolling.
fn axis_notch(a: &AxisScroll) -> f32 {
    if a.discrete != 0 {
        a.discrete as f32
    } else if a.absolute > 0.0 {
        1.0
    } else if a.absolute < 0.0 {
        -1.0
    } else {
        0.0
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let Some(g) = self.to_global(&e.surface, e.position) {
                        self.pointer_pos = Some(g);
                        if !matches!(self.gesture, Gesture::None) {
                            self.on_motion((g.0 as f32, g.1 as f32));
                        } else {
                            self.dirty = true; // move the (potential) shape preview anchor
                        }
                    }
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_pos = None;
                    self.dirty = true;
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => {
                    // The colour popup eats the click (pick a swatch / dismiss) instead
                    // of starting a stroke.
                    if self.show_palette {
                        self.palette_click(&e.surface, e.position);
                    } else if let Some(g) = self.to_global(&e.surface, e.position) {
                        self.pointer_pos = Some(g);
                        self.on_press((g.0 as f32, g.1 as f32));
                    }
                }
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => {
                    let g = self
                        .to_global(&e.surface, e.position)
                        .or(self.pointer_pos)
                        .unwrap_or((0.0, 0.0));
                    self.on_release((g.0 as f32, g.1 as f32));
                }
                // Right button: grab-and-move the element under the cursor, in any tool.
                PointerEventKind::Press {
                    button: BTN_RIGHT, ..
                } => {
                    if let Some(g) = self.to_global(&e.surface, e.position) {
                        self.pointer_pos = Some(g);
                        self.on_right_press((g.0 as f32, g.1 as f32));
                    }
                }
                PointerEventKind::Release {
                    button: BTN_RIGHT, ..
                } => self.on_right_release(),
                // In spotlight mode the wheel resizes the light; the tilt/second wheel
                // dims it. (Stroke width stays on +/-.)
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    ..
                } if self.spotlight_active && self.draw_mode && !self.passthrough => {
                    let v = axis_notch(&vertical);
                    if v != 0.0 {
                        self.adjust_spotlight_radius(-v * SPOTLIGHT_RADIUS_STEP); // up = bigger
                    }
                    let h = axis_notch(&horizontal);
                    if h != 0.0 {
                        self.adjust_spotlight_dim(h * SPOTLIGHT_DIM_STEP); // right = darker
                    }
                }
                _ => {}
            }
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_seat!(State);
delegate_keyboard!(State);
delegate_pointer!(State);
delegate_layer!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::arrowhead_len;

    #[test]
    fn arrowhead_never_panics_and_stays_within_the_shaft() {
        // Regression: a short arrow (len < 16.67) made `0.6*len` fall below the 10px
        // floor, so `clamp(10.0, 0.6*len)` panicked with min > max. The head must instead
        // shrink to the 60% cap. `len ≈ 1.28` reproduces the original panic.
        assert!((arrowhead_len(6.0, 1.276) - 0.6 * 1.276).abs() < 1e-4);
        // The head never exceeds 60% of the shaft, whatever the width.
        for &(w, len) in &[(1.0, 5.0), (6.0, 10.0), (200.0, 3.0), (2.0, 16.6)] {
            let h = arrowhead_len(w, len);
            assert!(
                h.is_finite() && h <= 0.6 * len + 1e-4,
                "w={w} len={len} h={h}"
            );
        }
        // A normal arrow keeps the width-scaled head within [10, 60% len].
        assert_eq!(arrowhead_len(6.0, 100.0), 27.0); // 6*4.5, within [10, 60]
        assert_eq!(arrowhead_len(1.0, 100.0), 10.0); // 4.5 floored to 10
    }
}
