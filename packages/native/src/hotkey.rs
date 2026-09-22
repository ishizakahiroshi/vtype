//! Global shortcut strings (`Ctrl+Alt+Space`) as the settings page writes them. Parsed here once;
//! each OS turns the result into its own key codes.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// `A`–`Z`.
    Letter(char),
    /// `0`–`9`.
    Digit(u8),
    /// `F1`–`F24`.
    F(u8),
    Space,
    Enter,
    Tab,
    Escape,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeySpec {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows key / Command / Super.
    pub meta: bool,
    pub key: Key,
}

fn parse_key(name: &str) -> Option<Key> {
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.chars().next()?;
        if c.is_ascii_uppercase() {
            return Some(Key::Letter(c));
        }
        if c.is_ascii_digit() {
            return Some(Key::Digit(c as u8 - b'0'));
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=24).contains(&n) {
            return Some(Key::F(n));
        }
    }
    Some(match upper.as_str() {
        "SPACE" => Key::Space,
        "ENTER" | "RETURN" => Key::Enter,
        "TAB" => Key::Tab,
        "ESC" | "ESCAPE" => Key::Escape,
        "BACKSPACE" => Key::Backspace,
        "INSERT" | "INS" => Key::Insert,
        "DELETE" | "DEL" => Key::Delete,
        "HOME" => Key::Home,
        "END" => Key::End,
        "PAGEUP" | "PGUP" => Key::PageUp,
        "PAGEDOWN" | "PGDN" => Key::PageDown,
        "UP" => Key::Up,
        "DOWN" => Key::Down,
        "LEFT" => Key::Left,
        "RIGHT" => Key::Right,
        _ => return None,
    })
}

/// `Ctrl+Alt+Space`, `control+option+v`, `Win+Shift+F9`… One key and at least one modifier: a
/// shortcut without a modifier would swallow that key in every app.
pub fn parse(spec: &str) -> Result<HotkeySpec, String> {
    let (mut ctrl, mut alt, mut shift, mut meta) = (false, false, false, false);
    let mut key = None;
    for part in spec.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(format!("{spec:?}: empty part"));
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" | "opt" => alt = true,
            "shift" => shift = true,
            "win" | "meta" | "super" | "cmd" | "command" => meta = true,
            _ => {
                if key.is_some() {
                    return Err(format!("{spec:?}: more than one key"));
                }
                key = Some(parse_key(part).ok_or_else(|| format!("{spec:?}: unknown key {part:?}"))?);
            }
        }
    }
    let key = key.ok_or_else(|| format!("{spec:?}: no key"))?;
    if !(ctrl || alt || shift || meta) {
        return Err(format!("{spec:?}: needs Ctrl, Alt, Shift or Win"));
    }
    Ok(HotkeySpec { ctrl, alt, shift, meta, key })
}

impl fmt::Display for HotkeySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.meta {
            parts.push("Win".into());
        }
        parts.push(match self.key {
            Key::Letter(c) => c.to_string(),
            Key::Digit(d) => d.to_string(),
            Key::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        });
        write!(f, "{}", parts.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_defaults() {
        let win = parse("Ctrl+Alt+Space").unwrap();
        assert!(win.ctrl && win.alt && !win.shift && !win.meta);
        assert_eq!(win.key, Key::Space);
        let mac = parse("Ctrl+Alt+V").unwrap();
        assert_eq!(mac.key, Key::Letter('V'));
        assert_eq!(parse(crate::config::default_hotkey()).unwrap().to_string(), crate::config::default_hotkey());
    }

    #[test]
    fn accepts_other_spellings() {
        assert_eq!(parse("control + option + v").unwrap(), parse("Ctrl+Alt+V").unwrap());
        assert_eq!(parse("win+shift+f9").unwrap().to_string(), "Shift+Win+F9");
        assert_eq!(parse("Ctrl+7").unwrap().key, Key::Digit(7));
        assert_eq!(parse("Alt+PgDn").unwrap().key, Key::PageDown);
    }

    #[test]
    fn refuses_what_would_not_work() {
        assert!(parse("Space").is_err()); // no modifier
        assert!(parse("Ctrl+A+B").is_err()); // two keys
        assert!(parse("Ctrl+Alt").is_err()); // no key
        assert!(parse("Ctrl++A").is_err());
        assert!(parse("Ctrl+F25").is_err());
        assert!(parse("Ctrl+Banana").is_err());
    }
}
