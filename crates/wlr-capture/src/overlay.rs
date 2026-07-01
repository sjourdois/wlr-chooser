//! Interactive frozen-screen overlay: a `wlr-layer-shell` surface per output showing
//! that output's frozen capture, shared by several interactions ([`Mode`]):
//! [`select_region`] (drag a rectangle), [`pick_point`] (the `wlr-peek` colour picker,
//! with a loupe) and [`magnify`] (a full-screen zoom that follows the cursor).
//! Coordinates are tracked in the global logical space so they span outputs. Rendered
//! with egui on the shared [`render::Gpu`](crate::render::Gpu), like the chooser's
//! overlay — one surface (and GL context) per output.

use crate::capture::OutputCapture;
use crate::render::Gpu;
use crate::wl::Region;
use crate::error::{CaptureError, Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4d, 0x9a, 0xff);

/// What the frozen overlay collects from the user.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Drag a rectangle — the region selector ([`select_region`]).
    Region,
    /// Move and click to pick a single pixel — the colour picker ([`pick_point`]),
    /// with a magnifying loupe and a live hex readout.
    Point,
    /// Magnify the frozen screen around the cursor ([`magnify`]); scroll to zoom,
    /// `Esc` to quit. There is no selection to confirm.
    Magnify,
}

/// The overlay's result, in global logical coordinates. Which variant comes back is
/// determined by the [`Mode`] the overlay ran in.
enum Outcome {
    Region(Region),
    Point { x: i32, y: i32 },
}

/// One output's overlay surface, its frozen backdrop, and its GL context.
struct Surface {
    layer: LayerSurface,
    /// Output top-left in the global logical space (to map pointer ↔ selection).
    logical_x: i32,
    logical_y: i32,
    egui_ctx: egui::Context,
    gpu: Option<Gpu>,
    tex: Option<egui::TextureHandle>,
    /// Frozen pixels (RGBA). Uploaded to `tex` for the backdrop, and kept around so
    /// the colour picker's loupe can sample exact pixel values.
    rgba: Vec<u8>,
    img_w: usize,
    img_h: usize,
    /// Configured logical size (points) and integer scale.
    width: u32,
    height: u32,
    scale: u32,
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

    /// Render the frozen backdrop plus the mode-specific overlay (selection
    /// rectangle, or crosshair + loupe), in global logical coordinates.
    fn render(
        &mut self,
        conn: &Connection,
        mode: Mode,
        hint: &str,
        selection: Option<Region>,
        pointer: Option<(f64, f64)>,
        zoom: f32,
    ) {
        self.ensure_gpu(conn);
        if self.width == 0 {
            return;
        }
        if self.tex.is_none() {
            let img =
                egui::ColorImage::from_rgba_unmultiplied([self.img_w, self.img_h], &self.rgba);
            self.tex = Some(self.egui_ctx.load_texture(
                "frozen",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width as f32, self.height as f32),
            )),
            ..Default::default()
        };
        let tex = self.tex.clone();
        let (w, h) = (self.width as f32, self.height as f32);
        let (lx, ly) = (self.logical_x, self.logical_y);
        let (img_w, img_h) = (self.img_w, self.img_h);
        let frozen: &[u8] = &self.rgba;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.render(
            &self.egui_ctx,
            raw_input,
            self.scale as f32,
            (pw, ph),
            [0.0, 0.0, 0.0, 1.0],
            |ui, _importer| match mode {
                Mode::Region => {
                    draw_region_overlay(ui, tex.as_ref(), w, h, lx, ly, selection, pointer, hint)
                }
                Mode::Point => draw_point_overlay(
                    ui,
                    tex.as_ref(),
                    w,
                    h,
                    lx,
                    ly,
                    pointer,
                    frozen,
                    img_w,
                    img_h,
                    hint,
                ),
                Mode::Magnify => {
                    draw_magnify_overlay(ui, tex.as_ref(), w, h, lx, ly, pointer, zoom, hint)
                }
            },
        );
        self.layer.commit();
    }
}

