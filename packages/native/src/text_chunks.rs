//! Text cut into key events for systems that take a short Unicode string per event (macOS takes
//! at most 20 UTF-16 units in one `CGEventKeyboardSetUnicodeString`). Line breaks and tabs
//! become real keys. A surrogate pair is never split across two events.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyStep {
    /// Up to the limit of UTF-16 units, sent as one key press.
    Text(Vec<u16>),
    Enter,
    Tab,
}

/// What macOS accepts in one keyboard event.
pub const MAC_MAX_UNITS: usize = 20;

/// `\r\n`, `\n` and `\r` alone each become one Enter.
pub fn key_steps(text: &str, max_units: usize) -> Vec<KeyStep> {
    let max_units = max_units.max(2); // room for one surrogate pair
    let mut out = Vec::new();
    let mut current: Vec<u16> = Vec::new();
    let mut chars = text.chars().peekable();
    let flush = |current: &mut Vec<u16>, out: &mut Vec<KeyStep>| {
        if !current.is_empty() {
            out.push(KeyStep::Text(std::mem::take(current)));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                flush(&mut current, &mut out);
                out.push(KeyStep::Enter);
            }
            '\t' => {
                flush(&mut current, &mut out);
                out.push(KeyStep::Tab);
            }
            _ => {
                let mut buf = [0u16; 2];
                let units = c.encode_utf16(&mut buf);
                if current.len() + units.len() > max_units {
                    flush(&mut current, &mut out);
                }
                current.extend_from_slice(units);
            }
        }
    }
    flush(&mut current, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn cuts_every_twenty_units() {
        let text = "a".repeat(45);
        let steps = key_steps(&text, MAC_MAX_UNITS);
        let lens: Vec<usize> = steps
            .iter()
            .map(|s| match s {
                KeyStep::Text(u) => u.len(),
                _ => 0,
            })
            .collect();
        assert_eq!(lens, vec![20, 20, 5]);
    }

    #[test]
    fn never_splits_a_surrogate_pair() {
        // 19 units, then 🎤 (two units) does not fit in the first event.
        let text = format!("{}🎤x", "a".repeat(19));
        let steps = key_steps(&text, MAC_MAX_UNITS);
        assert_eq!(steps, vec![KeyStep::Text(units(&"a".repeat(19))), KeyStep::Text(units("🎤x"))]);
    }

    #[test]
    fn line_breaks_and_tabs_are_keys() {
        let steps = key_steps("あい\nう\r\nえ\tお\r", MAC_MAX_UNITS);
        assert_eq!(
            steps,
            vec![
                KeyStep::Text(units("あい")),
                KeyStep::Enter,
                KeyStep::Text(units("う")),
                KeyStep::Enter,
                KeyStep::Text(units("え")),
                KeyStep::Tab,
                KeyStep::Text(units("お")),
                KeyStep::Enter,
            ]
        );
        assert!(key_steps("", MAC_MAX_UNITS).is_empty());
    }
}
