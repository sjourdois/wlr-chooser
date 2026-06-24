//! Focus-aware capture helpers: "the active window" and "the current output".
//!
//! Wayland deliberately gives a regular client no way to query the global pointer
//! position or which surface/output has the focus — so, like `grimshot`, we rely
//! on the compositor's own IPC. This is a small trait with per-compositor backends
//! selected from the environment: Sway (`swaymsg`), Hyprland (`hyprctl`) and niri
//! (`niri msg`).

use crate::wl::Region;

/// A compositor-specific source of focus information.
pub trait FocusBackend {
    /// Name of the focused output, if any.
    fn focused_output(&self) -> Option<String>;
    /// Logical rectangle of the active (focused) window, if any.
    fn active_window_rect(&self) -> Option<Region>;
    /// Human-readable backend name, for error messages.
    fn name(&self) -> &'static str;
}

/// Pick a focus backend from the environment. `None` if no supported compositor
/// IPC is present (Wayland has no portable fallback — see the module docs).
pub fn detect() -> Option<Box<dyn FocusBackend>> {
    if std::env::var_os("SWAYSOCK").is_some() {
        return Some(Box::new(Sway));
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Some(Box::new(Hyprland));
    }
    if std::env::var_os("NIRI_SOCKET").is_some() {
        return Some(Box::new(Niri));
    }
    None
}

/// Sway / wlroots `swaymsg` backend.
struct Sway;

impl Sway {
    fn query(kind: &str) -> Option<serde_json::Value> {
        let out = std::process::Command::new("swaymsg")
            .args(["-t", kind, "-r"])
            .output()
            .ok()?;
        out.status.success().then_some(())?;
        serde_json::from_slice(&out.stdout).ok()
    }
}

impl FocusBackend for Sway {
    fn name(&self) -> &'static str {
        "sway"
    }

    fn focused_output(&self) -> Option<String> {
        let outputs = Self::query("get_outputs")?;
        outputs
            .as_array()?
            .iter()
            .find(|o| o["focused"].as_bool() == Some(true))?["name"]
            .as_str()
            .map(String::from)
    }

    fn active_window_rect(&self) -> Option<Region> {
        let tree = Self::query("get_tree")?;
        let node = find_focused(&tree)?;
        // Only windows have an app_id / window properties; a focused empty
        // workspace is not an "active window".
        let is_window = node.get("app_id").is_some_and(|a| !a.is_null())
            || node.get("window_properties").is_some()
            || (matches!(
                node.get("type").and_then(|t| t.as_str()),
                Some("con") | Some("floating_con")
            ) && node.get("name").is_some_and(|n| !n.is_null()));
        if !is_window {
            return None;
        }
        rect_of(node)
    }
}

