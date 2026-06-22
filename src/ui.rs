//! egui front-end: a grid of live thumbnails. Capture happens on a dedicated
//! thread (it owns the non-`Send` Wayland client) and streams downscaled
//! thumbnails to the UI over a channel, so the window opens instantly and fills
//! in. Toplevel capture is occlusion-independent, so showing our own window
//! first is fine.

use crate::theme::Theme;
use crate::wl;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shared slot where the chosen source lands; read by `main` after the window closes.
pub type Outcome = Arc<Mutex<Option<Selection>>>;

/// The picked source, carrying the full identity so `main` can act on it per mode:
/// print `token` (portal), activate by `app_id`+`title` (`--switch`), or — later —
/// mirror it live by `identifier` (PiP). See the pip-mode design notes.
#[derive(Clone)]
pub struct Selection {
    pub token: String, // portal stdout contract: "Window: <id>" / "Monitor: <name>"
    pub is_window: bool,
    // Reserved for the upcoming PiP mode (mirror by identifier); not yet consumed.
    #[allow(dead_code)]
    pub identifier: String, // ext-foreign-toplevel identifier (capture / PiP); empty for outputs
    pub app_id: String, // for zwlr activation / PiP labelling
    pub title: String,  // window title
    /// Ordinal among windows sharing this (app_id, title), in creation order, to
    /// disambiguate identical windows when correlating to zwlr handles.
    pub dup_index: usize,
}

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
    /// Raw window identity (windows only), for activation / PiP.
    pub app_id: String,
    pub win_title: String,
    /// Ordinal among windows sharing this (app_id, title), in creation order.
    pub dup_index: usize,
}

impl Source {
    /// The identity to hand back to `main` when this source is picked.
    fn selection(&self) -> Selection {
        Selection {
            token: self.token.clone(),
            is_window: self.is_window,
            identifier: if self.is_window {
                self.key.clone()
            } else {
                String::new()
            },
            app_id: self.app_id.clone(),
            title: self.win_title.clone(),
            dup_index: self.dup_index,
        }
    }
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
    /// A GPU dma-buf frame to import zero-copy as a GL texture (host-side).
    Dmabuf {
        key: String,
        frame: wl::DmabufFrame,
    },
    /// A source disappeared (window closed): drop its cached textures.
    Drop {
        key: String,
    },
}

/// Host-side importer for GPU dma-buf frames. The layer-shell host owns the GL
/// context, so it (not the toolkit-agnostic [`App`]) turns a dma-buf into a
/// drawable egui texture. Returns the texture id + source pixel size.
pub trait DmabufImporter {
    fn import(
        &mut self,
        key: &str,
        frame: wl::DmabufFrame,
    ) -> Option<(egui::TextureId, egui::Vec2)>;
    /// Release any GPU resources cached for a source that went away.
    fn forget(&mut self, key: &str);
}

/// Per-round time budget = upper bound on the refresh rate (capture is
/// damage-driven, so this is a ceiling, not a forced rate). shm pays a full CPU
/// readback+convert+downscale+upload per frame, so we keep it modest (~6 fps);
/// the GPU dma-buf path is near-free per frame, so it runs much faster (~30 fps).
#[cfg(not(feature = "gpu"))]
const ROUND_SHM: Duration = Duration::from_millis(160);
#[cfg(feature = "gpu")]
const ROUND_GPU: Duration = Duration::from_millis(33);

/// The round budget for this run: faster when built with the near-free GPU path.
fn round_budget() -> Duration {
    #[cfg(feature = "gpu")]
    {
        ROUND_GPU
    }
    #[cfg(not(feature = "gpu"))]
    {
        ROUND_SHM
    }
}

/// A source paired with what it takes to (re)open its capture session.
enum Capturable {
    Output(wl::Output),
    Window(wl::Toplevel),
}

