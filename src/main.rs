//! wlr-chooser — graphical window & screen picker for wlroots screencast portals.
//!
//! Output contract (stdout) expected by xdg-desktop-portal-wlr:
//! `Window: <foreign-toplevel-identifier>` or `Monitor: <output-name>`.
//! On cancel: no output, non-zero exit.

mod i18n;
mod icons;
mod shell;
mod theme;
mod ui;
mod wl;

use clap::Parser;
use std::sync::{Arc, Mutex, mpsc};
use ui::Mode;

/// Graphical window & screen picker for xdg-desktop-portal-wlr.
///
/// Prints the chosen source to stdout (`Window: <id>` / `Monitor: <name>`); exits
/// non-zero if cancelled.
#[derive(Parser)]
#[command(name = "wlr-chooser", version, about)]
struct Cli {
    /// Show only windows
    #[arg(short = 'w', long, group = "what")]
    windows: bool,
    /// Show only screens
    #[arg(short = 'o', long, visible_alias = "screens", group = "what")]
    outputs: bool,
    /// Show both windows and screens (default)
    #[arg(long, group = "what")]
    both: bool,
    /// Include windows with no app-id (system surfaces)
    #[arg(long)]
    include_system: bool,
    /// Show a fixed COLSxROWS grid of thumbnails (e.g. 4x3)
    #[arg(long, value_name = "COLSxROWS", value_parser = parse_grid)]
    grid: Option<(u32, u32)>,
}

/// Parse a `COLSxROWS` grid spec (e.g. `4x3`).
fn parse_grid(s: &str) -> Result<(u32, u32), String> {
    let (c, r) = s
        .split_once(['x', 'X', '×'])
        .ok_or("expected COLSxROWS, e.g. 4x3")?;
    let n = |v: &str, what: &str| {
        v.trim()
            .parse::<u32>()
            .ok()
            .filter(|&n| n >= 1)
            .ok_or(format!("{what} must be a positive integer"))
    };
    Ok((n(c, "columns")?, n(r, "rows")?))
}

fn main() {
    let cli = Cli::parse();
    i18n::init();

    let mode = if cli.windows {
        Mode::Windows
    } else if cli.outputs {
        Mode::Outputs
    } else {
        Mode::All
    };

    // Capture runs on its own thread (it owns the non-Send Wayland client) and
    // streams thumbnails to the UI; the window opens immediately.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || ui::capture_thread(tx));

    let out: ui::Outcome = Arc::new(Mutex::new(None));
    let out_for_app = out.clone();
    let theme = theme::Theme::load();
    let app = ui::App::new(rx, out_for_app, mode, cli.include_system, cli.grid, theme);

    // Native wlr-layer-shell overlay (rofi-like): dims the desktop, grabs the keyboard.
    if let Err(e) = shell::run(app) {
        eprintln!("{}", tr!("error", error = format!("{e:#}")));
        std::process::exit(2);
    }

    match out.lock().unwrap().take() {
        Some(token) => {
            println!("{token}");
            std::process::exit(0);
        }
        None => std::process::exit(1), // cancelled
    }
}
