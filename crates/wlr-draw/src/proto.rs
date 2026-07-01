//! The control protocol carried over the daemon's Unix socket.
//!
//! One command per line, a verb plus an optional argument: `toggle`, `clear`, `undo`,
//! `redo`, `visibility`, `tool pen`, `color #ff0000`, `width 6`, `quit`. The client
//! ([`crate::ipc::send`]) serializes a [`Cmd`] with [`Cmd::to_line`]; the daemon's
//! socket thread parses each line with [`Cmd::parse`] and feeds the [`Cmd`] into the
//! event loop. Text, not a binary format — trivial to send by hand (`socat`, `echo`)
//! and to read in logs.

use crate::model::{Color, Tool, parse_color};

/// A control command. Mirrors the `wlr-draw <verb>` subcommands.
#[derive(Clone, PartialEq, Debug)]
pub enum Cmd {
    /// Flip between draw mode and click-through.
    Toggle,
    /// Enter draw mode (grab input).
    On,
    /// Leave draw mode (click-through).
    Off,
    /// Erase all annotations.
    Clear,
    Undo,
    Redo,
    /// Hide / show the annotations without discarding them.
    Visibility,
    /// Select a drawing tool.
    Tool(Tool),
    /// Set the stroke colour.
    Color(Color),
    /// Set the stroke width (logical px).
    Width(f32),
    /// Save the annotated screen (the output under the cursor) to a PNG. With no path,
    /// a timestamped file in the user's Pictures directory.
    Save(Option<String>),
    /// Stop the daemon.
    Quit,
}

impl Cmd {
    /// Parse one protocol line. Errors carry a human-readable reason (sent back to the
    /// client as `err <reason>`).
    pub fn parse(line: &str) -> Result<Cmd, String> {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let Some(verb) = it.next() else {
            return Err("empty command".into());
        };
        let arg = it.next();
        let need = |what: &str| arg.ok_or_else(|| format!("{verb} needs {what}"));
        Ok(match verb {
            "toggle" => Cmd::Toggle,
            "on" => Cmd::On,
            "off" => Cmd::Off,
            "clear" => Cmd::Clear,
            "undo" => Cmd::Undo,
            "redo" => Cmd::Redo,
            "visibility" | "hide" | "show" => Cmd::Visibility,
            // The path may contain spaces, so take the whole remainder of the line
            // rather than a single whitespace-split token.
            "save" | "screenshot" => {
                let rest = line[verb.len()..].trim_start();
                Cmd::Save((!rest.is_empty()).then(|| rest.to_string()))
            }
            "quit" | "exit" => Cmd::Quit,
            "tool" => {
                let a = need("a tool name")?;
                Cmd::Tool(Tool::from_name(a).ok_or_else(|| format!("unknown tool: {a}"))?)
            }
            "color" | "colour" => {
                let a = need("a colour")?;
                Cmd::Color(parse_color(a).ok_or_else(|| format!("unknown colour: {a}"))?)
            }
            "width" => {
                let a = need("a number")?;
                Cmd::Width(a.parse::<f32>().map_err(|_| format!("bad width: {a}"))?)
            }
            other => return Err(format!("unknown command: {other}")),
        })
    }

    /// Serialize to a single protocol line (no trailing newline).
    pub fn to_line(&self) -> String {
        match self {
            Cmd::Toggle => "toggle".into(),
            Cmd::On => "on".into(),
            Cmd::Off => "off".into(),
            Cmd::Clear => "clear".into(),
            Cmd::Undo => "undo".into(),
            Cmd::Redo => "redo".into(),
            Cmd::Visibility => "visibility".into(),
            Cmd::Quit => "quit".into(),
            Cmd::Tool(t) => format!("tool {}", t.name()),
            Cmd::Color([r, g, b, a]) => format!("color #{r:02x}{g:02x}{b:02x}{a:02x}"),
            Cmd::Width(w) => format!("width {w}"),
            Cmd::Save(None) => "save".into(),
            Cmd::Save(Some(p)) => format!("save {p}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let cases = [
            Cmd::Toggle,
            Cmd::On,
            Cmd::Off,
            Cmd::Clear,
            Cmd::Undo,
            Cmd::Redo,
            Cmd::Visibility,
            Cmd::Quit,
            Cmd::Tool(Tool::Arrow),
            Cmd::Color([0xff, 0x3b, 0x30, 0xff]),
            Cmd::Width(6.0),
            Cmd::Save(None),
            Cmd::Save(Some("/home/me/Pictures/shot.png".into())),
            // A path with spaces must survive the round-trip: `save` takes the rest
            // of the line, not a single whitespace-split token.
            Cmd::Save(Some("/home/me/My Pictures/a b.png".into())),
        ];
        for c in cases {
            assert_eq!(Cmd::parse(&c.to_line()), Ok(c.clone()), "{c:?}");
        }
    }

    #[test]
    fn aliases_and_errors() {
        assert_eq!(
            Cmd::parse("colour red"),
            Ok(Cmd::Color([0xff, 0x3b, 0x30, 0xff]))
        );
        assert_eq!(Cmd::parse("hide"), Ok(Cmd::Visibility));
        assert!(Cmd::parse("").is_err());
        assert!(Cmd::parse("tool").is_err());
        assert!(Cmd::parse("tool wobble").is_err());
        assert!(Cmd::parse("width xyz").is_err());
        assert!(Cmd::parse("frobnicate").is_err());
    }
}
