//! xdg-toplevel windowing host for the live mirror.
//!
//! A normal floating window (so the compositor handles stacking; pair with sway
//! rules `floating enable, sticky enable` for always-on-top across workspaces).
//! Rendering uses the shared [`wlr_capture::render::Gpu`]. Capture frames arrive
//! over a calloop channel, so we only repaint when there is new content or an
//! interaction — no free-running render loop for an always-on tile.
//!
//! Interactions (all hit-tested here, in logical coordinates, so egui stays a
//! pure painter): drag the body to move (`xdg_toplevel.move`), the bottom-right
//! grip to resize (`xdg_toplevel.resize`), the toolbar buttons to collapse to an
//! icon badge or close, and `Esc` to close. The tile keeps the source aspect
//! ratio; collapsing shrinks it, and any new frame while collapsed restores it.

use crate::pip::PipMsg;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop::channel::{Channel, Event as ChannelEvent, channel},
    reexports::calloop_wayland_source::WaylandSource,
    reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
};
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wlr_capture::render::{DmabufImporter, Gpu};
use wlr_capture::theme::Theme;
use wlr_capture::tr;

/// Texture cache key for the single mirrored source.
const KEY: &str = "pip";
/// Default tile width (logical px) before the source aspect ratio is known.
const DEFAULT_W: u32 = 480;
/// Side of the collapsed icon badge (logical px).
const BADGE: u32 = 132;
/// Smallest tile width the user can resize to.
const MIN_W: u32 = 120;
/// Repaint frames to keep the "just restored" accent border visible.
const ACCENT_FRAMES: u32 = 60;
/// How long to show the "window closed" notice before exiting.
const GONE_LINGER: Duration = Duration::from_millis(1400);

/// Frames + textures for the mirrored source; a pure painter (the host owns input).
struct Content {
    /// shm texture (CPU upload path).
    tex: Option<egui::TextureHandle>,
    /// dma-buf texture (GPU zero-copy path): egui id + source pixel size.
    native: Option<(egui::TextureId, egui::Vec2)>,
    icon: Option<egui::TextureHandle>,
    /// Frames awaiting upload/import at the next render.
    pending: Vec<PipMsg>,
    gone: bool,
    label: String,
    theme: Theme,
}

