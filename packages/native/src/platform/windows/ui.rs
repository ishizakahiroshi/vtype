//! The UI thread: one Win32 message loop that owns the tray icon, the global shortcut and the
//! overlay windows (all three need the thread that created them). Other threads hand it work
//! through a queue and a posted message (`WM_APP_RUN`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{CheckMenuItem, ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_BUSY, QUNS_PRESENTATION_MODE,
    QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetTimer, TranslateMessage, HWND_MESSAGE, MSG, WM_APP, WM_TIMER, WNDCLASSEXW,
};

use super::overlay::{wide, Overlay};
use crate::hotkey::{HotkeySpec, Key};
use crate::icon_draw::{draw_icon, to_straight_rgba};
use crate::menu::{tooltip, tray_menu, MenuItem as Item};
use crate::platform::{IconState, MenuAction, PlatformError, PlatformEvent, TrayState};

pub const WM_APP_RUN: u32 = WM_APP + 1;
pub const WM_APP_MENU: u32 = WM_APP + 2;
const TIMER_FULLSCREEN: usize = 1;
const TIMER_DONE: usize = 2;
const TIMER_BUBBLE: usize = 3;
/// How long the green check stays after text went in.
const DONE_MS: u32 = 800;
/// The bubble goes away this long after the last interim text.
const BUBBLE_IDLE_MS: u32 = 1500;

type Job = Box<dyn FnOnce(&mut Ui) + Send>;

/// Shared between the UI thread and everyone who sends it work.
#[derive(Default)]
pub struct Shared {
    queue: Mutex<Vec<Job>>,
    msg_hwnd: AtomicIsize,
    menu_actions: Mutex<HashMap<String, MenuAction>>,
    hotkey_id: AtomicU32,
}

impl Shared {
    /// Runs `job` on the UI thread (later, if its loop is not up yet).
    pub fn run(&self, job: impl FnOnce(&mut Ui) + Send + 'static) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(job));
        let hwnd = self.msg_hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            unsafe { PostMessageW(hwnd as HWND, WM_APP_RUN, 0, 0) };
        }
    }

    /// Runs `job` on the UI thread and waits for its answer.
    pub fn call<R: Send + 'static>(&self, job: impl FnOnce(&mut Ui) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.run(move |ui| {
            let _ = tx.send(job(ui));
        });
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }
}

pub struct Ui {
    shared: Arc<Shared>,
    msg_hwnd: HWND,
    tray: Option<TrayIcon>,
    menu: Menu,
    checks: Vec<(MenuAction, CheckMenuItem)>,
    idle_icon: Option<Icon>,
    recording_icon: Option<Icon>,
    hotkeys: Option<GlobalHotKeyManager>,
    current_hotkey: Option<HotKey>,
    overlay: Overlay,
    icon_wanted: bool,
    icon_position: Option<(i32, i32)>,
    hide_on_fullscreen: bool,
    hidden_for_fullscreen: bool,
    recording: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Runs queued jobs one at a time, each with its own borrow of the UI state.
fn drain(shared: &Shared) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            if q.is_empty() {
                return;
            }
            q.remove(0)
        };
        UI.with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                if let Some(ui) = guard.as_mut() {
                    job(ui);
                }
            }
        });
    }
}

fn with_ui(f: impl FnOnce(&mut Ui)) {
    UI.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            if let Some(ui) = guard.as_mut() {
                f(ui);
            }
        }
    });
}

