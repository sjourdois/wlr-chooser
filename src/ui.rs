//! egui front-end: a grid of live thumbnails. Capture happens on a dedicated
//! thread (it owns the non-`Send` Wayland client) and streams downscaled
//! thumbnails to the UI over a channel, so the window opens instantly and fills
//! in. Toplevel capture is occlusion-independent, so showing our own window
//! first is fine.

use crate::theme::Theme;
use crate::wl;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Shared slot where the chosen token lands; read by `main` after the window closes.
pub type Outcome = Arc<Mutex<Option<String>>>;

pub const APP_ID: &str = "wlr-chooser";
const TILE_W: f32 = 300.0; // reference tile size (aspect ratio for the thumbnail)
const TILE_H: f32 = 180.0;
const MIN_TILE: f32 = 280.0; // tiles grow from here to fill the row width
const GRID_GAP: f32 = 10.0; // gap between tiles
const THUMB_MAX: u32 = 480;

/// Which kinds of source to show. Set by `--windows`/`--outputs`/`--both` and
/// switchable at runtime via the tab bar.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    All,
    Windows,
    Outputs,
}

/// One pickable source, as shown in the grid.
#[derive(Clone)]
pub struct Source {
    pub key: String,   // texture key (window identifier or "out:<name>")
    pub token: String, // what we print on stdout: "Window: …" / "Monitor: …"
    pub title: String,
    pub subtitle: String,
    pub filter: String,
    pub is_window: bool,
    pub is_system: bool, // window with an empty app-id (hidden unless asked)
}

/// Messages from the capture thread to the UI.
pub enum Msg {
    Sources(Vec<Source>),
    Thumb {
        key: String,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    },
    Icon {
        key: String,
        w: usize,
        h: usize,
        rgba: Vec<u8>,
    },
}

