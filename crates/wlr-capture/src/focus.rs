//! Focus-aware capture helpers: "the active window" and "the current output".
//!
//! Wayland deliberately gives a regular client no way to query the global pointer
//! position or which surface/output has the focus — so, like `grimshot`, we rely
//! on the compositor's own IPC. This is a small trait with per-compositor backends
//! selected from the environment: Sway (`swaymsg`), Hyprland (`hyprctl`) and niri
//! (`niri msg`).

use crate::wl::Region;

/// A window's identity + content geometry, for binding a region mirror to the window
/// under it (`app_id` + `title` match a `wl::Toplevel`; `rect` is its content area).
pub struct WindowRef {
    pub app_id: String,
    pub title: String,
    pub rect: Region,
}

/// A compositor-specific source of focus information.
pub trait FocusBackend {
    /// Name of the focused output, if any.
    fn focused_output(&self) -> Option<String>;
    /// Logical rectangle of the active (focused) window, if any.
    fn active_window_rect(&self) -> Option<Region>;
    /// The window under the given global logical point, if any. Used to make a region
    /// mirror follow the window beneath it. Default `None` (only Sway implements it).
    fn window_at(&self, _x: i32, _y: i32) -> Option<WindowRef> {
        None
    }
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

    fn window_at(&self, x: i32, y: i32) -> Option<WindowRef> {
        sway_window_at(&Self::query("get_tree")?, x, y)
    }
}

/// Whether a sway node is a window (vs a container/workspace/output).
fn sway_is_window(node: &serde_json::Value) -> bool {
    node.get("app_id").is_some_and(|a| !a.is_null()) || node.get("window_properties").is_some()
}

/// A node's content rectangle in global logical coordinates: its `rect` shifted by the
/// `window_rect` (content offset within the node), so the crop lines up with what the
/// foreign-toplevel capture actually contains (no server-side borders).
fn sway_content_rect(node: &serde_json::Value) -> Option<Region> {
    let rect = rect_of(node)?;
    if let Some(wr) = node.get("window_rect") {
        if let (Some(w), Some(h)) = (wr["width"].as_u64(), wr["height"].as_u64()) {
            if w > 0 && h > 0 {
                return Some(Region {
                    x: rect.x + wr["x"].as_i64().unwrap_or(0) as i32,
                    y: rect.y + wr["y"].as_i64().unwrap_or(0) as i32,
                    w: w as u32,
                    h: h as u32,
                });
            }
        }
    }
    Some(rect)
}

/// The deepest window node whose `rect` contains the global logical point `(x, y)`.
fn sway_window_at(node: &serde_json::Value, x: i32, y: i32) -> Option<WindowRef> {
    // Skip anything not actually on screen — sway keeps the geometry of windows on
    // hidden workspaces (and tabbed/stacked-behind windows) in the tree, so without
    // this we'd match a window the point only "contains" on a workspace you can't see.
    if node.get("visible").and_then(|v| v.as_bool()) == Some(false) {
        return None;
    }
    // Descend into children first so the innermost (leaf) window wins.
    for key in ["floating_nodes", "nodes"] {
        if let Some(children) = node.get(key).and_then(|c| c.as_array()) {
            for child in children {
                if rect_of(child).is_some_and(|r| contains(&r, x, y)) {
                    if let Some(found) = sway_window_at(child, x, y) {
                        return Some(found);
                    }
                }
            }
        }
    }
    if sway_is_window(node) && rect_of(node).is_some_and(|r| contains(&r, x, y)) {
        let app_id = node["app_id"]
            .as_str()
            .or_else(|| node["window_properties"]["class"].as_str())
            .unwrap_or_default()
            .to_string();
        return Some(WindowRef {
            app_id,
            title: node["name"].as_str().unwrap_or_default().to_string(),
            rect: sway_content_rect(node)?,
        });
    }
    None
}

/// Whether `(x, y)` falls inside a logical region.
fn contains(r: &Region, x: i32, y: i32) -> bool {
    x >= r.x && x < r.x + r.w as i32 && y >= r.y && y < r.y + r.h as i32
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

    // A trimmed sway `get_tree`: an output with a visible workspace (firefox, with a
    // 20px title-bar `window_rect`) and a *hidden* workspace whose window (vim) covers
    // the same coordinates — sway keeps its geometry even though it's off screen.
    const SWAY_TREE: &str = r#"{
      "type":"root","rect":{"x":0,"y":0,"width":3840,"height":1440},
      "nodes":[{
        "type":"output","name":"DP-4","rect":{"x":0,"y":0,"width":3840,"height":1440},
        "nodes":[
          {
            "type":"workspace","name":"1","visible":true,
            "rect":{"x":0,"y":0,"width":3840,"height":1440},
            "nodes":[{
              "type":"con","app_id":"firefox","name":"Page Title","visible":true,
              "rect":{"x":100,"y":100,"width":800,"height":600},
              "window_rect":{"x":0,"y":20,"width":800,"height":580}
            }]
          },
          {
            "type":"workspace","name":"2","visible":false,
            "rect":{"x":0,"y":0,"width":3840,"height":1440},
            "nodes":[{
              "type":"con","app_id":"vim","name":"editor","visible":false,
              "rect":{"x":100,"y":100,"width":800,"height":600}
            }]
          }
        ]
      }]
    }"#;

    #[test]
    fn sway_window_at_finds_visible_window_and_content_rect() {
        let v: serde_json::Value = serde_json::from_str(SWAY_TREE).unwrap();
        let w = sway_window_at(&v, 200, 200).expect("window under the point");
        // The visible window wins, never the one on the hidden workspace.
        assert_eq!(w.app_id, "firefox");
        assert_eq!(w.title, "Page Title");
        // Content rect = node rect shifted by the 20px title bar.
        assert_eq!(
            w.rect,
            Region {
                x: 100,
                y: 120,
                w: 800,
                h: 580
            }
        );
        // A point on the empty desktop hits no window.
        assert!(sway_window_at(&v, 2000, 1300).is_none());
    }

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