/// Capture thread body: a continuous loop. Each round it refreshes the window
/// list, opens persistent sessions for new sources, captures one frame from every
/// session, and streams thumbnails — so tiles show *live* content. Sessions are
/// reused across rounds (the buffer is not reallocated unless a window resizes).
///
/// Toplevels with an empty app-id are captured but marked `is_system`, so the UI
/// can hide them by default and reveal them on demand. The loop exits when the UI
/// drops the channel.
// SessionId (a wayland ObjectId) is used as a map key: its interior-mutable
// "alive" flag is not part of Hash/Eq, so it is a sound key.
#[allow(clippy::mutable_key_type)]
pub fn capture_thread(tx: Sender<Msg>) {
    let mut client = match wl::Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", crate::tr!("error", error = format!("{e:#}")));
            return;
        }
    };

    // Per-source bookkeeping that must persist across rounds.
    let mut sessions: HashMap<String, wl::SessionId> = HashMap::new(); // source key -> session
    let mut by_id: HashMap<wl::SessionId, String> = HashMap::new(); // reverse, to label frames
    let mut iconed: HashSet<String> = HashSet::new();
    let mut last_keys: Vec<String> = Vec::new();
    let budget = round_budget();

    'outer: loop {
        // Pick up newly-opened / closed windows since the last round.
        if client.refresh().is_err() {
            break;
        }

        // Build the current source set in a stable, predictable order:
        // outputs first (sorted by name), then windows (by app-id, then title).
        let mut outputs = client.outputs().to_vec();
        outputs.sort_by(|a, b| a.name.cmp(&b.name));
        let mut windows = client.toplevels().to_vec();
        windows.sort_by(|a, b| {
            a.app_id
                .to_lowercase()
                .cmp(&b.app_id.to_lowercase())
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });

        let mut current: Vec<(Source, Capturable)> = Vec::new();
        for o in &outputs {
            current.push((output_source(o), Capturable::Output(o.clone())));
        }
        // Number windows that share an (app_id, title); the stable sort keeps
        // them in creation order, matching zwlr's enumeration for activation.
        let mut dup: HashMap<(String, String), usize> = HashMap::new();
        for w in &windows {
            let e = dup.entry((w.app_id.clone(), w.title.clone())).or_insert(0);
            let dup_index = *e;
            *e += 1;
            current.push((window_source(w, dup_index), Capturable::Window(w.clone())));
        }
        let keys: Vec<String> = current.iter().map(|(s, _)| s.key.clone()).collect();

        // Announce the source list only when it actually changes (set or order).
        if keys != last_keys {
            let srcs: Vec<Source> = current.iter().map(|(s, _)| s.clone()).collect();
            if tx.send(Msg::Sources(srcs)).is_err() {
                break;
            }
            last_keys = keys.clone();
        }

        // Close sessions for windows that vanished and tell the UI to drop them.
        let present: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let gone: Vec<String> = sessions
            .keys()
            .filter(|k| !present.contains(k.as_str()))
            .cloned()
            .collect();
        for k in gone {
            if let Some(id) = sessions.remove(&k) {
                by_id.remove(&id);
                client.close_session(&id);
            }
            iconed.remove(&k);
            if tx.send(Msg::Drop { key: k }).is_err() {
                break 'outer;
            }
        }

        // Open a session for every source we don't track yet.
        for (s, cap) in &current {
            if sessions.contains_key(&s.key) {
                continue;
            }
            let opened = match cap {
                Capturable::Output(o) => client.open_output_session(o),
                Capturable::Window(w) => client.open_toplevel_session(w),
            };
            if let Ok(id) = opened {
                sessions.insert(s.key.clone(), id.clone());
                by_id.insert(id, s.key.clone());
            }

            // App icon (cheap) once per window, so it's identifiable independently
            // of its thumbnail.
            if let Capturable::Window(w) = cap {
                if iconed.insert(s.key.clone()) {
                    if let Some(path) = crate::icons::resolve(&w.app_id) {
                        if let Some((iw, ih, rgba)) = crate::icons::load(&path, 32) {
                            if tx
                                .send(Msg::Icon {
                                    key: s.key.clone(),
                                    w: iw as usize,
                                    h: ih as usize,
                                    rgba,
                                })
                                .is_err()
                            {
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }

        // Drive all sessions for one round: this blocks up to the round budget
        // waiting for damage, so an idle desktop costs ~one syscall, while updating
        // windows stream frames. Only sources that produced new content come back.
        let (frames, failed) = client.poll(budget);
        for (id, frame) in frames {
            let Some(key) = by_id.get(&id) else { continue };
            let msg = match frame {
                wl::Frame::Shm(img) => {
                    let (w, h, rgba) = thumbnail(img);
                    Msg::Thumb {
                        key: key.clone(),
                        w,
                        h,
                        rgba,
                    }
                }
                wl::Frame::Dmabuf(frame) => Msg::Dmabuf {
                    key: key.clone(),
                    frame,
                },
            };
            if tx.send(msg).is_err() {
                break 'outer;
            }
        }
        // Sessions the compositor stopped: drop them; if the window is still
        // listed we reopen it next round.
        for id in failed {
            if let Some(key) = by_id.remove(&id) {
                sessions.remove(&key);
            }
            client.close_session(&id);
        }
    }
}

/// Cheap content fingerprint of a frame (subsampled FNV-1a), to tell whether a
/// capture actually changed between rounds — used by the headless bench.
fn quick_hash(rgba: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    // Sample ~4096 bytes spread across the frame so large captures stay cheap.
    let step = (rgba.len() / 4096).max(1);
    let mut i = 0;
    while i < rgba.len() {
        h = (h ^ rgba[i] as u64).wrapping_mul(0x100000001b3);
        i += step;
    }
    (h ^ rgba.len() as u64).wrapping_mul(0x100000001b3)
}

/// Headless capture benchmark (debug): no overlay, no keyboard grab. Runs the
/// capture loop for `secs` seconds and reports, per source, how many frames were
/// captured and how many actually changed content (proof of "live").
#[allow(clippy::mutable_key_type)] // see capture_thread: ObjectId is a sound map key
pub fn bench_capture(secs: u64) {
    let mut client = match wl::Client::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bench: connexion échouée: {e:#}");
            return;
        }
    };
    let mut sessions: HashMap<String, wl::SessionId> = HashMap::new();
    let mut by_id: HashMap<wl::SessionId, String> = HashMap::new();
    // key -> (frames, changed, last_hash)
    let mut stats: HashMap<String, (u32, u32, u64)> = HashMap::new();

    let _ = client.refresh();
    eprintln!(
        "bench: {} sortie(s), {} fenêtre(s) ; capture pendant {secs}s…",
        client.outputs().len(),
        client.toplevels().len()
    );

    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut rounds = 0u32;
    while Instant::now() < deadline {
        let _ = client.refresh();
        let mut outputs = client.outputs().to_vec();
        outputs.sort_by(|a, b| a.name.cmp(&b.name));
        let windows = client.toplevels().to_vec();

        let mut items: Vec<(String, Capturable)> = Vec::new();
        for o in &outputs {
            items.push((format!("out:{}", o.name), Capturable::Output(o.clone())));
        }
        for w in &windows {
            items.push((w.identifier.clone(), Capturable::Window(w.clone())));
        }

        for (key, cap) in &items {
            if sessions.contains_key(key) {
                continue;
            }
            let opened = match cap {
                Capturable::Output(o) => client.open_output_session(o),
                Capturable::Window(w) => client.open_toplevel_session(w),
            };
            match opened {
                Ok(id) => {
                    sessions.insert(key.clone(), id.clone());
                    by_id.insert(id, key.clone());
                }
                Err(e) => eprintln!("bench: open {key}: {e:#}"),
            }
        }

        let (frames, failed) = client.poll(round_budget());
        for (id, frame) in frames {
            if let Some(key) = by_id.get(&id) {
                // shm carries pixels (hashable); dma-buf is GPU-only here, so we
                // just count it as a delivered frame (content lives on the GPU).
                let hash = match &frame {
                    wl::Frame::Shm(img) => quick_hash(&img.rgba),
                    wl::Frame::Dmabuf(_) => 0,
                };
                let e = stats.entry(key.clone()).or_insert((0, 0, hash));
                e.0 += 1;
                if e.0 > 1 && hash != 0 && e.2 != hash {
                    e.1 += 1;
                }
                e.2 = hash;
            }
        }
        for id in failed {
            if let Some(key) = by_id.remove(&id) {
                eprintln!("bench: session arrêtée {key}");
                sessions.remove(&key);
            }
            client.close_session(&id);
        }
        rounds += 1;
    }

    eprintln!("bench: {rounds} round(s) en {secs}s");
    let mut keys: Vec<_> = stats.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let (frames, changed, _) = stats[&k];
        eprintln!("  {k}: {frames} frames, {changed} changées");
    }
}

/// Build the grid entry for an output.
fn output_source(o: &wl::Output) -> Source {
    let title = crate::tr!("screen-label", name = o.name.clone());
    Source {
        key: format!("out:{}", o.name),
        token: format!("Monitor: {}", o.name),
        filter: format!("{} {}", title, o.name).to_lowercase(),
        title,
        subtitle: String::new(),
        is_window: false,
        is_system: false,
        app_id: String::new(),
        win_title: String::new(),
        dup_index: 0,
    }
}

/// Build the grid entry for a window. `dup_index` is its ordinal among windows
/// with the same (app_id, title), used to disambiguate identical windows.
fn window_source(w: &wl::Toplevel, dup_index: usize) -> Source {
    let is_system = w.app_id.is_empty();
    let (title, subtitle) = if is_system {
        (w.title.clone(), String::new())
    } else {
        (w.app_id.clone(), w.title.clone())
    };
    Source {
        key: w.identifier.clone(),
        token: format!("Window: {}", w.identifier),
        filter: format!("{} {}", w.app_id, w.title).to_lowercase(),
        title,
        subtitle,
        is_window: true,
        is_system,
        app_id: w.app_id.clone(),
        win_title: w.title.clone(),
        dup_index,
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
    /// GPU dma-buf thumbnails: egui texture id + source pixel size, imported by
    /// the host. Looked up before `textures` when drawing a tile.
    native: HashMap<String, (egui::TextureId, egui::Vec2)>,
    icons: HashMap<String, egui::TextureHandle>,
    filter: String,
    mode: Mode,
    show_system: bool,
    /// Fixed grid size (columns, rows), or `None` for an auto-fitting grid.
    grid: Option<(u32, u32)>,
    /// Exposé: full-screen mission-control layout instead of the centred card.
    expose: bool,
    /// Time (egui seconds) of the first exposé frame, to anchor the intro animation.
    expose_t0: Option<f32>,
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
        expose: bool,
        theme: Theme,
    ) -> Self {
        Self {
            rx,
            sources: Vec::new(),
            textures: HashMap::new(),
            native: HashMap::new(),
            icons: HashMap::new(),
            filter: String::new(),
            mode,
            show_system,
            grid,
            expose,
            expose_t0: None,
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

    fn choose(&mut self, sel: Selection) {
        *self.out.lock().unwrap() = Some(sel);
        self.closing = true;
    }

    fn pump(&mut self, ctx: &egui::Context, importer: &mut dyn DmabufImporter) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Sources(s) => self.sources = s,
                Msg::Thumb { key, w, h, rgba } if w > 0 && h > 0 => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    // Update the existing texture in place when we can (live frames
                    // stream in continuously), else allocate it the first time.
                    match self.textures.get_mut(&key) {
                        Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            let tex = ctx.load_texture(&key, img, egui::TextureOptions::LINEAR);
                            self.textures.insert(key, tex);
                        }
                    }
                }
                Msg::Dmabuf { key, frame } => {
                    // Imported by the host (it owns the GL context); the resulting
                    // texture samples the dma-buf directly (zero copy).
                    if let Some(tex) = importer.import(&key, frame) {
                        self.native.insert(key, tex);
                    }
                }
                Msg::Icon { key, w, h, rgba } if w > 0 && h > 0 => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex =
                        ctx.load_texture(format!("icon:{key}"), img, egui::TextureOptions::LINEAR);
                    self.icons.insert(key, tex);
                }
                Msg::Drop { key } => {
                    self.textures.remove(&key);
                    self.native.remove(&key);
                    self.icons.remove(&key);
                    importer.forget(&key);
                }
                Msg::Thumb { .. } | Msg::Icon { .. } => {}
            }
        }
    }

    /// The drawable thumbnail for a source: the GPU dma-buf texture if present,
    /// else the shm texture. Returns the egui texture id and source pixel size.
    fn thumb_tex(&self, key: &str) -> Option<(egui::TextureId, egui::Vec2)> {
        if let Some(&(id, size)) = self.native.get(key) {
            return Some((id, size));
        }
        self.textures.get(key).map(|t| (t.id(), t.size_vec2()))
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
        let mut c = self.theme.backdrop.to_normalized_gamma_f32();
        // Exposé covers the whole screen: dim almost to opaque so the real windows
        // behind are hidden (a client can't move them; this hides them instead).
        if self.expose {
            c[3] = c[3].max(0.96);
        }
        c
    }

    /// Build one egui frame. Toolkit-agnostic: the host loop drives it and checks
    /// [`App::closing`] afterwards. The host passes its dma-buf importer (it owns
    /// the GL context) so GPU frames can be turned into drawable textures.
    pub fn run_ui(&mut self, ctx: &egui::Context, importer: &mut dyn DmabufImporter) {
        self.pump(ctx, importer);
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
            if let Some(sel) = self.visible().get(self.selected).map(|s| s.selection()) {
                self.choose(sel);
            }
        }

        let mut chosen: Option<Selection> = None;
        if self.expose {
            chosen = self.render_expose(ctx);
            if let Some(sel) = chosen {
                self.choose(sel);
            }
            return;
        }

        // A centred card on the dimmed overlay backdrop. Its size is either fixed
        // to show exactly `grid` tiles, or a sensible default. Clicking the
        // backdrop cancels, like rofi.
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
                                        chosen = Some(s.selection());
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
        if let Some(sel) = chosen {
            self.choose(sel);
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

        if let Some((tex_id, ts)) = self.thumb_tex(&s.key) {
            // Contain (no crop): fit the texture inside img_rect, centred.
            let scale = (img_rect.width() / ts.x).min(img_rect.height() / ts.y);
            let size = ts * scale;
            let draw = egui::Rect::from_center_size(img_rect.center(), size);
            p.image(
                tex_id,
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

impl App {
    /// Exposé: a full-screen, mission-control-style layout over the dimmed
    /// backdrop. Tiles are placed in justified rows (variable heights, each row
    /// filling the width with aspect ratios preserved) and scaled to fit on
    /// screen. Returns the picked source, if any.
    fn render_expose(&mut self, ctx: &egui::Context) -> Option<Selection> {
        let area = ctx.screen_rect().shrink(24.0);
        let gap = 12.0;

        // Intro animation clock: anchor t0 to the first exposé frame so startup
        // latency doesn't eat the animation. (Computed before borrowing sources.)
        let now = ctx.input(|i| i.time) as f32;
        let elapsed = now - *self.expose_t0.get_or_insert(now);
        const ANIM: f32 = 0.28;

        let vis = self.visible();
        let items: Vec<(usize, f32)> = vis
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let aspect = self
                    .thumb_tex(&s.key)
                    .map(|(_, sz)| (sz.x / sz.y).clamp(0.3, 4.0))
                    .unwrap_or(16.0 / 9.0);
                (i, aspect)
            })
            .collect();
        let rects = expose_layout(&items, area, gap);

        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                for (i, rect) in &rects {
                    let s = vis[*i];
                    let resp =
                        ui.interact(*rect, ui.id().with(("expose", *i)), egui::Sense::click());
                    // Per-tile ease-out, slightly delayed by index.
                    let lt = ((elapsed - *i as f32 * 0.012) / ANIM).clamp(0.0, 1.0);
                    let ease = 1.0 - (1.0 - lt).powi(3);
                    let scaled = egui::Rect::from_center_size(
                        rect.center(),
                        rect.size() * (0.86 + 0.14 * ease),
                    );
                    self.paint_expose_tile(
                        ui,
                        s,
                        scaled,
                        *i == self.selected,
                        resp.hovered(),
                        ease,
                    );
                    if resp.clicked() {
                        chosen = Some(s.selection());
                    }
                }
            });

        // A press on empty space (no tile) cancels.
        let pressed_outside = ctx.input(|inp| {
            inp.pointer.any_pressed()
                && inp
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !rects.iter().any(|(_, r)| r.contains(pos)))
        });
        if pressed_outside {
            self.closing = true;
        }
        chosen
    }

    /// Paint one exposé tile: the live thumbnail filling `rect`, a translucent
    /// label strip with icon + name, and a selection/hover outline.
    fn paint_expose_tile(
        &self,
        ui: &egui::Ui,
        s: &Source,
        rect: egui::Rect,
        selected: bool,
        hovered: bool,
        a: f32, // intro-animation opacity (1.0 once settled)
    ) {
        let t = &self.theme;
        let p = ui.painter();
        let radius = 8.0;
        let fade = |c: egui::Color32| c.gamma_multiply(a);
        let white = egui::Color32::WHITE.gamma_multiply(a);
        p.rect_filled(rect, radius, fade(t.thumb));

        if let Some((tex_id, ts)) = self.thumb_tex(&s.key) {
            let scale = (rect.width() / ts.x).min(rect.height() / ts.y);
            let draw = egui::Rect::from_center_size(rect.center(), ts * scale);
            p.image(
                tex_id,
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                white,
            );
        } else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                crate::tr!("loading"),
                egui::FontId::proportional(18.0),
                fade(t.text_dim),
            );
        }

        // Translucent label strip at the bottom: icon + name.
        let strip_h = 24.0_f32.min(rect.height() * 0.3);
        let strip =
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - strip_h), rect.max);
        p.rect_filled(
            strip,
            0.0,
            egui::Color32::from_black_alpha(160).gamma_multiply(a),
        );
        let icon_sz = (strip_h - 8.0).max(10.0);
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(strip.left() + 6.0, strip.center().y - icon_sz / 2.0),
            egui::vec2(icon_sz, icon_sz),
        );
        if let Some(ic) = self.icons.get(&s.key) {
            let isz = ic.size_vec2();
            let sc = (icon_rect.width() / isz.x).min(icon_rect.height() / isz.y);
            let d = egui::Rect::from_center_size(icon_rect.center(), isz * sc);
            p.image(
                ic.id(),
                d,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                white,
            );
        } else {
            draw_window_glyph(p, icon_rect, fade(t.window_accent));
        }
        let label = if s.subtitle.is_empty() {
            s.title.clone()
        } else {
            format!("{} — {}", s.title, s.subtitle)
        };
        let tx = icon_rect.max.x + 6.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            label,
            egui::FontId::proportional(13.0),
            fade(t.text),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width((strip.right() - 6.0 - tx).max(0.0));
        let galley = ui.fonts(|f| f.layout_job(job));
        p.galley(
            egui::pos2(tx, strip.center().y - galley.size().y / 2.0),
            galley,
            fade(t.text),
        );

        let accent = if s.is_window {
            t.window_accent
        } else {
            t.screen_accent
        };
        let (sw, col) = if selected {
            (3.0, accent)
        } else if hovered {
            (2.0, accent)
        } else {
            (1.0, t.thumb)
        };
        p.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(sw, fade(col)),
            egui::StrokeKind::Inside,
        );
    }
}

