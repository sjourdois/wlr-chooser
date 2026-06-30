//! A StatusNotifierItem tray icon (the `tray` feature) showing the daemon's status and
//! a click-to-control menu, via [`ksni`].
//!
//! The icon reflects the current tool and draw state: a glyph for the active tool
//! (pen / rectangle / mask / arrow / text / eraser), drawn in the current stroke colour
//! while drawing and in grey when idle (click-through). Left-click toggles draw mode;
//! the menu exposes toggle / clear / undo / quit plus a Shortcuts submenu (the same key
//! legend the on-screen `h` overlay shows). Menu actions are sent as [`Cmd`]s over the
//! same calloop channel the control socket uses; the overlay pushes state changes back
//! with [`ksni::Handle::update`].
//!
//! Runs on its own thread (ksni owns a D-Bus connection); a no-op without a session bus.

use crate::model::{Color, Tool};
use crate::proto::Cmd;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, Tray};
use smithay_client_toolkit::reexports::calloop::channel::Sender;
use wlr_capture::tr;

/// The tray model. `active`/`color`/`tool` mirror the overlay; `tx` feeds menu actions.
/// `autostart` mirrors whether the XDG autostart entry exists (toggled from the menu,
/// independent of the daemon — see [`crate::autostart`]).
pub struct DrawTray {
    tx: Sender<Cmd>,
    pub active: bool,
    pub color: Color,
    pub tool: Tool,
    autostart: bool,
    /// Whether screen capture works here; gates the freeze/save shortcut rows (issue #1).
    capture_available: bool,
}

/// Side of the generated tray icon (px).
const ICON: i32 = 22;

impl Tray for DrawTray {
    fn id(&self) -> String {
        "wlr-draw".into()
    }