/// Paint one surface: the frozen image, a dim veil, the bright selection and its
/// outline + size label.
#[allow(clippy::too_many_arguments)]
fn draw_region_overlay(
    ui: &mut egui::Ui,
    tex: Option<&egui::TextureHandle>,
    w: f32,
    h: f32,
    lx: i32,
    ly: i32,
    selection: Option<Region>,
    pointer: Option<(f64, f64)>,
    hint: &str,
) {
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            let p = ui.painter();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            if let Some(t) = tex {
                p.image(t.id(), screen, full_uv, egui::Color32::WHITE);
            }

            match selection {
                None => {
                    // Idle: a light veil makes it obvious the screen is frozen and
                    // waiting for a selection.
                    p.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(48));
                }
                Some(sel) => {
                    // Selection in this surface's local (point) coordinates.
                    let local = egui::Rect::from_min_size(
                        egui::pos2(sel.x as f32 - lx as f32, sel.y as f32 - ly as f32),
                        egui::vec2(sel.w as f32, sel.h as f32),
                    );
                    // Dim everything, then restore the selected area at full brightness.
                    p.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(120));
                    let vis = local.intersect(screen);
                    if vis.width() > 0.5
                        && vis.height() > 0.5
                        && let Some(t) = tex
                    {
                        let uv = egui::Rect::from_min_max(
                            egui::pos2(vis.min.x / w, vis.min.y / h),
                            egui::pos2(vis.max.x / w, vis.max.y / h),
                        );
                        p.image(t.id(), vis, uv, egui::Color32::WHITE);
                    }
                    p.rect_stroke(
                        local,
                        0.0,
                        egui::Stroke::new(2.0, ACCENT),
                        egui::StrokeKind::Inside,
                    );
                    // Size label, once, on the surface holding the selection's top-left.
                    if screen.contains(local.min) {
                        p.text(
                            local.min + egui::vec2(6.0, 6.0),
                            egui::Align2::LEFT_TOP,
                            format!("{} × {}", sel.w, sel.h),
                            egui::FontId::monospace(13.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            }

            // Crosshair following the cursor on whichever surface holds it, plus an
            // idle hint so the selection mode is obvious.
            if let Some((gx, gy)) = pointer {
                let (cx, cy) = (gx as f32 - lx as f32, gy as f32 - ly as f32);
                if (0.0..=w).contains(&cx) && (0.0..=h).contains(&cy) {
                    let col = egui::Color32::from_white_alpha(150);
                    p.line_segment(
                        [egui::pos2(0.0, cy), egui::pos2(w, cy)],
                        egui::Stroke::new(1.0, col),
                    );
                    p.line_segment(
                        [egui::pos2(cx, 0.0), egui::pos2(cx, h)],
                        egui::Stroke::new(1.0, col),
                    );
                    if selection.is_none() {
                        let galley = p.layout_no_wrap(
                            hint.to_owned(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::WHITE,
                        );
                        let at = egui::pos2(cx + 16.0, cy + 16.0);
                        let bg = egui::Rect::from_min_size(
                            at - egui::vec2(8.0, 5.0),
                            galley.size() + egui::vec2(16.0, 10.0),
                        );
                        p.rect_filled(bg, 6.0, egui::Color32::from_black_alpha(200));
                        p.galley(at, galley, egui::Color32::WHITE);
                    }
                }
            }
        });
}

/// Paint the colour-picker surface: the true-colour frozen image, a crosshair, and a
/// magnifying loupe around the cursor showing individual pixels plus the hex value of
/// the one under the centre. The loupe samples `frozen` (RGBA) directly so the zoom
/// is pixel-exact regardless of the backdrop texture's filtering.
#[allow(clippy::too_many_arguments)]
fn draw_point_overlay(
    ui: &mut egui::Ui,
    tex: Option<&egui::TextureHandle>,
    w: f32,
    h: f32,
    lx: i32,
    ly: i32,
    pointer: Option<(f64, f64)>,
    frozen: &[u8],
    img_w: usize,
    img_h: usize,
    hint: &str,
) {
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            let p = ui.painter();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            if let Some(t) = tex {
                p.image(t.id(), screen, full_uv, egui::Color32::WHITE);
            }
            let Some((gx, gy)) = pointer else { return };
            let (cx, cy) = (gx as f32 - lx as f32, gy as f32 - ly as f32);
            if !((0.0..=w).contains(&cx) && (0.0..=h).contains(&cy)) {
                return;
            }

            // Map the cursor (local logical points) to a frozen image pixel, then a
            // sampler over the surrounding pixels.
            let px = ((cx / w) * img_w as f32).floor() as isize;
            let py = ((cy / h) * img_h as f32).floor() as isize;
            let sample = |ix: isize, iy: isize| -> Option<egui::Color32> {
                if ix < 0 || iy < 0 || ix >= img_w as isize || iy >= img_h as isize {
                    return None;
                }
                let idx = ((iy as usize) * img_w + ix as usize) * 4;
                frozen
                    .get(idx..idx + 3)
                    .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
            };

            // Crosshair across the whole surface.
            let col = egui::Color32::from_white_alpha(120);
            p.line_segment(
                [egui::pos2(0.0, cy), egui::pos2(w, cy)],
                egui::Stroke::new(1.0, col),
            );
            p.line_segment(
                [egui::pos2(cx, 0.0), egui::pos2(cx, h)],
                egui::Stroke::new(1.0, col),
            );

            // Loupe: a (2R+1)² grid of source pixels magnified to Z×Z squares,
            // offset from the cursor and flipped to stay on-screen.
            const R: isize = 6;
            const Z: f32 = 11.0;
            let loupe = (2 * R + 1) as f32 * Z;
            let mut origin = egui::pos2(cx + 20.0, cy + 20.0);
            if origin.x + loupe > w {
                origin.x = cx - 20.0 - loupe;
            }
            if origin.y + loupe + 26.0 > h {
                origin.y = cy - 20.0 - loupe - 26.0;
            }
            origin.x = origin.x.clamp(2.0, (w - loupe).max(2.0));
            origin.y = origin.y.clamp(2.0, (h - loupe - 26.0).max(2.0));

            let frame = egui::Rect::from_min_size(origin, egui::vec2(loupe, loupe));
            p.rect_filled(frame.expand(3.0), 4.0, egui::Color32::from_black_alpha(220));
            for dy in -R..=R {
                for dx in -R..=R {
                    let c = sample(px + dx, py + dy).unwrap_or(egui::Color32::from_gray(20));
                    let cell = egui::Rect::from_min_size(
                        origin + egui::vec2((dx + R) as f32 * Z, (dy + R) as f32 * Z),
                        egui::vec2(Z, Z),
                    );
                    p.rect_filled(cell, 0.0, c);
                }
            }
            // Outline the centre cell (the pixel that will be picked).
            let centre = egui::Rect::from_min_size(
                origin + egui::vec2(R as f32 * Z, R as f32 * Z),
                egui::vec2(Z, Z),
            );
            p.rect_stroke(
                centre,
                0.0,
                egui::Stroke::new(1.5, ACCENT),
                egui::StrokeKind::Outside,
            );

            // Hex readout + colour swatch under the loupe.
            if let Some(c) = sample(px, py) {
                let hex = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
                let at = egui::pos2(origin.x, origin.y + loupe + 4.0);
                let galley =
                    p.layout_no_wrap(hex, egui::FontId::monospace(15.0), egui::Color32::WHITE);
                let bg = egui::Rect::from_min_size(
                    at - egui::vec2(4.0, 2.0),
                    galley.size() + egui::vec2(30.0, 6.0),
                );
                p.rect_filled(bg, 4.0, egui::Color32::from_black_alpha(220));
                let sw = egui::Rect::from_min_size(
                    egui::pos2(bg.max.x - 20.0, bg.min.y + 3.0),
                    egui::vec2(14.0, bg.height() - 6.0),
                );
                p.rect_filled(sw, 2.0, c);
                p.galley(at, galley, egui::Color32::WHITE);

                // Hint below the readout.
                let hint = p.layout_no_wrap(
                    hint.to_owned(),
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_white_alpha(200),
                );
                let hat = egui::pos2(bg.min.x + 2.0, bg.max.y + 4.0);
                let hbg = egui::Rect::from_min_size(
                    hat - egui::vec2(4.0, 2.0),
                    hint.size() + egui::vec2(8.0, 4.0),
                );
                p.rect_filled(hbg, 4.0, egui::Color32::from_black_alpha(200));
                p.galley(hat, hint, egui::Color32::from_white_alpha(200));
            }
        });
}

