//! What the tray and the global shortcut look like on every system that uses `tray-icon` and
//! `global-hotkey`: the menu built from `crate::menu`, the tray icons, and our shortcut spelled the
//! way `global-hotkey` wants it.

use std::collections::HashMap;
use std::sync::Mutex;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::Icon;

use crate::hotkey::{HotkeySpec, Key};
use crate::icon_draw::{draw_icon, to_straight_rgba};
use crate::menu::{tray_menu, MenuItem as Item};
use crate::platform::{IconState, MenuAction, TrayState};

fn code_for(key: Key) -> Option<Code> {
    use Code::*;
    Some(match key {
        Key::Letter(c) => match c {
            'A' => KeyA,
            'B' => KeyB,
            'C' => KeyC,
            'D' => KeyD,
            'E' => KeyE,
            'F' => KeyF,
            'G' => KeyG,
            'H' => KeyH,
            'I' => KeyI,
            'J' => KeyJ,
            'K' => KeyK,
            'L' => KeyL,
            'M' => KeyM,
            'N' => KeyN,
            'O' => KeyO,
            'P' => KeyP,
            'Q' => KeyQ,
            'R' => KeyR,
            'S' => KeyS,
            'T' => KeyT,
            'U' => KeyU,
            'V' => KeyV,
            'W' => KeyW,
            'X' => KeyX,
            'Y' => KeyY,
            'Z' => KeyZ,
            _ => return None,
        },
        Key::Digit(d) => {
            [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9].get(d as usize).copied()?
        }
        Key::F(n) => [
            F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23,
            F24,
        ]
        .get(n as usize - 1)
        .copied()?,
        Key::Space => Space,
        Key::Enter => Enter,
        Key::Tab => Tab,
        Key::Escape => Escape,
        Key::Backspace => Backspace,
        Key::Insert => Insert,
        Key::Delete => Delete,
        Key::Home => Home,
        Key::End => End,
        Key::PageUp => PageUp,
        Key::PageDown => PageDown,
        Key::Up => ArrowUp,
        Key::Down => ArrowDown,
        Key::Left => ArrowLeft,
        Key::Right => ArrowRight,
    })
}

pub fn to_global_hotkey(spec: &HotkeySpec) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    if spec.ctrl {
        mods |= Modifiers::CONTROL;
    }
    if spec.alt {
        mods |= Modifiers::ALT;
    }
    if spec.shift {
        mods |= Modifiers::SHIFT;
    }
    if spec.meta {
        mods |= Modifiers::SUPER;
    }
    Some(HotKey::new(Some(mods), code_for(spec.key)?))
}

/// The tray icon at 32 px: the app's own icon, or the orange mic while recording.
pub fn tray_icons() -> (Option<Icon>, Option<Icon>) {
    let idle = tiny_skia::Pixmap::decode_png(include_bytes!("../../../../assets/icons/favicon-32.png"))
        .ok()
        .and_then(|pm| Icon::from_rgba(to_straight_rgba(&pm), pm.width(), pm.height()).ok());
    let rec = draw_icon(32, IconState::Recording, true);
    let recording = Icon::from_rgba(to_straight_rgba(&rec), 32, 32).ok();
    (idle, recording)
}

/// The tray menu, with each entry's id noted in `actions` so a menu event can be told apart.
pub fn build_menu(actions: &Mutex<HashMap<String, MenuAction>>) -> (Menu, Vec<(MenuAction, CheckMenuItem)>) {
    build_items(tray_menu(&TrayState::default()), actions)
}

fn build_items(
    items: Vec<Item>,
    actions: &Mutex<HashMap<String, MenuAction>>,
) -> (Menu, Vec<(MenuAction, CheckMenuItem)>) {
    let menu = Menu::new();
    let mut checks = Vec::new();
    let mut actions = actions.lock().unwrap_or_else(|e| e.into_inner());
    for item in items {
        match item {
            Item::Check { action, label, checked } | Item::Radio { action, label, checked } => {
                let entry = CheckMenuItem::new(label, true, checked, None);
                actions.insert(entry.id().0.clone(), action);
                let _ = menu.append(&entry);
                checks.push((action, entry));
            }
            Item::Action { action, label } => {
                let entry = MenuItem::new(label, true, None);
                actions.insert(entry.id().0.clone(), action);
                let _ = menu.append(&entry);
            }
            Item::Separator => {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
        }
    }
    (menu, checks)
}

/// Copies the foreground app's selection with `press_copy` (Ctrl+C / Cmd+C) and puts the
/// clipboard back. The clipboard is emptied first, so "nothing selected" is not mistaken for the
/// text that was already on it. Like pasting, only text is put back: an image on the clipboard is
/// lost.
pub fn copy_selection_with(
    press_copy: impl FnOnce() -> Result<(), crate::platform::PlatformError>,
) -> Result<Option<String>, crate::platform::PlatformError> {
    use crate::platform::PlatformError;
    let mut clipboard = arboard::Clipboard::new().map_err(PlatformError::failed)?;
    let previous = clipboard.get_text().ok();
    let _ = clipboard.clear();
    let pressed = press_copy();
    // The app answers the copy on its own time.
    let mut copied = None;
    if pressed.is_ok() {
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(30));
            if let Ok(text) = clipboard.get_text() {
                if !text.is_empty() {
                    copied = Some(text);
                    break;
                }
            }
        }
    }
    match previous {
        Some(old) => {
            let _ = clipboard.set_text(old);
        }
        None => {
            let _ = clipboard.clear();
        }
    }
    pressed.map(|()| copied)
}