/// Capture thread body: enumerate, announce the sources, then capture each
/// (outputs first so they are as clean as possible) and stream thumbnails.
///
/// Toplevels with an empty app-id are captured but marked `is_system`, so the UI
/// can hide them by default and reveal them on demand.
pub fn capture_thread(tx: Sender<Msg>) {
    let mut client = match wl::Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", crate::tr!("error", error = format!("{e:#}")));
            return;
        }
    };

    let mut outputs = client.outputs().to_vec();
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut windows: Vec<wl::Toplevel> = client.toplevels().to_vec();
    // Stable, predictable order: by app-id, then window title.
    windows.sort_by(|a, b| {
        a.app_id
            .to_lowercase()
            .cmp(&b.app_id.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    let mut sources = Vec::new();
    for o in &outputs {
        let title = crate::tr!("screen-label", name = o.name.clone());
        sources.push(Source {
            key: format!("out:{}", o.name),
            token: format!("Monitor: {}", o.name),
            filter: format!("{} {}", title, o.name).to_lowercase(),
            title,
            subtitle: String::new(),
            is_window: false,
            is_system: false,
        });
    }
    for w in &windows {
        let is_system = w.app_id.is_empty();
        let (title, subtitle) = if is_system {
            (w.title.clone(), String::new())
        } else {
            (w.app_id.clone(), w.title.clone())
        };
        sources.push(Source {
            key: w.identifier.clone(),
            token: format!("Window: {}", w.identifier),
            filter: format!("{} {}", w.app_id, w.title).to_lowercase(),
            title,
            subtitle,
            is_window: true,
            is_system,
        });
    }
    if tx.send(Msg::Sources(sources)).is_err() {
        return;
    }

    // App icons (cheap) first, so windows are identifiable before captures land.
    for w in &windows {
        if let Some(path) = crate::icons::resolve(&w.app_id) {
            if let Some((iw, ih, rgba)) = crate::icons::load(&path, 32) {
                let _ = tx.send(Msg::Icon {
                    key: w.identifier.clone(),
                    w: iw as usize,
                    h: ih as usize,
                    rgba,
                });
            }
        }
    }

    let send_thumb = |tx: &Sender<Msg>, key: String, img: wl::CapturedImage| {
        let (w, h, rgba) = thumbnail(img);
        let _ = tx.send(Msg::Thumb { key, w, h, rgba });
    };

    for o in &outputs {
        if let Ok(img) = client.capture_output(o) {
            send_thumb(&tx, format!("out:{}", o.name), img);
        }
    }
    for w in &windows {
        if let Ok(img) = client.capture_toplevel(w) {
            send_thumb(&tx, w.identifier.clone(), img);
        }
    }
}

/// Downscale a capture to a thumbnail (max side `THUMB_MAX`), never upscaling.
fn thumbnail(img: wl::CapturedImage) -> (usize, usize, Vec<u8>) {
    let (w, h) = (img.width, img.height);
    let scale = (THUMB_MAX as f32 / w as f32)
        .min(THUMB_MAX as f32 / h as f32)
        .min(1.0);
    let src = match image::RgbaImage::from_raw(w, h, img.rgba) {
        Some(s) => s,
        None => return (0, 0, Vec::new()),
    };
    if scale >= 0.999 {
        return (w as usize, h as usize, src.into_raw());
    }
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(1);
    let small = image::imageops::thumbnail(&src, nw, nh);
    (
        small.width() as usize,
        small.height() as usize,
        small.into_raw(),
    )
}

pub struct App {
    rx: Receiver<Msg>,
    sources: Vec<Source>,
    textures: HashMap<String, egui::TextureHandle>,
    icons: HashMap<String, egui::TextureHandle>,
    filter: String,
    mode: Mode,
    show_system: bool,
    /// Fixed grid size (columns, rows), or `None` for an auto-fitting grid.
    grid: Option<(u32, u32)>,
    /// Selected index into the *visible* list, for keyboard navigation.
    selected: usize,
    /// Focus the filter field on the first frame.
    focus_filter: bool,
    /// Set once a choice is made or the picker is cancelled; the host loop exits.
    closing: bool,
    out: Outcome,
    theme: Theme,
}

impl App {
    pub fn new(
        rx: Receiver<Msg>,
        out: Outcome,
        mode: Mode,
        show_system: bool,
        grid: Option<(u32, u32)>,
        theme: Theme,
    ) -> Self {
        Self {
            rx,
            sources: Vec::new(),
            textures: HashMap::new(),
            icons: HashMap::new(),
            filter: String::new(),
            mode,
            show_system,
            grid,
            selected: 0,
            focus_filter: true,
            closing: false,
            out,
            theme,
        }
    }

    /// True once a selection or cancellation happened; the host loop should exit.
    pub fn closing(&self) -> bool {
        self.closing
    }

    /// Cancel without a selection (e.g. the compositor closed the surface).
    pub fn cancel(&mut self) {
        self.closing = true;
    }

    /// Install the palette into an egui context (host loops own the context).
    pub fn apply_theme(&self, ctx: &egui::Context) {
        self.theme.apply(ctx);
    }

    fn choose(&mut self, token: String) {
        *self.out.lock().unwrap() = Some(token);
        self.closing = true;
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Sources(s) => self.sources = s,
                Msg::Thumb { key, w, h, rgba } if w > 0 && h > 0 => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture(&key, img, egui::TextureOptions::LINEAR);
                    self.textures.insert(key, tex);
                }
                Msg::Icon { key, w, h, rgba } if w > 0 && h > 0 => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex =
                        ctx.load_texture(format!("icon:{key}"), img, egui::TextureOptions::LINEAR);
                    self.icons.insert(key, tex);
                }
                Msg::Thumb { .. } | Msg::Icon { .. } => {}
            }
        }
    }

    fn visible(&self) -> Vec<&Source> {
        let f = self.filter.to_lowercase();
        self.sources
            .iter()
            .filter(|s| self.show_system || !s.is_system)
            .filter(|s| match self.mode {
                Mode::All => true,
                Mode::Windows => s.is_window,
                Mode::Outputs => !s.is_window,
            })
            .filter(|s| f.is_empty() || s.filter.contains(&f))
            .collect()
    }

    /// Whether any captured source is a system window (to decide if we show the
    /// "show system windows" toggle).
    fn has_system(&self) -> bool {
        self.sources.iter().any(|s| s.is_system)
    }
}

impl App {
    /// GL clear colour: the transparent, dimmed backdrop behind the card (rofi-like).
    pub fn backdrop(&self) -> [f32; 4] {
        self.theme.backdrop.to_normalized_gamma_f32()
    }