/// Paint the magnifier surface: the frozen image zoomed `zoom`× around the cursor (the
/// point under the cursor stays put), with a crosshair and a zoom + quit-hint readout.
/// A surface not holding the cursor shows its frozen image at 1×.
#[allow(clippy::too_many_arguments)]
fn draw_magnify_overlay(
    ui: &mut egui::Ui,
    tex: Option<&egui::TextureHandle>,
    w: f32,
    h: f32,
    lx: i32,
    ly: i32,
    pointer: Option<(f64, f64)>,
    zoom: f32,
    hint: &str,
) {
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            let p = ui.painter();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            let Some(t) = tex else { return };

            // Cursor in this surface's local coordinates, if it is here at all.
            let here = pointer
                .map(|(gx, gy)| (gx as f32 - lx as f32, gy as f32 - ly as f32))
                .filter(|(cx, cy)| (0.0..=w).contains(cx) && (0.0..=h).contains(cy));
            let Some((cx, cy)) = here else {
                // Cursor elsewhere: show this output's frozen image at 1×.
                p.image(t.id(), screen, full_uv, egui::Color32::WHITE);
                return;
            };

            // Map the screen to a uv window so the point under the cursor stays put
            // and everything around it is magnified `zoom`×.
            let (cu, cv) = (cx / w, cy / h);
            let uv = egui::Rect::from_min_max(
                egui::pos2(cu - cx / (zoom * w), cv - cy / (zoom * h)),
                egui::pos2(cu + (w - cx) / (zoom * w), cv + (h - cy) / (zoom * h)),
            );
            p.image(t.id(), screen, uv, egui::Color32::WHITE);

            // Crosshair at the focus point.
            let col = egui::Color32::from_white_alpha(120);
            p.line_segment(
                [egui::pos2(0.0, cy), egui::pos2(w, cy)],
                egui::Stroke::new(1.0, col),
            );
            p.line_segment(
                [egui::pos2(cx, 0.0), egui::pos2(cx, h)],
                egui::Stroke::new(1.0, col),
            );

            // Zoom readout + quit hint, top-left.
            let label = format!("{zoom:.1}×  ·  {}", hint);
            let galley = p.layout_no_wrap(
                label,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
            let at = egui::pos2(14.0, 14.0);
            let bg = egui::Rect::from_min_size(
                at - egui::vec2(6.0, 4.0),
                galley.size() + egui::vec2(12.0, 8.0),
            );
            p.rect_filled(bg, 6.0, egui::Color32::from_black_alpha(200));
            p.galley(at, galley, egui::Color32::WHITE);
        });
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    surfaces: Vec<Surface>,
    /// Which interaction the overlay is running.
    mode: Mode,
    /// The caller-provided, localised on-screen hint for this mode.
    hint: String,

    /// Live pointer position (global logical), for the crosshair + hint.
    pointer_pos: Option<(f64, f64)>,
    /// Drag anchor and current point, in global logical coordinates (region mode).
    start: Option<(f64, f64)>,
    cur: Option<(f64, f64)>,
    /// Magnification factor (magnify mode); changed with the scroll wheel.
    zoom: f32,
    /// The overlay must be redrawn on all surfaces.
    dirty: bool,
    /// Resolved result (on confirm) and whether the loop should exit.
    result: Option<Outcome>,
    done: bool,
}

