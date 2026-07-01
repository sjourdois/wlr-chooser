//! Configurable keybindings, loaded from `~/.config/wlr-draw/keys.toml`.
//!
//! Each action resolves to a [`Trigger`] — either a regular key (an XKB keysym) or a
//! modifier. Names follow the XKB keysym convention used by sway / Hyprland `bindsym`
//! (`space`, `Caps_Lock`, `plus`, `a`, …), parsed case-insensitively via libxkbcommon, so
//! anything you can bind in your compositor you can bind here. A missing file or field
//! falls back to the built-in defaults, which reproduce the historical hardcoded layout —
//! so existing users need no config.
//!
//! Example (the defaults):
//! ```toml
//! pen = "p"
//! save = "w"
//! width-inc = ["plus", "equal"]   # a single string or a list
//! passthrough = "caps"            # a modifier (caps/ctrl/shift/alt/super) or a key
//! constrain = "ctrl"
//! spotlight = "shift"
//! ```

use serde::Deserialize;
use smithay_client_toolkit::seat::keyboard::Keysym;
use std::path::PathBuf;

/// A modifier, usable as a bindable trigger and matched against the xkb modifier state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModKind {
    Ctrl,
    Shift,
    Alt,
    Logo,
    Caps,
}

/// What a binding fires on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// A regular key.
    Key(Keysym),
    /// A modifier held (or, for Caps Lock, latched).
    Mod(ModKind),
}

/// A discrete action — fires once on key press. The held roles (pass-through, constrain,
/// spotlight) are not here; they live as the `passthrough`/`constrain`/`spotlight` fields
/// of [`Keymap`] because they can be a modifier as well as a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Pen,
    Rect,
    Mask,
    Arrow,
    Text,
    Move,
    Eraser,
    Palette,
    Undo,
    Redo,
    Visibility,
    Save,
    Help,
    Clear,
    WidthInc,
    WidthDec,
    Freeze,
}

/// Parse one trigger name. Modifier aliases first (`ctrl`, `shift`, `alt`, `super`/`logo`,
/// `caps`/`caps_lock`), otherwise an XKB keysym name (case-insensitive). Errs on an
/// unknown name so the loader can warn and keep the default.
pub fn parse_trigger(name: &str) -> Result<Trigger, String> {
    let n = name.trim();
    match n.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => return Ok(Trigger::Mod(ModKind::Ctrl)),
        "shift" => return Ok(Trigger::Mod(ModKind::Shift)),
        "alt" | "meta" => return Ok(Trigger::Mod(ModKind::Alt)),
        "super" | "logo" | "win" => return Ok(Trigger::Mod(ModKind::Logo)),
        "caps" | "caps_lock" | "capslock" => return Ok(Trigger::Mod(ModKind::Caps)),
        // Friendly aliases for the symbols the legend prettifies, so the displayed label
        // round-trips and users can write the natural symbol instead of the XKB name.
        "+" | "plus" => return Ok(Trigger::Key(Keysym::plus)),
        "-" | "minus" => return Ok(Trigger::Key(Keysym::minus)),
        "=" => return Ok(Trigger::Key(Keysym::equal)),
        "del" => return Ok(Trigger::Key(Keysym::Delete)),
        "esc" => return Ok(Trigger::Key(Keysym::Escape)),
        _ => {}
    }
    let ks = xkbcommon::xkb::keysym_from_name(n, xkbcommon::xkb::KEYSYM_CASE_INSENSITIVE);
    if ks == Keysym::NoSymbol {
        Err(format!("unknown key name `{name}`"))
    } else {
        Ok(Trigger::Key(ks))
    }
}

/// The display label for a trigger (the key column of the help legend). Modifiers get a
/// short name; keys use libxkbcommon's canonical name, with a few prettied for the HUD.
pub fn trigger_label(t: Trigger) -> String {
    match t {
        Trigger::Mod(ModKind::Ctrl) => "Ctrl".into(),
        Trigger::Mod(ModKind::Shift) => "Shift".into(),
        Trigger::Mod(ModKind::Alt) => "Alt".into(),
        Trigger::Mod(ModKind::Logo) => "Super".into(),
        Trigger::Mod(ModKind::Caps) => "Caps".into(),
        Trigger::Key(k) => match xkbcommon::xkb::keysym_get_name(k).as_str() {
            "plus" | "KP_Add" => "+".into(),
            "minus" | "KP_Subtract" => "-".into(),
            "equal" => "=".into(),
            "Delete" => "Del".into(),
            "space" | "KP_Space" => "Space".into(),
            "Escape" => "Esc".into(),
            // Single letters/digits get a capital so the column reads uniformly (`P`, not
            // `p`, next to `Del`/`Ctrl`); multi-char names keep libxkbcommon's casing.
            other if other.len() == 1 => other.to_ascii_uppercase(),
            other => other.to_string(),
        },
    }
}

/// The resolved bindings. `keys` maps discrete-action triggers (an action may have several
/// — e.g. `+` and `=`); the three held roles each carry one trigger.
#[derive(Clone)]
pub struct Keymap {
    keys: Vec<(Trigger, Action)>,
    pub passthrough: Trigger,
    pub constrain: Trigger,
    pub spotlight: Trigger,
}

