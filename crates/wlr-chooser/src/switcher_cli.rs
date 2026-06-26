//! wlr-switcher — window switcher / Alt-Tab / exposé for wlroots compositors.
//!
//! Picks a window from a live overlay and **focuses** it (via
//! `zwlr-foreign-toplevel-management-v1`). Bind it to a held modifier for a true
//! Alt-Tab: hold the modifier, `Tab`/`Shift+Tab` cycle, release to switch. Three
//! presentations via `--layout`; live previews are the differentiator.
//!
//! For the xdg-desktop-portal-wlr picker (prints to stdout), see `wlr-chooser`.

use crate::ui::{Live, Mode, Options, View};
use crate::{acquire_switch_lock, run_overlay};
use clap::{Parser, ValueEnum};
use std::time::Instant;
use wlr_capture::{i18n, tr, wl};

/// Presentation of the switcher (CLI mirror of [`View`]).
#[derive(Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum LayoutArg {
    /// macOS-style single row of tiles (default).
    #[default]
    Strip,
    /// Full-screen mission-control exposé grid.
    Grid,
    /// Centred rofi-like card with tabs + search.
    Card,
}

/// Which tiles show a live preview (CLI mirror of [`Live`]).
#[derive(Clone, Copy, ValueEnum)]
enum LiveArg {
    None,
    Current,
    All,
}

impl From<LiveArg> for Live {
    fn from(v: LiveArg) -> Self {
        match v {
            LiveArg::None => Live::None,
            LiveArg::Current => Live::Current,
            LiveArg::All => Live::All,
        }
    }
}

/// Window switcher / Alt-Tab / exposé for wlroots: focuses the picked window.
#[derive(Parser)]
#[command(
    name = "wlr-switcher",
    version,
    about = "Window switcher / Alt-Tab / exposé for wlroots (focuses the picked window)"
)]
struct Cli {
    /// Presentation: `strip` (macOS-style row, default), `grid` (full-screen
    /// exposé) or `card` (centred rofi-like card).
    #[arg(long, value_enum, default_value_t = LayoutArg::Strip)]
    layout: LayoutArg,
    /// Live previews: `none` (icons only), `current` (only the highlighted window)
    /// or `all` (default). Live capture is the differentiator.
    #[arg(long, value_enum, default_value_t = LiveArg::All)]
    live: LiveArg,
    /// Hold-to-switch: confirm and close the moment the held launch modifier
    /// (Alt/Super) is released. Default: on for `strip`, off for `grid`/`card`.
    /// Bind it to a held modifier — e.g. `Mod1+Tab exec wlr-switcher` — for a
    /// true Alt-Tab. Use this to force it on for `grid`/`card`.
    #[arg(long)]
    hold: bool,
    /// Disable hold-to-switch: the overlay stays open after releasing the
    /// modifier — confirm with Enter or a click. Overrides the per-layout default.
    #[arg(long, conflicts_with = "hold")]
    no_hold: bool,
    /// Include windows with no app-id (system surfaces)
    #[arg(long)]
    include_system: bool,
}

pub fn main() {
    let t0 = Instant::now();
    let cli = Cli::parse();
    i18n::init();

    // Single-instance guard: re-pressing the keybind while we're up is a no-op
    // rather than a stacked overlay (sway runs its bindings over our grab).
    let _lock = match acquire_switch_lock() {
        Some(lock) => lock,
        None => return,
    };

    let view = match cli.layout {
        LayoutArg::Strip => View::Strip,
        LayoutArg::Grid => View::Grid,
        LayoutArg::Card => View::Card,
    };
    // Hold-to-switch defaults on for the strip (a true Alt-Tab) and off for the
    // exposé/card; --hold / --no-hold force either.
    let hold = if cli.hold {
        true
    } else if cli.no_hold {
        false
    } else {
        cli.layout == LayoutArg::Strip
    };
    let opts = Options {
        mode: Mode::Windows,
        show_system: cli.include_system,
        grid: None,
        view,
        hold,
        live: cli.live.into(),
    };

    match run_overlay(opts, t0) {
        Ok(Some(sel)) => {
            // Focus the picked window (outputs aren't focusable, so ignore them).
            if sel.is_window
                && let Err(e) = wl::activate_window(&sel.app_id, &sel.title, sel.dup_index)
            {
                eprintln!("{}", tr!("error", error = format!("{e:#}")));
                std::process::exit(2);
            }
        }
        Ok(None) => std::process::exit(1), // cancelled
        Err(e) => {
            eprintln!("{}", tr!("error", error = format!("{e:#}")));
            std::process::exit(2);
        }
    }
}