impl State {
    /// Current selection rectangle (normalized) from the drag, if any.
    fn selection(&self) -> Option<Region> {
        let (sx, sy) = self.start?;
        let (cx, cy) = self.cur?;
        let (x0, y0) = (sx.min(cx), sy.min(cy));
        let (x1, y1) = (sx.max(cx), sy.max(cy));
        let r = Region {
            x: x0.floor() as i32,
            y: y0.floor() as i32,
            w: (x1 - x0).round() as u32,
            h: (y1 - y0).round() as u32,
        };
        (!r.is_empty()).then_some(r)
    }

    /// Map a pointer event's surface-local position to global logical coordinates.
    fn to_global(&self, surface: &wl_surface::WlSurface, pos: (f64, f64)) -> Option<(f64, f64)> {
        self.surfaces
            .iter()
            .find(|s| s.layer.wl_surface() == surface)
            .map(|s| (s.logical_x as f64 + pos.0, s.logical_y as f64 + pos.1))
    }

    fn redraw_all(&mut self, conn: &Connection) {
        let sel = self.selection();
        let ptr = self.pointer_pos;
        let mode = self.mode;
        for s in &mut self.surfaces {
            s.render(conn, mode, &self.hint, sel, ptr, self.zoom);
        }
    }
}

/// Drag a rectangle on a frozen overlay spanning every captured output; returns the
/// chosen region (global logical coordinates) or `None` if cancelled (`Esc`).
pub fn select_region(captures: &[OutputCapture], hint: &str) -> Result<Option<Region>> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    select_region_on(&conn, captures, hint)
}