impl Content {
    /// Drain pending frames into textures (needs the egui ctx for shm uploads and
    /// the host importer for dma-buf). Called inside the render closure.
    fn pump(&mut self, ctx: &egui::Context, importer: &mut dyn DmabufImporter) {
        for msg in self.pending.drain(..) {
            match msg {
                PipMsg::Shm { w, h, rgba } if w > 0 && h > 0 => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    match self.tex.as_mut() {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            self.tex =
                                Some(ctx.load_texture(KEY, img, egui::TextureOptions::LINEAR))
                        }
                    }
                    self.native = None; // shm now authoritative
                }
                PipMsg::Dmabuf { frame } => {
                    if let Some(t) = importer.import(KEY, frame) {
                        self.native = Some(t);
                    }
                }
                PipMsg::Shm { .. } => {}
                PipMsg::Gone => self.gone = true,
            }
        }
    }

    /// The drawable texture: dma-buf if present, else shm.
    fn tex(&self) -> Option<(egui::TextureId, egui::Vec2)> {
        if let Some(t) = self.native {
            return Some(t);
        }
        self.tex.as_ref().map(|t| (t.id(), t.size_vec2()))
    }

    /// Paint one frame into the surface-sized area.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        ctx: &egui::Context,
        size: (f32, f32),
        collapsed: bool,
        hovered: bool,
        accent: bool,
        frozen: bool,
        opacity: f32,
    ) {
        let (w, h) = size;
        let full = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let t = &self.theme;
        // Image/icon tint carries the window opacity (the GL backdrop carries it too).
        let tint = egui::Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let p = ui.painter();
                // The captured image, contained (letterboxed) in the tile.
                if let Some((id, ts)) = self.tex() {
                    let scale = (full.width() / ts.x).min(full.height() / ts.y);
                    let draw = egui::Rect::from_center_size(full.center(), ts * scale);
                    p.image(
                        id,
                        draw,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tint,
                    );
                } else if let Some(icon) = &self.icon {
                    let isz = icon.size_vec2();
                    let scale = (full.width() / isz.x).min(full.height() / isz.y).min(1.0);
                    let draw = egui::Rect::from_center_size(full.center(), isz * scale);
                    p.image(
                        icon.id(),
                        draw,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        tint,
                    );
                } else {
                    p.text(
                        full.center(),
                        egui::Align2::CENTER_CENTER,
                        tr!("loading"),
                        egui::FontId::proportional(16.0),
                        t.text_dim,
                    );
                }

                if self.gone {
                    p.rect_filled(full, 0.0, egui::Color32::from_black_alpha(180));
                    p.text(
                        full.center(),
                        egui::Align2::CENTER_CENTER,
                        tr!("pip-gone"),
                        egui::FontId::proportional(16.0),
                        t.text,
                    );
                    return;
                }

                // "Just restored" / live accent border.
                if accent {
                    p.rect_stroke(
                        full,
                        0.0,
                        egui::Stroke::new(2.0, t.window_accent),
                        egui::StrokeKind::Inside,
                    );
                }

                // Freeze indicator: a pause glyph in the top-left corner.
                if frozen {
                    let bar = egui::Stroke::new(3.0, egui::Color32::from_white_alpha(220));
                    let (x, y) = (8.0, 8.0);
                    p.line_segment([egui::pos2(x, y), egui::pos2(x, y + 12.0)], bar);
                    p.line_segment([egui::pos2(x + 6.0, y), egui::pos2(x + 6.0, y + 12.0)], bar);
                }

                if collapsed {
                    return; // badge: just the contained image/icon
                }

                if hovered {
                    let (close, collapse) = toolbar_rects(w);
                    let strip = egui::Rect::from_min_max(
                        egui::pos2(0.0, 0.0),
                        egui::pos2(w, close.bottom() + 6.0),
                    );
                    p.rect_filled(strip, 0.0, egui::Color32::from_black_alpha(150));
                    // Title at the left.
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        self.label.clone(),
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(
                        (collapse.left() - 12.0).max(0.0),
                    );
                    let galley = ui.fonts(|f| f.layout_job(job));
                    p.galley(
                        egui::pos2(8.0, strip.center().y - galley.size().y / 2.0),
                        galley,
                        egui::Color32::WHITE,
                    );
                    // Collapse glyph (a downward chevron) and close glyph (an X).
                    draw_collapse(p, collapse, egui::Color32::WHITE);
                    draw_close(p, close, egui::Color32::WHITE);
                    // Resize grip in the bottom-right corner.
                    draw_grip(p, grip_rect(w, h), egui::Color32::from_white_alpha(160));
                }
            });
    }
}

/// Close / collapse button rects (top-right), in logical coordinates.
fn toolbar_rects(w: f32) -> (egui::Rect, egui::Rect) {
    let s = 22.0;
    let pad = 6.0;
    let close = egui::Rect::from_min_size(egui::pos2(w - pad - s, pad), egui::vec2(s, s));
    let collapse =
        egui::Rect::from_min_size(egui::pos2(w - pad - 2.0 * s - 6.0, pad), egui::vec2(s, s));
    (close, collapse)
}

/// Resize-grip rect (bottom-right), in logical coordinates.
fn grip_rect(w: f32, h: f32) -> egui::Rect {
    let s = 18.0;
    egui::Rect::from_min_size(egui::pos2(w - s, h - s), egui::vec2(s, s))
}

fn draw_close(p: &egui::Painter, r: egui::Rect, c: egui::Color32) {
    let r = r.shrink(5.0);
    let s = egui::Stroke::new(2.0, c);
    p.line_segment([r.left_top(), r.right_bottom()], s);
    p.line_segment([r.right_top(), r.left_bottom()], s);
}

