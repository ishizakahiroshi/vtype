//! Typing into the front app (child plan C6-C2): CGEvent key presses that carry up to 20 UTF-16
//! units each, or the clipboard and Cmd+V. The caller has checked the Accessibility permission;
//! without it macOS drops these events without saying so.

use std::thread;
use std::time::Duration;

use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};

use crate::config::{InjectMethod, SendKey};
use crate::platform::{EditKeys, InjectOutcome, PlatformError};
use crate::text_chunks::{key_steps, KeyStep, MAC_MAX_UNITS};

/// How long the pasted text stays on the clipboard before the old content comes back.
const PASTE_RESTORE_DELAY: Duration = Duration::from_millis(300);
/// A breath between events, so a busy app does not lose one.
const EVENT_GAP: Duration = Duration::from_millis(2);

// Virtual key codes (Carbon's kVK_*), the same on every keyboard layout.
const KEY_RETURN: CGKeyCode = 36;
const KEY_TAB: CGKeyCode = 48;
const KEY_V: CGKeyCode = 9;
const KEY_A: CGKeyCode = 0;
const KEY_C: CGKeyCode = 8;
const KEY_DELETE: CGKeyCode = 51;

/// Cmd+C, to copy the front app's selection.
pub fn press_copy() -> Result<(), PlatformError> {
    if press(KEY_C, None, CGEventFlags::MaskCommand) {
        Ok(())
    } else {
        Err(PlatformError::Failed("Cmd+C could not be sent".into()))
    }
}

/// The floating mic's buttons: Cmd+A then Delete (Backspace), or the send key (Ctrl+Enter is
/// Cmd+Enter on macOS).
pub fn press_keys(keys: EditKeys) -> Result<(), PlatformError> {
    let none = CGEventFlags(0);
    let ok = match keys {
        EditKeys::ClearField => press(KEY_A, None, CGEventFlags::MaskCommand) && press(KEY_DELETE, None, none),
        EditKeys::Send(SendKey::Enter) => press(KEY_RETURN, None, none),
        EditKeys::Send(SendKey::CtrlEnter) => press(KEY_RETURN, None, CGEventFlags::MaskCommand),
    };
    if ok {
        Ok(())
    } else {
        Err(PlatformError::Failed("the keys could not be sent".into()))
    }
}

/// Presses and releases `key`. With `text`, the press types that text instead of the key's own
/// character (the key code is then only a carrier). No modifier flags, so a still-held Control or
/// Option from the shortcut does not turn the text into shortcuts.
fn press(key: CGKeyCode, text: Option<&[u16]>, flags: CGEventFlags) -> bool {
    for down in [true, false] {
        let Some(event) = CGEvent::new_keyboard_event(None, key, down) else {
            return false;
        };
        CGEvent::set_flags(Some(&event), flags);
        if let Some(units) = text {
            unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr()) };
        }
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        thread::sleep(EVENT_GAP);
    }
    true
}

/// True when every event could be made (macOS does not report whether an app took them).
pub fn type_text(text: &str) -> bool {
    let none = CGEventFlags(0);
    key_steps(text, MAC_MAX_UNITS).iter().all(|step| match step {
        KeyStep::Text(units) => press(0, Some(units), none),
        KeyStep::Enter => press(KEY_RETURN, None, none),
        KeyStep::Tab => press(KEY_TAB, None, none),
    })
}

enum PreviousClipboard {
    Text(String),
    Image(arboard::ImageData<'static>),
    Empty,
}

/// Puts the text on the clipboard, presses Cmd+V, and brings back what was there (text or image;
/// cleared if it held neither, so typed voice text does not linger on the clipboard).
pub fn paste_text(text: &str) -> Result<(), PlatformError> {
    let mut clipboard = arboard::Clipboard::new().map_err(PlatformError::failed)?;
    let previous = match clipboard.get_text() {
        Ok(t) => PreviousClipboard::Text(t),
        Err(_) => match clipboard.get_image() {
            Ok(img) => PreviousClipboard::Image(arboard::ImageData {
                width: img.width,
                height: img.height,
                bytes: std::borrow::Cow::Owned(img.bytes.into_owned()),
            }),
            Err(_) => PreviousClipboard::Empty,
        },
    };
    clipboard.set_text(text.to_string()).map_err(PlatformError::failed)?;
    let pressed = press(KEY_V, None, CGEventFlags::MaskCommand);
    thread::sleep(PASTE_RESTORE_DELAY);
    match previous {
        PreviousClipboard::Text(old) => {
            let _ = clipboard.set_text(old);
        }
        PreviousClipboard::Image(img) => {
            let _ = clipboard.set_image(img);
        }
        PreviousClipboard::Empty => {
            let _ = clipboard.clear();
        }
    }
    if pressed {
        Ok(())
    } else {
        Err(PlatformError::Failed("Cmd+V could not be sent".into()))
    }
}

pub fn inject(text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
    if method == InjectMethod::Paste {
        return paste_text(text).map(|()| InjectOutcome::Pasted);
    }
    if type_text(text) {
        return Ok(InjectOutcome::Typed);
    }
    if method == InjectMethod::Auto {
        tracing::info!("typing failed; pasting instead");
        return paste_text(text).map(|()| InjectOutcome::Pasted);
    }
    Err(PlatformError::Failed("typing failed".into()))
}