/// Ticks the check and radio entries to match `state`.
pub fn update_checks(checks: &[(MenuAction, CheckMenuItem)], state: &TrayState) {
    for (action, item) in checks {
        let checked = match action {
            MenuAction::ToggleIconVisible => state.icon_visible,
            MenuAction::ToggleHideOnFullscreen => state.hide_on_fullscreen,
            MenuAction::SetMode(m) => state.mode == *m,
            _ => continue,
        };
        item.set_checked(checked);
    }
}

/// Milliseconds from a `FieldChanged` report to the beside mic on screen (child plan C8).
#[cfg(any(windows, target_os = "macos"))]
#[derive(Default)]
pub struct Timing {
    count: u32,
    last_ms: u128,
    max_ms: u128,
}

#[cfg(any(windows, target_os = "macos"))]
impl Timing {
    pub fn record(&mut self, ms: u128) {
        self.count += 1;
        self.last_ms = ms;
        self.max_ms = self.max_ms.max(ms);
    }

    pub fn describe(&self) -> Option<String> {
        (self.count > 0).then(|| format!("last {}ms, max {}ms, {} times", self.last_ms, self.max_ms, self.count))
    }
}

/// The beside mic's watcher. The daemon may ask for it before the event loop (and with it the
/// event sender) is up; it starts once both are there. Dropping a watcher stops it.
#[cfg(any(windows, target_os = "macos"))]
pub struct BesideControl<W> {
    events: Option<std::sync::mpsc::Sender<crate::platform::PlatformEvent>>,
    config: Option<crate::config::BesideFieldConfig>,
    watcher: Option<W>,
    start: fn(crate::config::BesideFieldTrigger, std::sync::mpsc::Sender<crate::platform::PlatformEvent>) -> W,
}

#[cfg(any(windows, target_os = "macos"))]
impl<W> BesideControl<W> {
    pub fn new(
        start: fn(crate::config::BesideFieldTrigger, std::sync::mpsc::Sender<crate::platform::PlatformEvent>) -> W,
    ) -> Self {
        BesideControl { events: None, config: None, watcher: None, start }
    }

    fn restart(&mut self) {
        self.watcher = None;
        if let (Some(events), Some(config)) = (&self.events, &self.config) {
            if config.enabled {
                self.watcher = Some((self.start)(config.trigger, events.clone()));
            }
        }
    }

    pub fn set_events(&mut self, events: std::sync::mpsc::Sender<crate::platform::PlatformEvent>) {
        self.events = Some(events);
        self.restart();
    }

    pub fn set_config(&mut self, config: &crate::config::BesideFieldConfig) {
        if self.config.as_ref() != Some(config) {
            self.config = Some(config.clone());
            self.restart();
        }
    }

    pub fn stop(&mut self) {
        self.watcher = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse;

    #[test]
    fn maps_the_default_shortcuts() {
        let hk = to_global_hotkey(&parse("Ctrl+Alt+Space").unwrap()).unwrap();
        assert_eq!(hk, HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space));
        let mac = to_global_hotkey(&parse("Ctrl+Alt+V").unwrap()).unwrap();
        assert_eq!(mac, HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyV));
        let f = to_global_hotkey(&parse("Win+Shift+F24").unwrap()).unwrap();
        assert_eq!(f, HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::F24));
        assert!(to_global_hotkey(&parse("Ctrl+9").unwrap()).is_some());
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn the_watcher_starts_once_it_has_both_a_sender_and_an_enabled_config() {
        use crate::config::{BesideFieldConfig, BesideFieldTrigger};
        fn start(
            trigger: BesideFieldTrigger,
            _events: std::sync::mpsc::Sender<crate::platform::PlatformEvent>,
        ) -> BesideFieldTrigger {
            trigger
        }
        let mut c = BesideControl::new(start);
        let on = BesideFieldConfig { enabled: true, trigger: BesideFieldTrigger::Hover };
        c.set_config(&on);
        assert!(c.watcher.is_none(), "no sender yet");
        let (tx, _rx) = std::sync::mpsc::channel();
        c.set_events(tx);
        assert_eq!(c.watcher, Some(BesideFieldTrigger::Hover));
        c.set_config(&BesideFieldConfig { enabled: false, ..on });
        assert!(c.watcher.is_none());
        let mut t = Timing::default();
        assert_eq!(t.describe(), None);
        t.record(40);
        t.record(12);
        assert_eq!(t.describe().as_deref(), Some("last 12ms, max 40ms, 2 times"));
    }
}