fn draw_collapse(p: &egui::Painter, r: egui::Rect, c: egui::Color32) {
    let r = r.shrink(5.0);
    let s = egui::Stroke::new(2.0, c);
    // A downward chevron ⌄.
    p.line_segment([r.left_top(), egui::pos2(r.center().x, r.bottom())], s);
    p.line_segment([egui::pos2(r.center().x, r.bottom()), r.right_top()], s);
}

fn draw_grip(p: &egui::Painter, r: egui::Rect, c: egui::Color32) {
    let s = egui::Stroke::new(1.5, c);
    for f in [0.35_f32, 0.7] {
        p.line_segment(
            [
                egui::pos2(r.right() - r.width() * f, r.bottom()),
                egui::pos2(r.right(), r.bottom() - r.height() * f),
            ],
            s,
        );
    }
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,

    window: Window,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,

    egui_ctx: egui::Context,
    gpu: Option<Gpu>,
    content: Content,

    // logical size we render at, integer scale, and the source aspect (w/h).
    width: u32,
    height: u32,
    scale: u32,
    aspect: Option<f32>,
    /// Remembered expanded size, restored when un-collapsing.
    expanded: (u32, u32),
    collapsed: bool,

    hovered: bool,
    pointer_pos: egui::Pos2,
    accent: u32,
    gone_since: Option<Instant>,

    /// Window opacity (1.0 = opaque); adjusted with the wheel or `+`/`-`.
    opacity: f32,
    /// Freeze the live feed on the last frame (Space toggles).
    frozen: bool,

    start: Instant,
    closing: bool,
    configured: bool,
}

/// Run the mirror until the source closes or the user quits. `label` is shown in
/// the hover toolbar; `icon` is the app icon for the collapsed badge.
pub fn run(
    identifier: String,
    label: String,
    icon: Option<(u32, u32, Vec<u8>)>,
) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let lh = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(lh.clone())
        .map_err(|e| anyhow::anyhow!("calloop wayland source: {e}"))?;

    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|e| anyhow::anyhow!("wl_compositor: {e}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|e| anyhow::anyhow!("xdg-shell absent: {e}"))?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_app_id("wlr-pip");
    window.set_title(label.clone());
    window.set_min_size(Some((MIN_W, (MIN_W * 9 / 16).max(1))));
    window.commit();

    // Capture thread streams frames over a calloop channel; we repaint on each.
    let (tx, ch): (_, Channel<PipMsg>) = channel();
    let id = identifier.clone();
    std::thread::spawn(move || crate::pip::capture_thread(id, move |m| tx.send(m).is_ok()));
    lh.insert_source(ch, |event, _, state: &mut State| {
        if let ChannelEvent::Msg(m) = event {
            state.on_msg(m);
        }
    })
    .map_err(|e| anyhow::anyhow!("calloop channel source: {e}"))?;

    let theme = Theme::load();
    let egui_ctx = egui::Context::default();
    theme.apply(&egui_ctx);
    let icon_tex = icon.map(|(w, h, rgba)| {
        let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        egui_ctx.load_texture("pip-icon", img, egui::TextureOptions::LINEAR)
    });

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        window,
        seat: None,
        keyboard: None,
        pointer: None,
        egui_ctx,
        gpu: None,
        content: Content {
            tex: None,
            native: None,
            icon: icon_tex,
            pending: Vec::new(),
            gone: false,
            label,
            theme,
        },
        width: DEFAULT_W,
        height: DEFAULT_W * 9 / 16,
        scale: 1,
        aspect: None,
        expanded: (DEFAULT_W, DEFAULT_W * 9 / 16),
        collapsed: false,
        hovered: false,
        pointer_pos: egui::Pos2::ZERO,
        accent: 0,
        gone_since: None,
        opacity: 1.0,
        frozen: false,
        start: Instant::now(),
        closing: false,
        configured: false,
    };

    while !state.closing {
        event_loop.dispatch(Duration::from_millis(400), &mut state)?;
        if let Some(t) = state.gone_since {
            if t.elapsed() >= GONE_LINGER {
                break;
            }
        }
    }
    Ok(())
}

