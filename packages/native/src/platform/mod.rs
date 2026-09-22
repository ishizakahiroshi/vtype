//! Everything that differs between Windows, macOS and Linux sits behind `Platform`. The daemon
//! calls the OS only through this trait, so its logic can be tested with a fake.
//!
//! Threading: `run_event_loop` owns the main thread (the tray and the overlay windows need it).
//! Every other method may be called from the daemon's worker thread; an implementation that must
//! touch the UI forwards the call to its event loop.

use std::sync::mpsc::Sender;

use thiserror::Error;

use crate::config::InjectMethod;
use crate::protocol::InputMode;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
pub mod windows;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlatformError {
    #[error("not supported on this system")]
    Unsupported,
    #[error("{0}")]
    Failed(String),
}

impl PlatformError {
    pub fn failed(e: impl std::fmt::Display) -> PlatformError {
        PlatformError::Failed(e.to_string())
    }
}

/// Something the user did in the OS UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    /// Left click on the tray icon, the floating mic, or the global shortcut.
    ToggleRequested,
    Menu(MenuAction),
    /// The floating mic was dragged to a new top-left position.
    IconMoved {
        x: i32,
        y: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    ToggleIconVisible,
    ToggleHideOnFullscreen,
    SetMode(InputMode),
    OpenSettings,
    ReportBug,
    CopyDiagnostics,
    Quit,
}

/// What the tray menu shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayState {
    pub connected: bool,
    pub recording: bool,
    pub mode: InputMode,
    pub icon_visible: bool,
    pub hide_on_fullscreen: bool,
}

/// The floating mic's look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconState {
    #[default]
    Idle,
    Recording,
    /// Text was just inserted (a short check mark).
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectOutcome {
    Typed,
    Pasted,
    /// Could not type or paste; the text is on the clipboard for the user to paste.
    CopiedOnly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The focused text field of the foreground app, as far as the OS tells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldInfo {
    /// `Some(true)` for a password field; `None` when the OS could not say.
    pub is_password: Option<bool>,
    pub caret_rect: Option<Rect>,
    /// Executable name or bundle id of the foreground app.
    pub app_id: Option<String>,
}

pub trait Platform: Send + Sync {
    /// Runs the OS event loop on the calling (main) thread until `quit` is called.
    fn run_event_loop(&self, events: Sender<PlatformEvent>) -> Result<(), PlatformError>;
    fn quit(&self);
    fn set_tray(&self, state: &TrayState);
    fn register_hotkey(&self, spec: &str) -> Result<(), PlatformError>;
    fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError>;
    fn focused_field(&self) -> FieldInfo;
    fn show_icon(&self, state: IconState, position: Option<(i32, i32)>);
    fn hide_icon(&self);
    fn show_bubble(&self, text: &str);
    fn hide_bubble(&self);
    fn notify(&self, title: &str, body: &str);
    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError>;
    /// Starts Chrome with `args` (e.g. `--no-startup-window`, or a URL to open).
    fn launch_chrome(&self, args: &[String]) -> Result<(), PlatformError>;
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
    fn copy_to_clipboard(&self, text: &str) -> Result<(), PlatformError>;
    /// e.g. `Windows 10.0.26200`.
    fn os_description(&self) -> String;
    /// BCP 47-ish UI language of the OS, e.g. `ja-JP`.
    fn ui_language(&self) -> String;
}

/// The implementation for the OS this binary was built for.
pub fn current() -> Box<dyn Platform> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }
}

/// Language from the POSIX locale variables, for the systems that have no better source.
pub fn language_from_env() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.split('.').next().unwrap_or("").replace('_', "-");
            if !v.is_empty() && v != "C" && v != "POSIX" {
                return v;
            }
        }
    }
    "en".to_string()
}

/// A loop that just waits for `quit`, for platforms whose UI is not written yet.
#[derive(Default)]
pub struct IdleLoop {
    quit: std::sync::Mutex<bool>,
    cv: std::sync::Condvar,
}

impl IdleLoop {
    pub fn run(&self) {
        let mut done = self.quit.lock().unwrap_or_else(|e| e.into_inner());
        while !*done {
            done = self.cv.wait(done).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub fn quit(&self) {
        *self.quit.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }
}