    /// Build one egui frame. Toolkit-agnostic: the host loop drives it and checks
    /// [`App::closing`] afterwards.
    pub fn run_ui(&mut self, ctx: &egui::Context) {
        self.pump(ctx);
        ctx.request_repaint(); // keep draining the channel while captures stream in

        // Keyboard (read states first; don't call ctx methods inside ctx.input).
        let vis_len = self.visible().len();
        let (esc, next, prev, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if esc {
            self.closing = true;
        }
        if vis_len > 0 {
            if next {
                self.selected = (self.selected + 1) % vis_len;
            }
            if prev {
                self.selected = (self.selected + vis_len - 1) % vis_len;
            }
        }
        if enter {
            if let Some(s) = self.visible().get(self.selected) {
                let t = s.token.clone();
                self.choose(t);
            }
        }

        // A centred card on the dimmed overlay backdrop. Its size is either fixed
        // to show exactly `grid` tiles, or a sensible default. Clicking the
        // backdrop cancels, like rofi.
        let mut chosen: Option<String> = None;
        let screen = ctx.screen_rect();
        let forced_cols = self.grid.map(|(c, _)| c as usize);
        let (cw, ch) = match self.grid {
            Some((cols, rows)) => {
                let (cols, rows) = (cols as f32, rows as f32);
                let bar = 14.0; // scrollbar gutter
                let tile_h = MIN_TILE * (TILE_H / TILE_W) + 26.0;
                let inner_w = cols * MIN_TILE + (cols - 1.0) * GRID_GAP + bar;
                let inner_h = 78.0 + rows * tile_h + (rows - 1.0) * GRID_GAP; // 78 = header
                (inner_w + 24.0, inner_h + 24.0) // + card inner margin (12 each side)
            }
            None => (1000.0, 760.0),
        };
        let w = cw.min(screen.width() - 24.0);
        let h = ch.min(screen.height() - 24.0);
        let card_rect = egui::Rect::from_center_size(screen.center(), egui::vec2(w, h));
        let radius = 12.0;

        egui::Window::new("wlr-chooser-card")
            .title_bar(false)
            .resizable(false)
            .fixed_rect(card_rect)
            .frame(
                egui::Frame::new()
                    .fill(self.theme.card)
                    .corner_radius(radius)
                    .inner_margin(12.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let before = self.mode;
                    ui.selectable_value(&mut self.mode, Mode::All, crate::tr!("tab-all"));
                    ui.selectable_value(&mut self.mode, Mode::Windows, crate::tr!("tab-windows"));
                    ui.selectable_value(&mut self.mode, Mode::Outputs, crate::tr!("tab-outputs"));
                    if self.mode != before {
                        self.selected = 0;
                    }
                    // Reveal system windows (empty app-id) only when some exist.
                    if self.has_system() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .checkbox(&mut self.show_system, crate::tr!("show-system"))
                                .changed()
                            {
                                self.selected = 0;
                            }
                        });
                    }
                });
                ui.add_space(6.0);
                let te = egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(crate::tr!("filter-hint"))
                    .desired_width(f32::INFINITY);
                let resp = ui.add(te);
                if resp.changed() {
                    self.selected = 0;
                }
                if self.focus_filter {
                    resp.request_focus(); // type-to-filter immediately
                    self.focus_filter = false;
                }
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Grid: either a forced column count (--grid) or as many as
                        // fit. Tiles fill the row exactly; reserve the scrollbar gutter
                        // so the last column isn't hidden by it.
                        let gap = GRID_GAP;
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        let bar =
                            ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
                        let avail = ui.available_width() - bar;
                        let cols = forced_cols
                            .unwrap_or_else(|| ((avail + gap) / (MIN_TILE + gap)).floor() as usize)
                            .max(1);
                        let tile_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                        let visible = self.visible();
                        let mut idx = 0;
                        for chunk in visible.chunks(cols) {
                            ui.horizontal(|ui| {
                                for s in chunk {
                                    if self.tile(ui, s, idx == self.selected, tile_w) {
                                        chosen = Some(s.token.clone());
                                    }
                                    idx += 1;
                                }
                            });
                        }
                    });
            });

        // Click on the backdrop cancels, like rofi (works in both modes).
        let bg_click = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pos| !card_rect.contains(pos))
        });
        if bg_click {
            self.closing = true;
        }
        if let Some(tok) = chosen {
            self.choose(tok);
        }
    }
}

