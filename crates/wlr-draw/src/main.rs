//! `wlr-draw` — draw and annotate live on screen on wlroots compositors.
//!
//! With no subcommand it runs the daemon: a transparent always-on-top overlay you draw
//! on (see [`overlay`]). Every other invocation is a one-shot control message sent to
//! the running daemon over a Unix socket ([`ipc`]) — `toggle`, `clear`, `tool arrow`,
//! `color #00ff00`, … — so you bind them to compositor keys.

#[cfg(feature = "tray")]
mod autostart;
mod ipc;
mod model;
mod overlay;
mod proto;
#[cfg(feature = "tray")]
mod tray;

use crate::model::{Tool, parse_color};
use crate::proto::Cmd;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "wlr-draw",
    version,
    about = "Draw and annotate live on screen (wlroots / layer-shell)",
    long_about = "Run with no subcommand to start the overlay daemon. A wlroots client \
cannot grab a global hotkey, so further invocations drive the running daemon over a \
control socket — bind them to compositor keys (e.g. sway `bindsym $mod+a exec wlr-draw \
toggle`)."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Ctl>,
}

#[derive(Subcommand)]
enum Ctl {
    /// Toggle draw mode (grab input ↔ click-through)
    Toggle,
    /// Enter draw mode (grab input)
    On,
    /// Leave draw mode (click-through; annotations stay on screen)
    Off,
    /// Erase all annotations
    Clear,
    /// Undo the last action
    Undo,
    /// Redo the last undone action
    Redo,
    /// Hide / show the annotations without discarding them
    Visibility,
    /// Select a tool: pen, line, rect, ellipse, arrow, text, eraser
    Tool {
        /// Tool name
        name: String,
    },
    /// Set the stroke colour: a name (red, blue…) or #rrggbb[aa]
    Color {
        /// Colour name or hex
        value: String,
    },
    /// Set the stroke width in pixels
    Width {
        /// Width in logical pixels
        px: f32,
    },
    /// Stop the running daemon
    Quit,
}

fn main() -> anyhow::Result<()> {
    // Negotiate the UI language from the desktop locale (no-op without the `i18n`
    // feature). Like every other binary in the workspace — without it the tray menu and
    // on-screen hints stay English regardless of `$LANG`.
    wlr_capture::i18n::init();
    let cli = Cli::parse();
    match cli.cmd {
        None => overlay::run(),
        Some(ctl) => ipc::send(&ctl_to_cmd(ctl)?),
    }
}

/// Map a CLI subcommand to a protocol command, validating tool/colour client-side so
/// errors surface before anything is sent.
fn ctl_to_cmd(ctl: Ctl) -> anyhow::Result<Cmd> {
    Ok(match ctl {
        Ctl::Toggle => Cmd::Toggle,
        Ctl::On => Cmd::On,
        Ctl::Off => Cmd::Off,
        Ctl::Clear => Cmd::Clear,
        Ctl::Undo => Cmd::Undo,
        Ctl::Redo => Cmd::Redo,
        Ctl::Visibility => Cmd::Visibility,
        Ctl::Quit => Cmd::Quit,
        Ctl::Tool { name } => Cmd::Tool(
            Tool::from_name(&name).ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?,
        ),
        Ctl::Color { value } => Cmd::Color(
            parse_color(&value).ok_or_else(|| anyhow::anyhow!("unknown colour: {value}"))?,
        ),
        Ctl::Width { px } => Cmd::Width(px),
    })
}