impl Default for Keymap {
    /// The default hardcoded layout
    fn default() -> Self {
        let c = |ch: char| Trigger::Key(Keysym::from_char(ch));
        Keymap {
            keys: vec![
                (c('p'), Action::Pen),
                (c('r'), Action::Rect),
                (c('m'), Action::Mask),
                (c('a'), Action::Arrow),
                (c('t'), Action::Text),
                (c('s'), Action::Move),
                (c('e'), Action::Eraser),
                (c('c'), Action::Palette),
                (c('u'), Action::Undo),
                (c('y'), Action::Redo),
                (c('v'), Action::Visibility),
                (c('w'), Action::Save),
                (c('h'), Action::Help),
                (Trigger::Key(Keysym::Delete), Action::Clear),
                (Trigger::Key(Keysym::plus), Action::WidthInc),
                (Trigger::Key(Keysym::equal), Action::WidthInc),
                (Trigger::Key(Keysym::KP_Add), Action::WidthInc),
                (Trigger::Key(Keysym::minus), Action::WidthDec),
                (Trigger::Key(Keysym::KP_Subtract), Action::WidthDec),
                (Trigger::Key(Keysym::space), Action::Freeze),
                (Trigger::Key(Keysym::KP_Space), Action::Freeze),
            ],
            passthrough: Trigger::Mod(ModKind::Caps),
            constrain: Trigger::Mod(ModKind::Ctrl),
            spotlight: Trigger::Mod(ModKind::Shift),
        }
    }
}

impl Keymap {
    /// Load `~/.config/wlr-draw/keys.toml`, merging any overrides over the defaults. A
    /// missing/unreadable/malformed file leaves the defaults intact. Unlike the theme
    /// loader, bad entries are reported on stderr (a wrong bind is otherwise baffling).
    pub fn load() -> Self {
        let mut km = Keymap::default();
        let Some(raw) = config_path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| match toml::from_str::<RawConfig>(&s) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("wlr-draw: ignoring keys.toml ({e})");
                    None
                }
            })
        else {
            return km;
        };
        km.apply(raw);
        km
    }

    /// The discrete action a key press triggers, if any.
    pub fn action_for_key(&self, ks: Keysym) -> Option<Action> {
        self.keys
            .iter()
            .find(|(t, _)| *t == Trigger::Key(ks))
            .map(|(_, a)| *a)
    }

    /// The label for an action's (first) trigger — for the help legend.
    pub fn label_for(&self, action: Action) -> String {
        self.keys
            .iter()
            .find(|(_, a)| *a == action)
            .map(|(t, _)| trigger_label(*t))
            .unwrap_or_else(|| "?".into())
    }

    /// Replace every binding for `action` from a config value (or keep the default if the
    /// value is absent or has no parseable trigger).
    fn override_action(&mut self, opt: Option<OneOrMany>, action: Action, name: &str) {
        let Some(o) = opt else { return };
        let triggers: Vec<Trigger> = o
            .into_vec()
            .iter()
            .filter_map(|s| match parse_trigger(s) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("wlr-draw: {name}: {e}");
                    None
                }
            })
            .collect();
        if triggers.is_empty() {
            return; // all invalid → keep default
        }
        self.keys.retain(|(_, a)| *a != action);
        for t in triggers {
            self.keys.push((t, action));
        }
    }

    /// Set a held role from a config value, keeping the default on an unknown name.
    fn override_role(&mut self, opt: Option<String>, role: &mut Trigger, name: &str) {
        if let Some(s) = opt {
            match parse_trigger(&s) {
                Ok(t) => *role = t,
                Err(e) => eprintln!("wlr-draw: {name}: {e}"),
            }
        }
    }

    fn apply(&mut self, raw: RawConfig) {
        self.override_action(raw.pen, Action::Pen, "pen");
        self.override_action(raw.rect, Action::Rect, "rect");
        self.override_action(raw.mask, Action::Mask, "mask");
        self.override_action(raw.arrow, Action::Arrow, "arrow");
        self.override_action(raw.text, Action::Text, "text");
        self.override_action(raw.r#move, Action::Move, "move");
        self.override_action(raw.eraser, Action::Eraser, "eraser");
        self.override_action(raw.palette, Action::Palette, "palette");
        self.override_action(raw.undo, Action::Undo, "undo");
        self.override_action(raw.redo, Action::Redo, "redo");
        self.override_action(raw.visibility, Action::Visibility, "visibility");
        self.override_action(raw.save, Action::Save, "save");
        self.override_action(raw.help, Action::Help, "help");
        self.override_action(raw.clear, Action::Clear, "clear");
        self.override_action(raw.width_inc, Action::WidthInc, "width-inc");
        self.override_action(raw.width_dec, Action::WidthDec, "width-dec");
        self.override_action(raw.freeze, Action::Freeze, "freeze");

        let mut pt = self.passthrough;
        let mut co = self.constrain;
        let mut sp = self.spotlight;
        self.override_role(raw.passthrough, &mut pt, "passthrough");
        self.override_role(raw.constrain, &mut co, "constrain");
        self.override_role(raw.spotlight, &mut sp, "spotlight");
        self.passthrough = pt;
        self.constrain = co;
        self.spotlight = sp;

        self.warn_conflicts();
    }

    /// Best-effort diagnostics: the same key bound to two different actions, or a held
    /// role sharing a key with a discrete action. Doesn't change anything — just warns.
    fn warn_conflicts(&self) {
        for i in 0..self.keys.len() {
            for j in (i + 1)..self.keys.len() {
                if self.keys[i].0 == self.keys[j].0 && self.keys[i].1 != self.keys[j].1 {
                    eprintln!(
                        "wlr-draw: `{}` is bound to two actions",
                        trigger_label(self.keys[i].0)
                    );
                }
            }
        }
        for role in [self.passthrough, self.constrain, self.spotlight] {
            if matches!(role, Trigger::Key(_)) && self.keys.iter().any(|(t, _)| *t == role) {
                eprintln!(
                    "wlr-draw: `{}` is bound to both a held role and a tool",
                    trigger_label(role)
                );
            }
        }
    }
}

