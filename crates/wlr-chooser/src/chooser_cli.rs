//! wlr-chooser — graphical window & screen picker for wlroots screencast portals.
//!
//! Output contract (stdout) expected by xdg-desktop-portal-wlr:
//! `Window: <foreign-toplevel-identifier>` or `Monitor: <output-name>`.
//! On cancel: no output, non-zero exit.
//!
//! For an interactive window switcher / Alt-Tab / exposé, see the sibling
//! `wlr-switcher` binary.

use crate::ui::{self, Live, Mode, Options, View};
use crate::{i18n, tr};
use crate::{parse_grid, run_overlay};
use clap::Parser;
use std::time::Instant;

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
    /// Headless capture benchmark: run the capture loop for SECS seconds and
    /// print per-source frame/change stats to stderr (debug; no overlay).
    #[arg(long, value_name = "SECS", hide = true)]
    bench_capture: Option<u64>,
    /// Report which capture protocols the current compositor supports, then exit.
    #[arg(long)]
    doctor: bool,
}

pub fn main() {
    let t0 = Instant::now();
    let cli = Cli::parse();
    i18n::init();

    if cli.doctor {
        if let Err(e) = wlr_capture::doctor::report() {
            eprintln!("wlr-chooser: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Some(secs) = cli.bench_capture {
        ui::bench_capture(secs);
        return;
    }

    let _ = cli.both; // default; accepted for symmetry with -w/-o
    let mode = if cli.windows {
        Mode::Windows
    } else if cli.outputs {
        Mode::Outputs
    } else {
        Mode::All
    };
    let opts = Options {
        mode,
        show_system: cli.include_system,
        grid: cli.grid,
        view: View::Card,
        hold: false,
        live: Live::All,
    };

    match run_overlay(opts, t0) {
        Ok(Some(sel)) => {
            // Portal contract: print the chosen source.
            println!("{}", sel.token);
        }
        Ok(None) => std::process::exit(1), // cancelled
        Err(e) => {
            eprintln!("{}", tr!("error", error = format!("{e:#}")));
            std::process::exit(2);
        }
    }
}