impl App {
    /// Draw one tile of width `w`; returns true if it was clicked.
    fn tile(&self, ui: &mut egui::Ui, s: &Source, selected: bool, w: f32) -> bool {
        let thumb_h = w * (TILE_H / TILE_W); // keep the 300:180 thumbnail aspect
        let desired = egui::vec2(w, thumb_h + 26.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp.clicked();
        }
        let t = &self.theme;
        let p = ui.painter();
        let bg = if selected {
            t.tile_selected
        } else if resp.hovered() {
            t.tile_hover
        } else {
            t.tile
        };
        p.rect_filled(rect, 8.0, bg);

        // Coloured outline distinguishing screens (screen_accent) from windows
        // (window_accent) at a glance.
        let accent = if s.is_window {
            t.window_accent
        } else {
            t.screen_accent
        };
        p.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(if selected { 3.0 } else { 2.0 }, accent),
            egui::StrokeKind::Inside,
        );

        let pad = 6.0;
        let img_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(pad, pad),
            egui::vec2(w - 2.0 * pad, thumb_h - 2.0 * pad),
        );
        p.rect_filled(img_rect, 4.0, t.thumb);

        if let Some(tex) = self.textures.get(&s.key) {
            // Contain (no crop): fit the texture inside img_rect, centred.
            let ts = tex.size_vec2();
            let scale = (img_rect.width() / ts.x).min(img_rect.height() / ts.y);
            let size = ts * scale;
            let draw = egui::Rect::from_center_size(img_rect.center(), size);
            p.image(
                tex.id(),
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            let placeholder = if s.is_window {
                crate::tr!("loading")
            } else {
                s.title.clone()
            };
            p.text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                placeholder,
                egui::FontId::proportional(20.0),
                t.text_dim,
            );
        }

        // Label row: a type-distinguishing icon, then the name.
        let icon_sz = 16.0;
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 8.0, rect.max.y - 21.0),
            egui::vec2(icon_sz, icon_sz),
        );
        if !s.is_window {
            draw_monitor_glyph(p, icon_rect, t.screen_accent);
        } else if let Some(ic) = self.icons.get(&s.key) {
            let ts = ic.size_vec2();
            let scale = (icon_rect.width() / ts.x).min(icon_rect.height() / ts.y);
            let draw = egui::Rect::from_center_size(icon_rect.center(), ts * scale);
            p.image(
                ic.id(),
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            draw_window_glyph(p, icon_rect, t.window_accent);
        }

        let text_x = icon_rect.max.x + 6.0;
        let label = if s.subtitle.is_empty() {
            s.title.clone()
        } else {
            format!("{} — {}", s.title, s.subtitle)
        };
        let mut job = egui::text::LayoutJob::simple_singleline(
            label,
            egui::FontId::proportional(13.0),
            t.text,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.max.x - 6.0 - text_x);
        let galley = ui.fonts(|f| f.layout_job(job));
        p.galley(
            egui::pos2(text_x, rect.max.y - 20.0),
            galley,
            egui::Color32::PLACEHOLDER,
        );

        resp.clicked()
    }
}

/// A small monitor glyph marking *output* tiles (so a full-screen window can't be
/// mistaken for a screen).
fn draw_monitor_glyph(p: &egui::Painter, r: egui::Rect, col: egui::Color32) {
    let screen = egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.max.y - r.height() * 0.28));
    p.rect_stroke(
        screen,
        2.0,
        egui::Stroke::new(1.6, col),
        egui::StrokeKind::Inside,
    );
    let cx = r.center().x;
    p.line_segment(
        [
            egui::pos2(cx - r.width() * 0.18, r.max.y),
            egui::pos2(cx + r.width() * 0.18, r.max.y),
        ],
        egui::Stroke::new(1.6, col),
    );
}

/// A generic window glyph for windows whose app icon could not be resolved.
fn draw_window_glyph(p: &egui::Painter, r: egui::Rect, col: egui::Color32) {
    p.rect_stroke(
        r,
        2.0,
        egui::Stroke::new(1.4, col),
        egui::StrokeKind::Inside,
    );
    p.line_segment(
        [
            egui::pos2(r.min.x, r.min.y + r.height() * 0.3),
            egui::pos2(r.max.x, r.min.y + r.height() * 0.3),
        ],
        egui::Stroke::new(1.4, col),
    );
}
