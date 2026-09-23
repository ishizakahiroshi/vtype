//! Typing into the foreground app: SendInput with Unicode key events, or the clipboard and
//! Ctrl+V (child plan C5-C2).

use std::thread;
use std::time::Duration;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_RETURN, VK_TAB,
};

use crate::config::{InjectMethod, SendKey};
use crate::platform::{EditKeys, InjectOutcome, PlatformError};

/// How long the pasted text stays on the clipboard before the old content comes back.
pub const PASTE_RESTORE_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// One UTF-16 code unit (a surrogate pair is two of these, in order).
    Unit(u16),
    /// A real key: Enter for a line break, Tab for a tab.
    Virtual(VIRTUAL_KEY),
}

/// What to press for `text`. `\r\n` and `\n` become one Enter; `\r` alone too.
pub fn plan_keys(text: &str) -> Vec<KeyAction> {
    let mut out = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(KeyAction::Virtual(VK_RETURN));
            }
            '\n' => out.push(KeyAction::Virtual(VK_RETURN)),
            '\t' => out.push(KeyAction::Virtual(VK_TAB)),
            _ => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push(KeyAction::Unit(*unit));
                }
            }
        }
    }
    out
}

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn inputs_for(actions: &[KeyAction]) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(actions.len() * 2);
    for action in actions {
        match *action {
            KeyAction::Unit(unit) => {
                inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
                inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
            }
            KeyAction::Virtual(vk) => {
                inputs.push(key_input(vk, 0, 0));
                inputs.push(key_input(vk, 0, KEYEVENTF_KEYUP));
            }
        }
    }
    inputs
}

/// Sends the events; returns how many Windows accepted. Fewer than asked means another input
/// source (or UIPI, for an elevated window) blocked them.
fn send(inputs: &[INPUT]) -> usize {
    if inputs.is_empty() {
        return 0;
    }
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) as usize }
}

/// True when every event of the text went through.
pub fn type_text(text: &str) -> bool {
    let inputs = inputs_for(&plan_keys(text));
    let sent = send(&inputs);
    sent == inputs.len()
}

/// Puts the text on the clipboard, presses Ctrl+V, and brings back what was there (text only;
/// anything else cannot be read back through this API and is left replaced, which is logged).
pub fn paste_text(text: &str) -> Result<(), PlatformError> {
    let mut clipboard = arboard::Clipboard::new().map_err(PlatformError::failed)?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text.to_string()).map_err(PlatformError::failed)?;
    let v = 'V' as VIRTUAL_KEY;
    let inputs = [
        key_input(VK_CONTROL, 0, 0),
        key_input(v, 0, 0),
        key_input(v, 0, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP),
    ];
    let sent = send(&inputs);
    thread::sleep(PASTE_RESTORE_DELAY);
    match previous {
        Some(old) => {
            let _ = clipboard.set_text(old);
        }
        None => tracing::info!("the clipboard held no text before; it keeps the pasted text"),
    }
    if sent == inputs.len() {
        Ok(())
    } else {
        Err(PlatformError::Failed("Ctrl+V was blocked".into()))
    }
}

/// Presses `vk` with Ctrl held when `ctrl`.
fn chord(vk: VIRTUAL_KEY, ctrl: bool) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(4);
    if ctrl {
        inputs.push(key_input(VK_CONTROL, 0, 0));
    }
    inputs.push(key_input(vk, 0, 0));
    inputs.push(key_input(vk, 0, KEYEVENTF_KEYUP));
    if ctrl {
        inputs.push(key_input(VK_CONTROL, 0, KEYEVENTF_KEYUP));
    }
    inputs
}

/// Ctrl+C, to copy the foreground app's selection.
pub fn press_copy() -> Result<(), PlatformError> {
    let inputs = chord('C' as VIRTUAL_KEY, true);
    if send(&inputs) == inputs.len() {
        Ok(())
    } else {
        Err(PlatformError::Failed("Ctrl+C was blocked".into()))
    }
}

/// The floating mic's buttons: Ctrl+A then Backspace, or the send key.
pub fn press_keys(keys: EditKeys) -> Result<(), PlatformError> {
    let inputs = match keys {
        EditKeys::ClearField => [chord('A' as VIRTUAL_KEY, true), chord(VK_BACK, false)].concat(),
        EditKeys::Send(SendKey::Enter) => chord(VK_RETURN, false),
        EditKeys::Send(SendKey::CtrlEnter) => chord(VK_RETURN, true),
    };
    let sent = send(&inputs);
    // TEMP(clear-ime): how many of the key events Windows took; remove after the check.
    tracing::info!(?keys, sent, asked = inputs.len(), "edit keys");
    if sent == inputs.len() {
        Ok(())
    } else {
        Err(PlatformError::Failed("the keys were blocked".into()))
    }
}

/// `Auto` types, and pastes when typing did not go through.
pub fn should_fall_back_to_paste(method: InjectMethod, typed_all: bool) -> bool {
    method == InjectMethod::Auto && !typed_all
}

pub fn inject(text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
    if method == InjectMethod::Paste {
        return paste_text(text).map(|()| InjectOutcome::Pasted);
    }
    let typed_all = type_text(text);
    if typed_all {
        return Ok(InjectOutcome::Typed);
    }
    if should_fall_back_to_paste(method, typed_all) {
        tracing::info!("typing was blocked; pasting instead");
        return paste_text(text).map(|()| InjectOutcome::Pasted);
    }
    Err(PlatformError::Failed("typing was blocked".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_japanese_emoji_and_line_breaks() {
        let plan = plan_keys("あ🎤\nb\r\nc\td");
        assert_eq!(
            plan,
            vec![
                KeyAction::Unit(0x3042),
                KeyAction::Unit(0xD83C), // 🎤 is a surrogate pair
                KeyAction::Unit(0xDFA4),
                KeyAction::Virtual(VK_RETURN),
                KeyAction::Unit(b'b' as u16),
                KeyAction::Virtual(VK_RETURN),
                KeyAction::Unit(b'c' as u16),
                KeyAction::Virtual(VK_TAB),
                KeyAction::Unit(b'd' as u16),
            ]
        );
    }

    #[test]
    fn every_key_is_pressed_and_released() {
        let inputs = inputs_for(&plan_keys("ab"));
        assert_eq!(inputs.len(), 4);
        let flags: Vec<u32> = inputs.iter().map(|i| unsafe { i.Anonymous.ki.dwFlags }).collect();
        assert_eq!(
            flags,
            vec![
                KEYEVENTF_UNICODE,
                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                KEYEVENTF_UNICODE,
                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
            ]
        );
    }

    #[test]
    fn only_auto_falls_back_to_pasting() {
        assert!(should_fall_back_to_paste(InjectMethod::Auto, false));
        assert!(!should_fall_back_to_paste(InjectMethod::Auto, true));
        assert!(!should_fall_back_to_paste(InjectMethod::Type, false));
    }
}