    fn title(&self) -> String {
        "wlr-draw".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![draw_icon(self.active, self.color, self.tool)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![],
            title: "wlr-draw".into(),
            description: if self.active {
                tr!("tray-status-drawing")
            } else {
                tr!("tray-status-idle")
            },
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Cmd::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: tr!("tray-toggle"),
                activate: Box::new(|t: &mut DrawTray| {
                    let _ = t.tx.send(Cmd::Toggle);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: tr!("tray-clear"),
                activate: Box::new(|t: &mut DrawTray| {
                    let _ = t.tx.send(Cmd::Clear);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: tr!("tray-undo"),
                activate: Box::new(|t: &mut DrawTray| {
                    let _ = t.tx.send(Cmd::Undo);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu {
                label: tr!("tray-shortcuts"),
                submenu: shortcut_items(self.capture_available),
                ..Default::default()
            }
            .into(),
            // A self-checked entry: the state lives in the label (☑ / ☐) rather than a
            // dbusmenu `toggle-type=checkmark`, because several SNI hosts (notably waybar
            // via libdbusmenu-gtk + a dark GTK theme) don't render the native check
            // indicator reliably. A glyph in the text is renderer-independent.
            StandardItem {
                label: format!(
                    "{}  {}",
                    if self.autostart { "☑" } else { "☐" },
                    tr!("tray-autostart")
                ),
                activate: Box::new(|t: &mut DrawTray| {
                    let want = !t.autostart;
                    // The toggle is a filesystem op (write / remove the desktop entry);
                    // only flip the state if it actually took effect.
                    match crate::autostart::set(want) {
                        Ok(()) => t.autostart = want,
                        Err(e) => eprintln!("wlr-draw: autostart toggle failed: {e}"),
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: tr!("tray-quit"),
                activate: Box::new(|t: &mut DrawTray| {
                    let _ = t.tx.send(Cmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The keyboard / gesture cheat-sheet (shared with the on-screen `h` legend) as disabled
/// (informational) menu entries, available straight from the tray.
fn shortcut_items(capture_available: bool) -> Vec<MenuItem<DrawTray>> {
    crate::overlay::shortcut_rows(capture_available)
        .into_iter()
        .map(|(key, desc)| {
            StandardItem {
                label: format!("{key}   —   {desc}"),
                enabled: false,
                ..Default::default()
            }
            .into()
        })
        .collect()
}

/// Start the tray on its own thread, returning a handle for status updates. `None` if
/// there is no D-Bus session bus (e.g. headless), so the daemon still runs.
pub fn spawn(
    tx: Sender<Cmd>,
    color: Color,
    tool: Tool,
    capture_available: bool,
) -> Option<Handle<DrawTray>> {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS")?; // no session bus → no tray
    // One-time auto-register on first ever run; harmless on later launches.
    crate::autostart::ensure_initialized();
    DrawTray {
        tx,
        active: false,
        color,
        tool,
        autostart: crate::autostart::is_enabled(),
        capture_available,
    }
    .spawn()
    .ok()
}

/// A tiny ARGB32 (network byte order) canvas for the icon glyphs.
struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            px: vec![0u8; (ICON * ICON * 4) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if (0..ICON).contains(&x) && (0..ICON).contains(&y) {
            let i = ((y * ICON + x) * 4) as usize;
            self.px[i..i + 4].copy_from_slice(&c);
        }
    }

    /// A filled disc of radius `r` (used as a thick pen for lines).
    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
        let r2 = r * r;
        let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
        let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                if dx * dx + dy * dy <= r2 {
                    self.put(x, y, c);
                }
            }
        }
    }

    /// A thick line, by stamping discs along it.
    fn line(&mut self, a: (f32, f32), b: (f32, f32), w: f32, c: [u8; 4]) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let steps = len.ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.disc(a.0 + dx * t, a.1 + dy * t, w * 0.5, c);
        }
    }

    /// A filled rectangle (inclusive bounds).
    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.put(x, y, c);
            }
        }
    }

    /// A rectangle outline of thickness `w`.
    fn rect_outline(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: [u8; 4]) {
        self.line((x0, y0), (x1, y0), w, c);
        self.line((x1, y0), (x1, y1), w, c);
        self.line((x1, y1), (x0, y1), w, c);
        self.line((x0, y1), (x0, y0), w, c);
    }

    /// Add a 1px black border around every opaque pixel, so the glyph reads on any bar
    /// background (light or dark).
    fn outline_black(&mut self) {
        let snap = self.px.clone();
        let opaque = |x: i32, y: i32| -> bool {
            if !(0..ICON).contains(&x) || !(0..ICON).contains(&y) {
                return false;
            }
            snap[((y * ICON + x) * 4) as usize] != 0
        };
        for y in 0..ICON {
            for x in 0..ICON {
                if !opaque(x, y)
                    && (opaque(x - 1, y)
                        || opaque(x + 1, y)
                        || opaque(x, y - 1)
                        || opaque(x, y + 1))
                {
                    self.put(x, y, [255, 0, 0, 0]);
                }
            }
        }
    }
}

/// Render the tray icon. While drawing (`active`), a glyph for the current `tool` in the
/// stroke `color`; when idle, a **grey pencil drawing a red scribble** (clearly "annotate
/// the screen"). A 1px black outline gives contrast on any panel.
fn draw_icon(active: bool, color: Color, tool: Tool) -> Icon {
    let mut c = Canvas::new();
    if !active {
        // Idle: a grey pencil drawing a red scribble — clearly "annotate the screen",
        // and the red mark stays visible on any panel (plain grey was too faint).
        let grey = [255, 190, 190, 190];
        let red = [255, 0xff, 0x3b, 0x30];
        c.line((15.0, 4.0), (9.0, 12.0), 4.0, grey); // pencil shaft
        c.line((9.0, 12.0), (6.0, 15.0), 2.5, grey); // sharpened tip
        let wave = [
            (6.0, 16.0),
            (9.0, 19.0),
            (12.0, 15.0),
            (15.0, 19.0),
            (18.0, 16.0),
        ];
        for seg in wave.windows(2) {
            c.line(seg[0], seg[1], 2.0, red); // the red mark it draws
        }
        c.outline_black();
        return Icon {
            width: ICON,
            height: ICON,
            data: c.px,
        };
    }
    // Drawing: a glyph for the current tool, in the stroke colour.
    let fg = [255, color[0], color[1], color[2]];
    match tool {
        // Pen: a thick diagonal stroke.
        Tool::Pen => c.line((5.0, 16.0), (16.0, 5.0), 3.0, fg),
        // Rectangle outline.
        Tool::Rect => c.rect_outline(5.0, 6.0, 16.0, 15.0, 2.0, fg),
        // Mask: a solid filled rectangle.
        Tool::Mask => c.fill_rect(5, 6, 16, 15, fg),
        // Arrow: a shaft with a small head.
        Tool::Arrow => {
            c.line((5.0, 16.0), (15.0, 6.0), 2.0, fg);
            c.line((15.0, 6.0), (15.0, 11.0), 2.0, fg);
            c.line((15.0, 6.0), (10.0, 6.0), 2.0, fg);
        }
        // Text: a "T".
        Tool::Text => {
            c.line((5.0, 6.0), (17.0, 6.0), 3.0, fg);
            c.line((11.0, 6.0), (11.0, 16.0), 3.0, fg);
        }
        // Eraser: a filled diamond (distinct from the square tools).
        Tool::Eraser => {
            let (cx, cy, r) = (11.0, 11.0, 6.0);
            for y in 0..ICON {
                for x in 0..ICON {
                    if (x as f32 - cx).abs() + (y as f32 - cy).abs() <= r {
                        c.put(x, y, fg);
                    }
                }
            }
        }
        // Move: a four-way arrow (cross with arrow tips at each end).
        Tool::Move => {
            c.line((11.0, 4.0), (11.0, 18.0), 2.0, fg);
            c.line((4.0, 11.0), (18.0, 11.0), 2.0, fg);
            c.line((11.0, 4.0), (8.0, 7.0), 2.0, fg);
            c.line((11.0, 4.0), (14.0, 7.0), 2.0, fg);
            c.line((11.0, 18.0), (8.0, 15.0), 2.0, fg);
            c.line((11.0, 18.0), (14.0, 15.0), 2.0, fg);
            c.line((4.0, 11.0), (7.0, 8.0), 2.0, fg);
            c.line((4.0, 11.0), (7.0, 14.0), 2.0, fg);
            c.line((18.0, 11.0), (15.0, 8.0), 2.0, fg);
            c.line((18.0, 11.0), (15.0, 14.0), 2.0, fg);
        }
    }
    c.outline_black();
    Icon {
        width: ICON,
        height: ICON,
        data: c.px,
    }
}