/// Lay out `items` (index, aspect = w/h) as justified rows (each row fills the
/// width), sizing the tiles as large as possible while still fitting the height —
/// so the grid fills the screen (mission-control) instead of hugging the centre.
fn expose_layout(items: &[(usize, f32)], area: egui::Rect, gap: f32) -> Vec<(usize, egui::Rect)> {
    if items.is_empty() {
        return Vec::new();
    }
    let (aw, ah) = (area.width(), area.height());

    // Greedily pack items into rows at a trial row height `th`.
    let group = |th: f32| -> Vec<Vec<(usize, f32)>> {
        let mut rows: Vec<Vec<(usize, f32)>> = Vec::new();
        let mut cur: Vec<(usize, f32)> = Vec::new();
        let mut cur_w = 0.0;
        for &(idx, aspect) in items {
            let w = th * aspect;
            let extra = if cur.is_empty() { w } else { gap + w };
            if !cur.is_empty() && cur_w + extra > aw {
                rows.push(std::mem::take(&mut cur));
                cur.push((idx, aspect));
                cur_w = w;
            } else {
                cur.push((idx, aspect));
                cur_w += extra;
            }
        }
        if !cur.is_empty() {
            rows.push(cur);
        }
        rows
    };
    // Height of a row once justified to the full width (capped so a near-empty
    // last row doesn't balloon to a giant tile).
    let row_h = |row: &[(usize, f32)], th: f32| -> f32 {
        let sum_aspect: f32 = row.iter().map(|(_, a)| *a).sum();
        let total_gap = gap * (row.len() as f32 - 1.0);
        ((aw - total_gap) / sum_aspect).min(th * 1.5)
    };
    let total_h = |th: f32| -> f32 {
        let rows = group(th);
        rows.iter().map(|r| row_h(r, th)).sum::<f32>() + gap * (rows.len().saturating_sub(1) as f32)
    };

    // Bisect the trial row height to the largest value whose laid-out total still
    // fits the available height.
    let (mut lo, mut hi) = (30.0_f32, ah);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if total_h(mid) <= ah {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let rows = group(lo);
    let heights: Vec<f32> = rows.iter().map(|r| row_h(r, lo)).collect();
    let used_h = heights.iter().sum::<f32>() + gap * (rows.len().saturating_sub(1) as f32);

    let mut out = Vec::with_capacity(items.len());
    let mut y = area.top() + ((ah - used_h) * 0.5).max(0.0);
    for (row, &rh) in rows.iter().zip(heights.iter()) {
        let row_w = row.iter().map(|(_, a)| a * rh).sum::<f32>() + gap * (row.len() as f32 - 1.0);
        let mut x = area.left() + ((aw - row_w) * 0.5).max(0.0);
        for &(idx, aspect) in row {
            let w = aspect * rh;
            out.push((
                idx,
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, rh)),
            ));
            x += w + gap;
        }
        y += rh + gap;
    }
    out
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