unsafe extern "system" fn msg_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_RUN => {
            let shared = UI.with(|slot| slot.try_borrow().ok().and_then(|g| g.as_ref().map(|u| u.shared.clone())));
            if let Some(shared) = shared {
                drain(&shared);
            }
            0
        }
        WM_APP_MENU => {
            // Clone what is needed and let go of the borrow: the menu runs a modal loop.
            let target = UI.with(|slot| {
                slot.try_borrow().ok().and_then(|g| g.as_ref().map(|u| (u.menu.clone(), u.overlay.hwnd())))
            });
            if let Some((menu, owner)) = target {
                menu.show_context_menu_for_hwnd(owner as isize, None);
            }
            0
        }
        WM_TIMER => {
            match wparam {
                TIMER_FULLSCREEN => with_ui(|ui| ui.check_fullscreen()),
                TIMER_DONE => {
                    KillTimer(hwnd, TIMER_DONE);
                    with_ui(|ui| {
                        if ui.overlay.look() == IconState::Done {
                            ui.overlay.set_look(IconState::Idle);
                        }
                    });
                }
                TIMER_BUBBLE => {
                    KillTimer(hwnd, TIMER_BUBBLE);
                    with_ui(|ui| ui.overlay.hide_bubble());
                }
                _ => {}
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

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
fn tray_icons() -> (Option<Icon>, Option<Icon>) {
    let idle = tiny_skia::Pixmap::decode_png(include_bytes!("../../../../../assets/icons/favicon-32.png"))
        .ok()
        .and_then(|pm| Icon::from_rgba(to_straight_rgba(&pm), pm.width(), pm.height()).ok());
    let rec = draw_icon(32, IconState::Recording, true);
    let recording = Icon::from_rgba(to_straight_rgba(&rec), 32, 32).ok();
    (idle, recording)
}

impl Ui {
    pub fn set_tray(&mut self, state: TrayState) {
        for (action, item) in &self.checks {
            let checked = match action {
                MenuAction::ToggleIconVisible => state.icon_visible,
                MenuAction::ToggleHideOnFullscreen => state.hide_on_fullscreen,
                MenuAction::SetMode(m) => state.mode == *m,
                _ => continue,
            };
            item.set_checked(checked);
        }
        self.hide_on_fullscreen = state.hide_on_fullscreen;
        if state.recording != self.recording {
            self.recording = state.recording;
            if let Some(tray) = &self.tray {
                let icon = if state.recording { self.recording_icon.clone() } else { self.idle_icon.clone() };
                let _ = tray.set_icon(icon);
            }
        }
        if !self.hide_on_fullscreen && self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = false;
            if self.icon_wanted {
                self.overlay.show(self.overlay.look(), self.icon_position);
            }
        }
    }

    pub fn register_hotkey(&mut self, spec: &HotkeySpec) -> Result<(), PlatformError> {
        let manager = match &self.hotkeys {
            Some(m) => m,
            None => {
                self.hotkeys = Some(GlobalHotKeyManager::new().map_err(PlatformError::failed)?);
                self.hotkeys.as_ref().unwrap()
            }
        };
        if let Some(old) = self.current_hotkey.take() {
            let _ = manager.unregister(old);
        }
        let hotkey = to_global_hotkey(spec).ok_or_else(|| PlatformError::Failed(format!("{spec} is not supported")))?;
        manager.register(hotkey).map_err(PlatformError::failed)?;
        self.shared.hotkey_id.store(hotkey.id(), Ordering::Release);
        self.current_hotkey = Some(hotkey);
        Ok(())
    }

    pub fn show_icon(&mut self, look: IconState, position: Option<(i32, i32)>) {
        self.icon_wanted = true;
        self.icon_position = position;
        if self.hidden_for_fullscreen {
            self.overlay.set_look(look);
            return;
        }
        if self.overlay.is_shown() {
            self.overlay.set_look(look);
        } else {
            self.overlay.show(look, position);
        }
        unsafe {
            if look == IconState::Done {
                SetTimer(self.msg_hwnd, TIMER_DONE, DONE_MS, None);
            }
        }
    }

    pub fn hide_icon(&mut self) {
        self.icon_wanted = false;
        self.overlay.hide();
    }

    pub fn show_bubble(&mut self, text: &str) {
        if !self.overlay.is_shown() {
            return; // no mic to speak from (hidden, or over a full-screen app)
        }
        self.overlay.show_bubble(text);
        unsafe { SetTimer(self.msg_hwnd, TIMER_BUBBLE, BUBBLE_IDLE_MS, None) };
    }

    pub fn hide_bubble(&mut self) {
        self.overlay.hide_bubble();
    }

    pub fn overlay_hwnd(&self) -> HWND {
        self.overlay.hwnd()
    }

    /// Hides the mic while a full-screen app (a game, a video, a presentation) is in front.
    fn check_fullscreen(&mut self) {
        if !self.hide_on_fullscreen || !self.icon_wanted {
            return;
        }
        let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
        let ok = unsafe { SHQueryUserNotificationState(&mut state) } >= 0;
        let busy = ok && matches!(state, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE);
        if busy && !self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = true;
            self.overlay.hide();
        } else if !busy && self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = false;
            self.overlay.show(self.overlay.look(), self.icon_position);
        }
    }
}