impl State {
    /// A capture message arrived (host context): note source size / demise, queue
    /// the frame, restore from the badge on activity, and repaint.
    fn on_msg(&mut self, m: PipMsg) {
        match &m {
            PipMsg::Gone => {
                self.gone_since.get_or_insert(Instant::now());
                self.content.pending.push(m);
                self.redraw();
                return;
            }
            PipMsg::Shm { w, h, .. } => self.on_source_size(*w as u32, *h as u32),
            PipMsg::Dmabuf { frame } => self.on_source_size(frame.width, frame.height),
        }
        // Frozen: keep showing the last frame, drop incoming ones (the dropped
        // dma-buf fd closes here). Source-size tracking above still runs so a later
        // unfreeze keeps the right aspect.
        if self.frozen {
            return;
        }
        // Activity while collapsed pops the tile back open ("notify me on change").
        if self.collapsed {
            self.set_collapsed(false);
            self.accent = ACCENT_FRAMES;
        }
        self.content.pending.push(m);
        self.redraw();
    }

    /// Learn (or update) the source aspect ratio and, when expanded, keep the
    /// tile's height matching it.
    fn on_source_size(&mut self, sw: u32, sh: u32) {
        if sw == 0 || sh == 0 {
            return;
        }
        let a = sw as f32 / sh as f32;
        let first = self.aspect.is_none();
        self.aspect = Some(a);
        if !self.collapsed && (first || (self.height as f32 - self.width as f32 / a).abs() > 1.0) {
            let h = ((self.width as f32 / a).round() as u32).max(1);
            self.apply_size(self.width, h);
            self.expanded = (self.width, h);
        }
    }

    /// Snap a width/height to the source aspect ratio (width is authoritative).
    fn snap(&self, w: u32, h: u32) -> (u32, u32) {
        match self.aspect {
            Some(a) if a > 0.0 => (w.max(MIN_W), ((w as f32 / a).round() as u32).max(1)),
            _ => (w.max(MIN_W), h.max(1)),
        }
    }

    /// Set the render size and resize the EGL window to match (the floating
    /// compositor follows our buffer size).
    fn apply_size(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        if let Some(gpu) = &self.gpu {
            gpu.resize((w * self.scale) as i32, (h * self.scale) as i32);
        }
    }

    fn set_collapsed(&mut self, collapsed: bool) {
        if collapsed == self.collapsed {
            return;
        }
        self.collapsed = collapsed;
        if collapsed {
            self.expanded = (self.width, self.height);
            self.window.set_min_size(Some((BADGE, BADGE)));
            self.window.set_max_size(Some((BADGE, BADGE)));
            self.apply_size(BADGE, BADGE);
        } else {
            self.window
                .set_min_size(Some((MIN_W, (MIN_W * 9 / 16).max(1))));
            self.window.set_max_size(None);
            let (w, h) = self.expanded;
            self.apply_size(w, h);
        }
        self.window.commit();
        self.redraw();
    }

    fn ensure_gpu(&mut self, conn: &Connection) {
        if self.gpu.is_some() || self.width == 0 {
            return;
        }
        let (pw, ph) = (
            (self.width * self.scale) as i32,
            (self.height * self.scale) as i32,
        );
        self.gpu = Some(Gpu::new(conn, self.window.wl_surface(), pw, ph));
    }

