//! Compositor capability probe, shared by every tool's `doctor` command.
//!
//! It reports the capture-relevant Wayland globals the current compositor advertises,
//! so users (and bug reports) can tell at a glance whether — and how well — the suite
//! works here. The core is a plain read of [`wl::advertised_globals`] plus a string
//! table, so exposing it from every binary costs nothing beyond those strings.

use crate::error::{Context, Result};
use crate::wl;

/// `(interface, what it enables)`, ordered roughly by importance.
const CHECKS: &[(&str, &str)] = &[
    ("ext_image_copy_capture_manager_v1", "capture frames (core)"),
    (
        "ext_output_image_capture_source_manager_v1",
        "capture an output (core)",
    ),
    (
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "capture a window",
    ),
    (
        "ext_foreign_toplevel_list_v1",
        "enumerate windows (chooser, -w)",
    ),
    ("zxdg_output_manager_v1", "accurate output geometry"),
    (
        "zwlr_layer_shell_v1",
        "overlays: region select, loupe, switcher",
    ),
    ("zwlr_data_control_manager_v1", "clipboard copy (-c)"),
    ("zwp_linux_dmabuf_v1", "zero-copy GPU capture"),
    (
        "zwp_keyboard_shortcuts_inhibit_manager_v1",
        "switcher keyboard grab",
    ),
];

/// Whether the compositor advertises the screen- and window-capture sources.
/// Returns `(screen, window)` as independent booleans — screen capture can work
/// while window capture doesn't, and `doctor` reports them separately.
pub fn capture_verdict(globals: &[(String, u32)]) -> (bool, bool) {
    let has = |iface: &str| globals.iter().any(|(n, _)| n == iface);
    let screen = has("ext_image_copy_capture_manager_v1")
        && has("ext_output_image_capture_source_manager_v1");
    let window = has("ext_foreign_toplevel_image_capture_source_manager_v1");
    (screen, window)
}

/// Print the compositor capability report to stdout. Backs every tool's `doctor`
/// command / `--doctor` flag.
pub fn report() -> Result<()> {
    let globals = wl::advertised_globals().context("listing Wayland globals")?;
    let version = |iface: &str| globals.iter().find(|(n, _)| n == iface).map(|(_, v)| *v);

    println!("Compositor capabilities (advertised Wayland globals):\n");
    for (iface, desc) in CHECKS {
        match version(iface) {
            Some(v) => println!("  ✓ {iface} (v{v}) — {desc}"),
            None => println!("  ✗ {iface} — {desc}"),
        }
    }

    let (core, window) = capture_verdict(&globals);
    println!();
    if core {
        println!(
            "Screen capture: supported (screenshots, recording, loupe, colour picker, wlr-draw)."
        );
    } else {
        println!(
            "Screen capture: UNSUPPORTED — needs ext-image-copy-capture-v1 + the output \
             source (wlroots ≥ 0.19 / Sway ≥ 1.11; not on Mutter/KWin via this path)."
        );
    }
    if window {
        println!(
            "Window capture: supported (wlr-switcher, -w/--pick-window, window mirror/record)."
        );
    } else {
        println!(
            "Window capture: UNSUPPORTED — needs the foreign-toplevel source \
             (wlroots ≥ 0.20 / Sway ≥ 1.12). Screen capture still works; only window-only \
             features are unavailable (wlr-switcher exits with a notice)."
        );
    }

    // Focus IPC (active-window / current-output) needs the `focus` feature; a lean
    // build without it simply omits this line.
    #[cfg(feature = "focus")]
    match crate::focus::detect() {
        Some(b) => println!(
            "Focus IPC: {} detected (-a / --current-output work).",
            b.name()
        ),
        None => println!("Focus IPC: none detected (-a / --current-output unavailable)."),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::capture_verdict;

    fn globals(names: &[&str]) -> Vec<(String, u32)> {
        names.iter().map(|n| ((*n).to_string(), 1)).collect()
    }

    #[test]
    fn capture_verdict_reads_screen_and_window_floors() {
        const CORE: [&str; 2] = [
            "ext_image_copy_capture_manager_v1",
            "ext_output_image_capture_source_manager_v1",
        ];
        const FOREIGN: &str = "ext_foreign_toplevel_image_capture_source_manager_v1";

        // Nothing advertised → neither capture path works.
        assert_eq!(capture_verdict(&globals(&[])), (false, false));
        // Core copy-capture + output source → screen only (wlroots 0.19 / Sway 1.11).
        assert_eq!(capture_verdict(&globals(&CORE)), (true, false));
        // Add the foreign-toplevel source → window capture too (0.20 / 1.12).
        let mut all = CORE.to_vec();
        all.push(FOREIGN);
        assert_eq!(capture_verdict(&globals(&all)), (true, true));
        // Only one of the two core managers → screen still unsupported.
        assert_eq!(capture_verdict(&globals(&CORE[..1])), (false, false));
        // The screen and window verdicts are independent booleans, as report() prints them.
        assert_eq!(capture_verdict(&globals(&[FOREIGN])), (false, true));
    }
}
