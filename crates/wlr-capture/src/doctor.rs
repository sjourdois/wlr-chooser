//! Environment + compositor-capability probe, shared by every tool's `doctor` command.
//!
//! It reports the run environment (tool version, OS, compositor + version, install hint)
//! and the capture-relevant Wayland globals the current compositor advertises, so users
//! (and bug reports) can tell at a glance whether — and how well — the suite works here.
//! The capability core is a plain read of [`wl::advertised_globals`] plus a string table;
//! the environment block reads `/etc/os-release`, the desktop env vars and (best-effort)
//! the compositor's own `--version`, so exposing it from every binary adds no dependencies.

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
/// command / `--doctor` flag. `tool` / `version` identify the calling binary so the
/// report doubles as the environment block a bug report needs.
pub fn report(tool: &str, version: &str) -> Result<()> {
    println!("{tool} {version}\n");
    if let Some(os) = os_pretty_name() {
        println!("  OS:         {os}");
    }
    println!("  Compositor: {}", compositor());
    if let Ok(exe) = std::env::current_exe() {
        let home = std::env::var("HOME").ok();
        println!(
            "  Executable: {}",
            redact_home(&exe.display().to_string(), home.as_deref())
        );
    }
    println!();

    let globals = wl::advertised_globals().context("listing Wayland globals")?;
    let global_version = |iface: &str| globals.iter().find(|(n, _)| n == iface).map(|(_, v)| *v);

    println!("Compositor capabilities (advertised Wayland globals):\n");
    for (iface, desc) in CHECKS {
        match global_version(iface) {
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

/// Replace a leading `$HOME` in `path` with `~` so `doctor` output can be pasted into a
/// public bug report without leaking the username. Only the real `$HOME` prefix is masked
/// (at a path boundary, so `/home/bob` doesn't match `/home/bobby`); the rest of the path,
/// which hints at the install method, is kept.
fn redact_home(path: &str, home: Option<&str>) -> String {
    if let Some(rest) = home
        .filter(|h| !h.is_empty())
        .and_then(|h| path.strip_prefix(h))
        .filter(|r| r.is_empty() || r.starts_with('/'))
    {
        return format!("~{rest}");
    }
    path.to_string()
}

/// `PRETTY_NAME` from `/etc/os-release` (e.g. `Debian GNU/Linux 13 (trixie)`), if readable.
fn os_pretty_name() -> Option<String> {
    parse_pretty_name(&std::fs::read_to_string("/etc/os-release").ok()?)
}

fn parse_pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
}

/// Best-effort compositor identification: a name — from the focus backend when the
/// `focus` feature is on, else the desktop environment variables — plus its
/// self-reported version line when we recognise the compositor.
fn compositor() -> String {
    match detected_name() {
        None => "unknown (XDG_CURRENT_DESKTOP unset or unrecognised)".to_string(),
        Some(name) => match self_reported_version(&name) {
            Some(v) => format!("{name} — {v}"),
            None => name,
        },
    }
}

fn detected_name() -> Option<String> {
    #[cfg(feature = "focus")]
    if let Some(b) = crate::focus::detect() {
        return Some(b.name().to_string());
    }
    std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("XDG_SESSION_DESKTOP"))
        .ok()
        .filter(|s| !s.is_empty())
}

/// Ask a recognised compositor for its version (`sway --version`, `hyprctl version`,
/// `niri --version`) and return the first non-empty output line. Best-effort: returns
/// `None` if the compositor is unrecognised or the command isn't runnable.
fn self_reported_version(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    let (cmd, args): (&str, &[&str]) = if n.contains("sway") {
        ("sway", &["--version"])
    } else if n.contains("hypr") {
        ("hyprctl", &["version"])
    } else if n.contains("niri") {
        ("niri", &["--version"])
    } else {
        return None;
    };
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::{capture_verdict, parse_pretty_name, redact_home};

    #[test]
    fn redact_home_replaces_the_home_dir_with_tilde() {
        // $HOME prefix → `~`, keeping the install-hint tail.
        assert_eq!(
            redact_home("/home/bob/.cargo/bin/wlr-peek", Some("/home/bob")),
            "~/.cargo/bin/wlr-peek"
        );
        // A non-`/home` $HOME works just as well (that's the point of not hard-coding it).
        assert_eq!(
            redact_home("/root/.cargo/bin/wlr-peek", Some("/root")),
            "~/.cargo/bin/wlr-peek"
        );
        // Exactly $HOME.
        assert_eq!(redact_home("/home/bob", Some("/home/bob")), "~");
        // A prefix that isn't a path boundary must not match (bob vs bobby) — untouched.
        assert_eq!(
            redact_home("/home/bobby/x", Some("/home/bob")),
            "/home/bobby/x"
        );
        // No $HOME known, or a path outside it → left untouched.
        assert_eq!(redact_home("/home/alice/x", None), "/home/alice/x");
        assert_eq!(
            redact_home("/usr/bin/wlr-draw", Some("/home/bob")),
            "/usr/bin/wlr-draw"
        );
    }

    #[test]
    fn parse_pretty_name_reads_the_quoted_value() {
        let sample = "NAME=\"Debian GNU/Linux\"\n\
                      PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n\
                      VERSION_ID=\"13\"\n";
        assert_eq!(
            parse_pretty_name(sample).as_deref(),
            Some("Debian GNU/Linux 13 (trixie)")
        );
        assert_eq!(parse_pretty_name("ID=arch\n"), None);
    }

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