/// [`select_region`] on a caller-provided connection. Use this to chain a second
/// GPU/EGL overlay (e.g. a live mirror) in the *same* process: EGL caches its display
/// by the `wl_display` pointer, so a second connection there can alias a freed one and
/// fail (`eglCreateWindowSurface: BadAlloc`). Sharing one connection (one `EGLDisplay`)
/// avoids it — the same way the per-output surfaces already share one here.
pub fn select_region_on(
    conn: &Connection,
    captures: &[OutputCapture],
    hint: &str,
) -> Result<Option<Region>> {
    Ok(run(conn, captures, Mode::Region, hint)?.map(|o| match o {
        Outcome::Region(r) => r,
        Outcome::Point { .. } => unreachable!("region mode yields a region"),
    }))
}

/// Pick a single pixel on a frozen overlay (with a magnifying loupe); returns its
/// position in global logical coordinates, or `None` if cancelled (`Esc`).
pub fn pick_point(captures: &[OutputCapture], hint: &str) -> Result<Option<(i32, i32)>> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    pick_point_on(&conn, captures, hint)
}

/// [`pick_point`] on a caller-provided connection. Establish this connection *before*
/// capturing: capture may open and drop a transient EGL connection (the GPU readback),
/// and EGL caches its display by the `wl_display` pointer, so an overlay connection
/// opened afterwards can alias the freed one and fail (`eglCreateWindowSurface:
/// BadAlloc`). Creating the overlay connection first keeps its `EGLDisplay` valid.
pub fn pick_point_on(
    conn: &Connection,
    captures: &[OutputCapture],
    hint: &str,
) -> Result<Option<(i32, i32)>> {
    Ok(run(conn, captures, Mode::Point, hint)?.map(|o| match o {
        Outcome::Point { x, y } => (x, y),
        Outcome::Region(_) => unreachable!("point mode yields a point"),
    }))
}

/// Magnify the frozen `captures` around the cursor: a full-screen zoom that pans as
/// the pointer moves, scroll to change the zoom, `Esc` to quit. Returns when the user
/// quits. Not live (the screen is frozen on entry) — a fullscreen live magnifier
/// would capture its own output.
pub fn magnify(captures: &[OutputCapture], hint: &str) -> Result<()> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    magnify_on(&conn, captures, hint)
}

/// [`magnify`] on a caller-provided connection. As with [`pick_point_on`], open this
/// connection before capturing so the overlay's `EGLDisplay` can't alias a freed one.
pub fn magnify_on(conn: &Connection, captures: &[OutputCapture], hint: &str) -> Result<()> {
    run(conn, captures, Mode::Magnify, hint)?;
    Ok(())
}