fn build_menu(shared: &Shared) -> (Menu, Vec<(MenuAction, CheckMenuItem)>) {
    let menu = Menu::new();
    let mut checks = Vec::new();
    let mut actions = shared.menu_actions.lock().unwrap_or_else(|e| e.into_inner());
    for item in tray_menu(&TrayState::default()) {
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

pub fn run(shared: Arc<Shared>, events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let class = wide("vtypeMessages");
    let msg_hwnd = unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(msg_proc),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if msg_hwnd.is_null() {
        return Err(PlatformError::Failed("could not create the message window".into()));
    }

    let (menu, checks) = build_menu(&shared);
    let (idle_icon, recording_icon) = tray_icons();
    let mut builder =
        TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_menu_on_left_click(false).with_tooltip(tooltip());
    if let Some(icon) = idle_icon.clone() {
        builder = builder.with_icon(icon);
    }
    let tray = match builder.build() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(error = %e, "tray icon failed");
            None
        }
    };

    {
        let events = events.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = events.send(PlatformEvent::ToggleRequested);
            }
        }));
    }
    {
        let events = events.clone();
        let shared = shared.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let action = shared.menu_actions.lock().unwrap_or_else(|p| p.into_inner()).get(&e.id.0).copied();
            if let Some(action) = action {
                let _ = events.send(PlatformEvent::Menu(action));
            }
        }));
    }
    {
        let events = events.clone();
        let shared = shared.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed && e.id == shared.hotkey_id.load(Ordering::Acquire) {
                let _ = events.send(PlatformEvent::ToggleRequested);
            }
        }));
    }

    let overlay = Overlay::new(events, msg_hwnd);
    UI.with(|slot| {
        *slot.borrow_mut() = Some(Ui {
            shared: shared.clone(),
            msg_hwnd,
            tray,
            menu,
            checks,
            idle_icon,
            recording_icon,
            hotkeys: None,
            current_hotkey: None,
            overlay,
            icon_wanted: false,
            icon_position: None,
            hide_on_fullscreen: true,
            hidden_for_fullscreen: false,
            recording: false,
        });
    });
    shared.msg_hwnd.store(msg_hwnd as isize, Ordering::Release);
    drain(&shared);
    unsafe {
        SetTimer(msg_hwnd, TIMER_FULLSCREEN, 1000, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    shared.msg_hwnd.store(0, Ordering::Release);
    UI.with(|slot| slot.borrow_mut().take());
    Ok(())
}

pub fn quit() {
    unsafe { PostQuitMessage(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::parse;

    #[test]
    fn maps_the_default_shortcut() {
        let hk = to_global_hotkey(&parse("Ctrl+Alt+Space").unwrap()).unwrap();
        assert_eq!(hk, HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space));
        let f = to_global_hotkey(&parse("Win+Shift+F24").unwrap()).unwrap();
        assert_eq!(f, HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::F24));
        assert!(to_global_hotkey(&parse("Ctrl+9").unwrap()).is_some());
    }
}