/// `~/.config/wlr-draw/keys.toml`, honouring `$XDG_CONFIG_HOME` (mirrors the theme loader,
/// but under wlr-draw's own config dir).
fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("wlr-draw").join("keys.toml"))
}

/// A binding value: one trigger name or a list of them.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

/// The on-disk schema. Every field optional so a partial file is valid; missing keys keep
/// their default. Keys are kebab-case (`width-inc`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "kebab-case", default)]
struct RawConfig {
    pen: Option<OneOrMany>,
    rect: Option<OneOrMany>,
    mask: Option<OneOrMany>,
    arrow: Option<OneOrMany>,
    text: Option<OneOrMany>,
    r#move: Option<OneOrMany>,
    eraser: Option<OneOrMany>,
    palette: Option<OneOrMany>,
    undo: Option<OneOrMany>,
    redo: Option<OneOrMany>,
    visibility: Option<OneOrMany>,
    save: Option<OneOrMany>,
    help: Option<OneOrMany>,
    clear: Option<OneOrMany>,
    width_inc: Option<OneOrMany>,
    width_dec: Option<OneOrMany>,
    freeze: Option<OneOrMany>,
    passthrough: Option<String>,
    constrain: Option<String>,
    spotlight: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_names_and_modifiers() {
        assert_eq!(parse_trigger("p"), Ok(Trigger::Key(Keysym::from_char('p'))));
        assert_eq!(parse_trigger("space"), Ok(Trigger::Key(Keysym::space)));
        assert_eq!(parse_trigger("Caps_Lock"), Ok(Trigger::Mod(ModKind::Caps)));
        assert_eq!(parse_trigger("CTRL"), Ok(Trigger::Mod(ModKind::Ctrl)));
        assert_eq!(parse_trigger("super"), Ok(Trigger::Mod(ModKind::Logo)));
        assert!(parse_trigger("wobble").is_err());
    }

    #[test]
    fn label_round_trips_through_parse() {
        for name in ["p", "space", "plus", "Delete", "F5"] {
            let t = parse_trigger(name).unwrap();
            // The label re-parses to the same trigger (prettied labels included).
            assert_eq!(parse_trigger(&trigger_label(t)).unwrap(), t, "{name}");
        }
    }

    #[test]
    fn default_keymap_matches_legacy_layout() {
        let km = Keymap::default();
        assert_eq!(km.action_for_key(Keysym::from_char('p')), Some(Action::Pen));
        assert_eq!(
            km.action_for_key(Keysym::from_char('w')),
            Some(Action::Save)
        );
        assert_eq!(km.action_for_key(Keysym::plus), Some(Action::WidthInc));
        assert_eq!(km.action_for_key(Keysym::equal), Some(Action::WidthInc));
        assert_eq!(km.action_for_key(Keysym::space), Some(Action::Freeze));
        assert_eq!(km.action_for_key(Keysym::from_char('z')), None);
        assert_eq!(km.passthrough, Trigger::Mod(ModKind::Caps));
        assert_eq!(km.constrain, Trigger::Mod(ModKind::Ctrl));
        assert_eq!(km.spotlight, Trigger::Mod(ModKind::Shift));
    }

    #[test]
    fn override_replaces_and_keeps_defaults() {
        let mut km = Keymap::default();
        let raw: RawConfig = toml::from_str(
            r#"
            pen = "b"
            passthrough = "alt"
            width-inc = ["plus", "equal"]
        "#,
        )
        .unwrap();
        km.apply(raw);
        // pen moved to 'b', the old 'p' is freed.
        assert_eq!(km.action_for_key(Keysym::from_char('b')), Some(Action::Pen));
        assert_eq!(km.action_for_key(Keysym::from_char('p')), None);
        // role rebound, others untouched.
        assert_eq!(km.passthrough, Trigger::Mod(ModKind::Alt));
        assert_eq!(km.constrain, Trigger::Mod(ModKind::Ctrl));
        assert_eq!(
            km.action_for_key(Keysym::from_char('w')),
            Some(Action::Save)
        );
    }
}
