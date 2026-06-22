//! wlr-chooser — graphical window & screen picker for wlroots screencast portals.
//!
//! Output contract (stdout) expected by xdg-desktop-portal-wlr:
//! `Window: <foreign-toplevel-identifier>` or `Monitor: <output-name>`.
//! On cancel: no output, non-zero exit.

mod shell;
mod ui;

use clap::{Parser, ValueEnum};
use std::sync::{Arc, Mutex, mpsc};
use ui::Mode;
use wlr_capture::tr;
use wlr_capture::{i18n, theme, wl};

/// Presentation for the `--switch` window switcher.
#[derive(Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum Layout {
    /// Full-screen mission-control grid (default).
    #[default]
    Full,
    /// Compact centred card, like the portal picker.
    Compact,
}

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
    /// Window switcher: pick a window to focus it (no stdout). Implies --windows.
    #[arg(long)]
    switch: bool,
    /// Switcher presentation: `full` (full-screen mission-control grid, default)
    /// or `compact` (centred card). Only meaningful with --switch.
    #[arg(long, value_enum, default_value_t = Layout::Full)]
    layout: Layout,
    /// Headless capture benchmark: run the capture loop for SECS seconds and
    /// print per-source frame/change stats to stderr (debug; no overlay).
    #[arg(long, value_name = "SECS", hide = true)]
    bench_capture: Option<u64>,
}

/// Acquire the single-instance advisory lock for the interactive switcher/exposé.
/// Returns the held lock file (keep it alive), or `None` if another instance owns it.
fn acquire_switch_lock() -> Option<std::fs::File> {
    use rustix::fs::{FlockOperation, flock};
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("wlr-chooser-switch.lock"))
        .ok()?;
    flock(&f, FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(f)
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

    if let Some(secs) = cli.bench_capture {
        ui::bench_capture(secs);
        return;
    }

    let switch = cli.switch; // focuses the picked window
    // Full-screen exposé layout only applies to the switcher's `full` presentation.
    let expose = switch && cli.layout == Layout::Full;
    let mode = if cli.windows || switch {
        Mode::Windows
    } else if cli.outputs {
        Mode::Outputs
    } else {
        Mode::All
    };

    // Single-instance guard for the interactive switcher/exposé. sway handles the
    // keybind itself (compositor bindings aren't delivered to surfaces, even with
    // our exclusive keyboard grab), so pressing $mod+Tab again would relaunch and
    // stack overlays. Hold an advisory lock for our lifetime; a second instance
    // bails immediately. Kept alive in `_instance_lock` until the process exits.
    let _instance_lock = if switch {
        match acquire_switch_lock() {
            Some(lock) => Some(lock),
            None => return, // another switcher/exposé is already open
        }
    } else {
        None
    };

    // Capture runs on its own thread (it owns the non-Send Wayland client) and
    // streams thumbnails to the UI; the window opens immediately.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || ui::capture_thread(tx));

    let out: ui::Outcome = Arc::new(Mutex::new(None));
    let out_for_app = out.clone();
    let theme = theme::Theme::load();
    let app = ui::App::new(
        rx,
        out_for_app,
        mode,
        cli.include_system,
        cli.grid,
        expose,
        theme,
    );

    // Native wlr-layer-shell overlay (rofi-like): dims the desktop, grabs the keyboard.
    if let Err(e) = shell::run(app) {
        eprintln!("{}", tr!("error", error = format!("{e:#}")));
        std::process::exit(2);
    }

    let selection = out.lock().unwrap().take();
    match selection {
        Some(sel) if switch => {
            // Window switcher / exposé: focus the picked window instead of printing.
            if sel.is_window {
                if let Err(e) = wl::activate_window(&sel.app_id, &sel.title, sel.dup_index) {
                    eprintln!("{}", tr!("error", error = format!("{e:#}")));
                    std::process::exit(2);
                }
            }
            std::process::exit(0);
        }
        Some(sel) => {
            // Portal contract: print the chosen source.
            println!("{}", sel.token);
            std::process::exit(0);
        }
        None => std::process::exit(1), // cancelled
    }
}