/// The single node with `"focused": true` in a sway tree (the active container).
fn find_focused(node: &serde_json::Value) -> Option<&serde_json::Value> {
    if node.get("focused").and_then(|f| f.as_bool()) == Some(true) {
        return Some(node);
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(|c| c.as_array()) {
            for child in children {
                if let Some(found) = find_focused(child) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Read a sway `rect` object into a logical [`Region`].
fn rect_of(node: &serde_json::Value) -> Option<Region> {
    let r = node.get("rect")?;
    Some(Region {
        x: r["x"].as_i64()? as i32,
        y: r["y"].as_i64()? as i32,
        w: r["width"].as_u64()? as u32,
        h: r["height"].as_u64()? as u32,
    })
}

/// Hyprland `hyprctl -j` backend.
struct Hyprland;

impl Hyprland {
    fn query(cmd: &str) -> Option<serde_json::Value> {
        let out = std::process::Command::new("hyprctl")
            .args(["-j", cmd])
            .output()
            .ok()?;
        out.status.success().then_some(())?;
        serde_json::from_slice(&out.stdout).ok()
    }
}

impl FocusBackend for Hyprland {
    fn name(&self) -> &'static str {
        "Hyprland"
    }

    fn focused_output(&self) -> Option<String> {
        hypr_focused_output(&Self::query("monitors")?)
    }

    fn active_window_rect(&self) -> Option<Region> {
        hypr_active_window_rect(&Self::query("activewindow")?)
    }
}

/// Pick the focused monitor's name from `hyprctl -j monitors` (an array of monitors,
/// one with `"focused": true`).
fn hypr_focused_output(monitors: &serde_json::Value) -> Option<String> {
    monitors
        .as_array()?
        .iter()
        .find(|m| m["focused"].as_bool() == Some(true))?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Read the active window's rectangle from `hyprctl -j activewindow`: `at: [x, y]`
/// and `size: [w, h]` in global logical coordinates. An empty object (`{}`) — nothing
/// focused — yields `None`.
fn hypr_active_window_rect(w: &serde_json::Value) -> Option<Region> {
    let at = w.get("at")?.as_array()?;
    let size = w.get("size")?.as_array()?;
    Some(Region {
        x: at.first()?.as_i64()? as i32,
        y: at.get(1)?.as_i64()? as i32,
        w: size.first()?.as_i64()? as u32,
        h: size.get(1)?.as_i64()? as u32,
    })
}

/// niri `niri msg --json` backend.
struct Niri;

impl Niri {
    fn query(action: &str) -> Option<serde_json::Value> {
        let out = std::process::Command::new("niri")
            .args(["msg", "--json", action])
            .output()
            .ok()?;
        out.status.success().then_some(())?;
        serde_json::from_slice(&out.stdout).ok()
    }
}

impl FocusBackend for Niri {
    fn name(&self) -> &'static str {
        "niri"
    }

    fn focused_output(&self) -> Option<String> {
        niri_focused_output(&Self::query("focused-output")?)
    }

    fn active_window_rect(&self) -> Option<Region> {
        // niri's IPC does not expose a window's rectangle in global logical
        // coordinates (scrollable tiling lets windows extend off-screen), so the
        // active-window source is unavailable — callers get a clear error and can
        // use `--current-output` or `-g` instead.
        None
    }
}

/// Pick the focused output's name from `niri msg --json focused-output` (the Output
/// object, or `null` when none).
fn niri_focused_output(o: &serde_json::Value) -> Option<String> {
    o.get("name")?.as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed but faithful `hyprctl -j monitors` sample (two monitors, the second
    // focused) — locks the field names (`focused`, `name`) the parser relies on.
    const HYPR_MONITORS: &str = r#"[
        {"id":0,"name":"DP-1","make":"Dell","model":"X","width":2560,"height":1440,
         "x":0,"y":0,"refreshRate":59.95,"scale":1.0,"focused":false},
        {"id":1,"name":"HDMI-A-1","make":"LG","model":"Y","width":1920,"height":1080,
         "x":2560,"y":0,"refreshRate":60.0,"scale":1.0,"focused":true}
    ]"#;

    // `hyprctl -j activewindow` gives `at`/`size` pairs in global logical coords.
    const HYPR_ACTIVEWINDOW: &str =
        r#"{"address":"0x55","class":"foot","title":"foot","at":[120,340],"size":[800,600]}"#;

    #[test]
    fn hypr_focused_output_picks_focused_monitor() {
        let v: serde_json::Value = serde_json::from_str(HYPR_MONITORS).unwrap();
        assert_eq!(hypr_focused_output(&v).as_deref(), Some("HDMI-A-1"));
    }

    #[test]
    fn hypr_active_window_rect_reads_at_and_size() {
        let v: serde_json::Value = serde_json::from_str(HYPR_ACTIVEWINDOW).unwrap();
        assert_eq!(
            hypr_active_window_rect(&v),
            Some(Region {
                x: 120,
                y: 340,
                w: 800,
                h: 600
            })
        );
    }

    #[test]
    fn hypr_no_active_window_is_none() {
        // Hyprland returns `{}` when nothing is focused.
        let v: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert!(hypr_active_window_rect(&v).is_none());
    }

    #[test]
    fn niri_focused_output_reads_name() {
        // Shape per niri's `focused-output` (the Output object). Unverified live.
        let v: serde_json::Value = serde_json::from_str(
            r#"{"name":"eDP-1","make":"BOE","model":"Z",
                "logical":{"x":0,"y":0,"width":1920,"height":1080,"scale":1.0}}"#,
        )
        .unwrap();
        assert_eq!(niri_focused_output(&v).as_deref(), Some("eDP-1"));
        // `null` (no focused output) → None.
        assert!(niri_focused_output(&serde_json::Value::Null).is_none());
    }
}