    /// Render one frame (no frame-callback chain: driven by capture/interaction).
    fn redraw(&mut self) {
        if !self.configured {
            return;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.width as f32, self.height as f32),
            )),
            time: Some(self.start.elapsed().as_secs_f64()),
            focused: true,
            ..Default::default()
        };
        let opacity = self.opacity;
        let backdrop = {
            let c = self.content.theme.thumb.to_normalized_gamma_f32();
            [c[0], c[1], c[2], opacity]
        };
        let size = (self.width as f32, self.height as f32);
        let (collapsed, hovered, accent) = (self.collapsed, self.hovered, self.accent > 0);
        let frozen = self.frozen;
        let content = &mut self.content;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.render(
            &self.egui_ctx,
            raw_input,
            self.scale as f32,
            (pw, ph),
            backdrop,
            |ctx, imp| {
                content.pump(ctx, imp);
                content.draw(ctx, size, collapsed, hovered, accent, frozen, opacity);
            },
        );
        if self.accent > 0 {
            self.accent -= 1;
        }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.scale = new_factor.max(1) as u32;
        self.window.wl_surface().set_buffer_scale(new_factor.max(1));
        if let Some(gpu) = &self.gpu {
            gpu.resize(
                (self.width * self.scale) as i32,
                (self.height * self.scale) as i32,
            );
        }
        self.redraw();
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        // Driven by capture frames instead; nothing to do.
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

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.closing = true;
    }

    fn configure(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        if let (Some(w), Some(h)) = configure.new_size {
            let (w, h) = if self.collapsed {
                (BADGE, BADGE)
            } else {
                self.snap(w.get(), h.get())
            };
            self.apply_size(w, h);
            if !self.collapsed {
                self.expanded = (w, h);
            }
        }
        self.ensure_gpu(conn);
        self.configured = true;
        self.redraw();
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
        self.seat = Some(seat);
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
        self.on_key(event.keysym);
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
        let Some(seat) = self.seat.clone() else {
            return;
        };
        for e in events {
            let pos = egui::pos2(e.position.0 as f32, e.position.1 as f32);
            match e.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_pos = pos;
                    self.hovered = true;
                    self.redraw();
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_pos = pos;
                }
                PointerEventKind::Leave { .. } => {
                    self.hovered = false;
                    self.redraw();
                }
                // 0x110 == BTN_LEFT.
                PointerEventKind::Press {
                    button: 0x110,
                    serial,
                    ..
                } => {
                    self.pointer_pos = pos;
                    self.on_press(&seat, serial);
                }
                // Wheel adjusts opacity (up = more opaque).
                PointerEventKind::Axis { vertical, .. } if vertical.absolute != 0.0 => {
                    let step = if vertical.absolute < 0.0 { 0.1 } else { -0.1 };
                    self.set_opacity(self.opacity + step);
                }
                _ => {}
            }
        }
    }
}

impl State {
    /// Left-button press: hit-test the toolbar / grip / body and act.
    fn on_press(&mut self, seat: &wl_seat::WlSeat, serial: u32) {
        if self.collapsed {
            self.set_collapsed(false);
            return;
        }
        let (w, h) = (self.width as f32, self.height as f32);
        let (close, collapse) = toolbar_rects(w);
        let p = self.pointer_pos;
        if self.hovered && close.contains(p) {
            self.closing = true;
        } else if self.hovered && collapse.contains(p) {
            self.set_collapsed(true);
        } else if self.hovered && grip_rect(w, h).contains(p) {
            self.window
                .xdg_toplevel()
                .resize(seat, serial, ResizeEdge::BottomRight);
        } else {
            self.window.xdg_toplevel()._move(seat, serial);
        }
    }

    /// Keyboard shortcuts (the window must hold keyboard focus — click it first).
    fn on_key(&mut self, key: Keysym) {
        match key {
            Keysym::Escape | Keysym::q => self.closing = true,
            Keysym::space => {
                self.frozen = !self.frozen;
                self.redraw();
            }
            Keysym::c => self.set_collapsed(!self.collapsed),
            // Opacity: `+`/`=` more opaque, `-` more transparent.
            Keysym::plus | Keysym::equal | Keysym::KP_Add => self.set_opacity(self.opacity + 0.1),
            Keysym::minus | Keysym::KP_Subtract => self.set_opacity(self.opacity - 0.1),
            Keysym::r => self.repick(),
            _ => {}
        }
    }

    fn set_opacity(&mut self, o: f32) {
        let o = o.clamp(0.2, 1.0);
        if (o - self.opacity).abs() > f32::EPSILON {
            self.opacity = o;
            self.redraw();
        }
    }

    /// Re-pick the mirrored window: launch a fresh `wlr-pip` (which runs the
    /// chooser) and close this tile. Simpler and more robust than swapping the
    /// capture thread in place, and the compositor controls position anyway.
    fn repick(&mut self) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
        self.closing = true;
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
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_registry!(State);
