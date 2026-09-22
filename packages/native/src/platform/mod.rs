//! Everything that differs between Windows, macOS and Linux sits behind `Platform`. The daemon
//! calls the OS only through this trait, so its logic can be tested with a fake.
//!
//! Threading: `run_event_loop` owns the main thread (the tray and the overlay windows need it).
//! Every other method may be called from the daemon's worker thread; an implementation that must
//! touch the UI forwards the call to its event loop.

use std::sync::mpsc::Sender;

use std::time::Instant;

use thiserror::Error;

pub use crate::beside_field::FieldProbe;
use crate::config::{BesideFieldConfig, InjectMethod};
use crate::protocol::InputMode;

pub mod desktop;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
pub mod windows;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// (Every macOS feature is there, so the macOS code never says this.)
    #[error("not supported on this system")]
    #[cfg_attr(target_os = "macos", allow(dead_code))]
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
    /// The focused element changed, or (hover) the pointer settled on or left a field; for the
    /// mic beside the field. `at` is when the OS told us, to measure how fast the mic appears.
    /// (Linux has no beside mic, so nothing sends it there.)
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    FieldChanged {
        probe: FieldProbe,
        at: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Start or stop recording (the menu item exists where the tray cannot be clicked).
    ToggleRecording,
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
    /// Could not type or paste; the text is on the clipboard for the user to paste. Only the
    /// Wayland fallback ends here (elsewhere a failure is an error, and the daemon copies).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
    /// Named facts for the diagnostic report (e.g. how far the Wayland typing fallback got).
    fn platform_notes(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    // The mic beside the text field (child plan C8). Systems without it keep these no-ops.

    /// Starts, changes or stops watching the focused element (or the pointer, for hover).
    fn watch_fields(&self, _config: &BesideFieldConfig) {}
    /// Shows the beside mic with its top-left at `pos` (kept on a screen by the platform).
    /// `reported_at` is the `FieldChanged` time, to measure how fast the mic appeared.
    fn show_beside(&self, _pos: (i32, i32), _look: IconState, _reported_at: Instant) {}
    fn hide_beside(&self) {}
    fn set_beside_look(&self, _look: IconState) {}
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
#[cfg(not(windows))]
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