/// Run the frozen overlay over `captures` in the given [`Mode`] on `conn`; returns the
/// user's choice or `None` if cancelled.
fn run(
    conn: &Connection,
    captures: &[OutputCapture],
    mode: Mode,
    hint: &str,
) -> Result<Option<Outcome>> {
    let (globals, mut queue) = registry_queue_init(conn).context("Wayland registry")?;
    let qh = queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("layer-shell missing")?;

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        keyboard: None,
        pointer: None,
        surfaces: Vec::new(),
        mode,
        hint: hint.to_string(),
        pointer_pos: None,
        start: None,
        cur: None,
        zoom: 3.0,
        dirty: false,
        result: None,
        done: false,
    };

    // Let outputs (and their logical geometry) come in, then build one overlay per
    // output that we have a frozen capture for.
    queue.roundtrip(&mut state).context("Wayland roundtrip")?;

    let outputs: Vec<_> = state.output_state.outputs().collect();
    for wl_out in outputs {
        let Some(info) = state.output_state.info(&wl_out) else {
            continue;
        };
        let Some(name) = info.name.clone() else {
            continue;
        };
        let Some(cap) = captures.iter().find(|c| c.output.name == name) else {
            continue;
        };
        let (lx, ly) = info
            .logical_position
            .unwrap_or((cap.output.logical_x, cap.output.logical_y));

        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("wlr-overlay"),
            Some(&wl_out),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        layer.commit();

        let egui_ctx = egui::Context::default();
        state.surfaces.push(Surface {
            layer,
            logical_x: lx,
            logical_y: ly,
            egui_ctx,
            gpu: None,
            tex: None,
            rgba: cap.image.rgba.clone(),
            img_w: cap.image.width as usize,
            img_h: cap.image.height as usize,
            width: 0,
            height: 0,
            scale: 1,
        });
    }

    if state.surfaces.is_empty() {
        return Err(CaptureError::msg("no output to select"));
    }

    while !state.done {
        queue.blocking_dispatch(&mut state).context("Wayland dispatch")?;
        if state.dirty {
            state.dirty = false;
            state.redraw_all(conn);
        }
    }

    Ok(state.result)
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        conn: &Connection,
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
        let sel = self.selection();
        let ptr = self.pointer_pos;
        let mode = self.mode;
        if let Some(s) = self
            .surfaces
            .iter_mut()
            .find(|s| s.layer.wl_surface() == surface)
        {
            s.render(conn, mode, &self.hint, sel, ptr, self.zoom);
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let sel = self.selection();
        let ptr = self.pointer_pos;
        let mode = self.mode;
        if let Some(s) = self
            .surfaces
            .iter_mut()
            .find(|s| s.layer.wl_surface() == surface)
        {
            s.render(conn, mode, &self.hint, sel, ptr, self.zoom);
        }
    }

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
        self.done = true; // a surface went away: bail out (cancel)
    }

    fn configure(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        let sel = self.selection();
        let ptr = self.pointer_pos;
        let mode = self.mode;
        if let Some(s) = self.surfaces.iter_mut().find(|s| &s.layer == layer) {
            if w > 0 && h > 0 {
                s.width = w;
                s.height = h;
            }
            if s.width == 0 {
                return;
            }
            if let Some(gpu) = s.gpu.as_ref() {
                gpu.resize((s.width * s.scale) as i32, (s.height * s.scale) as i32);
            }
            s.render(conn, mode, &self.hint, sel, ptr, self.zoom);
        }
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
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
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
        match event.keysym {
            Keysym::Escape => {
                self.result = None;
                self.done = true;
            }
            Keysym::Return | Keysym::KP_Enter => match self.mode {
                Mode::Region => {
                    if let Some(r) = self.selection() {
                        self.result = Some(Outcome::Region(r));
                        self.done = true;
                    }
                }
                Mode::Point => {
                    if let Some((gx, gy)) = self.pointer_pos {
                        self.result = Some(Outcome::Point {
                            x: gx.round() as i32,
                            y: gy.round() as i32,
                        });
                        self.done = true;
                    }
                }
                // Nothing to confirm; quit on Esc only.
                Mode::Magnify => {}
            },
            _ => {}
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
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
        let mode = self.mode;
        for e in events {
            match e.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_pos = self.to_global(&e.surface, e.position);
                    self.dirty = true;
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_pos = None;
                    self.dirty = true;
                }
                PointerEventKind::Press { button: 0x110, .. } => {
                    let g = self.to_global(&e.surface, e.position);
                    match mode {
                        Mode::Region => {
                            if let Some(g) = g {
                                self.start = Some(g);
                                self.cur = Some(g);
                                self.pointer_pos = Some(g);
                                self.dirty = true;
                            }
                        }
                        // Point mode: a click picks the pixel under the cursor.
                        Mode::Point => {
                            if let Some((gx, gy)) = g {
                                self.result = Some(Outcome::Point {
                                    x: gx.round() as i32,
                                    y: gy.round() as i32,
                                });
                                self.done = true;
                            }
                        }
                        Mode::Magnify => {}
                    }
                }
                // Magnify mode: scroll wheel changes the zoom (scroll up = zoom in).
                PointerEventKind::Axis { vertical, .. } if mode == Mode::Magnify => {
                    let notches = if vertical.discrete != 0 {
                        vertical.discrete as f32
                    } else {
                        (vertical.absolute / 15.0) as f32
                    };
                    if notches != 0.0 {
                        // Wayland axis is positive downward; scrolling up zooms in.
                        self.zoom = (self.zoom * 1.15_f32.powf(-notches)).clamp(1.5, 40.0);
                        self.dirty = true;
                    }
                }
                PointerEventKind::Motion { .. } => {
                    // Track the cursor for the crosshair/loupe, and extend a drag.
                    if let Some(g) = self.to_global(&e.surface, e.position) {
                        self.pointer_pos = Some(g);
                        if mode == Mode::Region && self.start.is_some() {
                            self.cur = Some(g);
                        }
                        self.dirty = true;
                    }
                }
                PointerEventKind::Release { button: 0x110, .. } if mode == Mode::Region => {
                    if let Some(g) = self.to_global(&e.surface, e.position) {
                        self.cur = Some(g);
                    }
                    if let Some(r) = self.selection() {
                        self.result = Some(Outcome::Region(r));
                        self.done = true;
                    } else {
                        // Empty (a click without a drag): reset, keep waiting.
                        self.start = None;
                        self.cur = None;
                        self.dirty = true;
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
