//! Everything that differs between Windows, macOS and Linux sits behind `Platform`. The daemon
//! calls the OS only through this trait, so its logic can be tested with a fake.
//!
//! Threading: `run_event_loop` owns the main thread (the tray and the overlay windows need it).
//! Every other method may be called from the daemon's worker thread; an implementation that must
//! touch the UI forwards the call to its event loop.

use std::sync::mpsc::Sender;

use std::time::{Duration, Instant};

use thiserror::Error;

pub use crate::beside_field::{Anchor, FieldProbe};
use crate::config::{BesideFieldConfig, InjectMethod};
pub use crate::field_check::FieldCheck;
pub use crate::kept_bubble::KeptButton;
pub use crate::overlay_logic::{MicButton, MicPart};
use crate::protocol::InputMode;
pub use crate::ripple::VoiceCue;

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
    /// The floating mic was dragged (or resized around its centre) to a new top-left position.
    IconMoved {
        x: i32,
        y: i32,
    },
    /// Ctrl+wheel over the floating mic: this many notches, up (positive) makes it bigger.
    IconZoom {
        steps: i32,
    },
    /// One of the small buttons around the floating mic was clicked.
    MicButton(MicButton),
    /// A button of the kept words' bubble (`Platform::show_kept`) was clicked.
    KeptButton(KeptButton),
    /// The pointer moved onto this part of the floating mic, or (`None`) off the mic: the bubble
    /// says what the part does once the pointer rests there. Sent only when the part changes.
    MicHover(Option<MicPart>),
    /// The focused element changed, or (hover) the pointer settled on or left a field; for the
    /// mic beside the field. `at` is when the OS told us, to measure how fast the mic appears.
    /// (Linux has no beside mic, so nothing sends it there.)
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    FieldChanged {
        probe: FieldProbe,
        at: Instant,
    },
    /// What `Platform::check_fields` saw of the app (only Windows checks).
    #[cfg_attr(not(windows), allow(dead_code))]
    FieldsChecked(FieldCheck),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Start or stop recording (the menu item exists where the tray cannot be clicked).
    ToggleRecording,
    ToggleIconVisible,
    ToggleHideOnFullscreen,
    SetMode(InputMode),
    /// Put the template at this index (of the config's list) into the foreground app.
    InsertTemplate(usize),
    /// Open the settings page with the template at this index being edited.
    EditTemplate(usize),
    /// Delete the template at this index at once (the list offers to put it back).
    DeleteTemplate(usize),
    /// Put back the template deleted last.
    UndoDeleteTemplate,
    /// Copy what is selected in the foreground app and keep it as a template.
    AddSelectionAsTemplate,
    OpenSettings,
    ReportBug,
    CopyDiagnostics,
    /// "The mic does not come to this app's text fields": watch the app the user was in, and say
    /// why (plan C1). Only in Windows' menu (`crate::menu::MENU_CHECKS_FIELDS`).
    CheckFields,
    /// Open the settings page at "About vtype".
    About,
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

/// How long a message (`Platform::tell`) stays in the bubble.
pub const MESSAGE_HOLD: Duration = Duration::from_secs(6);

/// Bubble height in lines. Live text keeps its latest words in two; a message is shown whole.
pub const LIVE_LINES: i32 = 2;
pub const MESSAGE_LINES: i32 = 6;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
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
    /// `Some(true)` for an editable text field (what the clear button may empty); `None` when the
    /// OS could not say.
    pub is_text_field: Option<bool>,
}

/// Keys the floating mic's buttons press in the foreground app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKeys {
    /// Select all, then Backspace: empties the focused field.
    ClearField,
    /// The send button.
    Send(crate::config::SendKey),
}

pub trait Platform: Send + Sync {
    /// Runs the OS event loop on the calling (main) thread until `quit` is called.
    fn run_event_loop(&self, events: Sender<PlatformEvent>) -> Result<(), PlatformError>;
    fn quit(&self);
    fn set_tray(&self, state: &TrayState);
    fn register_hotkey(&self, spec: &str) -> Result<(), PlatformError>;
    fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError>;
    /// Presses `keys` in the foreground app.
    fn press_keys(&self, _keys: EditKeys) -> Result<(), PlatformError> {
        Err(PlatformError::Failed("pressing keys is not implemented here".into()))
    }
    /// What is selected in the foreground app, through a copy (the clipboard is put back).
    /// `None` when nothing was selected.
    fn copy_selection(&self) -> Result<Option<String>, PlatformError> {
        Err(PlatformError::Failed("copying is not implemented here".into()))
    }
    /// Opens the templates list (`crate::menu::template_list`) at the floating mic. A click on a
    /// row sends its `MenuAction`; deleting and putting back leave the list open
    /// (`crate::menu::keeps_list_open`), everything else closes it and gives the focus back to the
    /// foreground app.
    fn show_templates(&self, _list: &crate::menu::TemplateList) {}
    /// The templates changed while the list was open (a deletion, or putting one back): shows
    /// `list` in its place. Nothing when the list is closed by now.
    fn refresh_templates(&self, _list: &crate::menu::TemplateList) {}
    fn focused_field(&self) -> FieldInfo;
    fn show_icon(&self, state: IconState, position: Option<(i32, i32)>);
    fn hide_icon(&self);
    /// The floating mic's size in percent (`overlay_logic::SCALE_MIN..=SCALE_MAX`). Set before
    /// the first `show_icon`; later it resizes the mic on screen, which may report `IconMoved`.
    fn set_icon_scale(&self, _percent: u16) {}
    /// The recognizer heard something: the ripple around the floating mic follows it.
    fn voice_cue(&self, _cue: VoiceCue) {}
    /// Recognition in progress above the floating mic; it goes away shortly after the last text.
    fn show_bubble(&self, text: &str);
    fn hide_bubble(&self);
    /// Tells the user something, the same way on every system: in the bubble above the floating
    /// mic for `hold` while the mic is on screen. Only when it is not (Wayland has no mic; the
    /// user can switch it off; it hides over full-screen apps) and `or_notify` is set does it fall
    /// back to a system notification, whose showing depends on the system's settings.
    fn tell(&self, text: &str, hold: Duration, or_notify: bool);
    /// Whether `show_kept` can put words on screen: this system can show the floating mic (not
    /// Wayland). Where it cannot, words are typed as before, whatever has the focus.
    fn keeps_words(&self) -> bool {
        false
    }
    /// Words that did not go in because no text field had the focus (plan C11), in the bubble
    /// above the floating mic with "Copy", "Insert" and ✕ (`PlatformEvent::KeptButton`), until
    /// `None` takes them away. Live text and messages cover them while they show, and the words
    /// come back after. Called again, it shows them where the mic is now.
    fn show_kept(&self, _text: Option<&str>) {}
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
    /// Whether a field taking the focus brings the floating mic itself there (Windows), rather
    /// than showing the beside mic.
    fn icon_follows_fields(&self) -> bool {
        false
    }
    /// Brings the floating mic to the field the user is at, until `icon_home`. Nothing while the
    /// mic is off screen, held, or under the pointer. `reported_at` as for `show_beside`.
    fn icon_to_field(&self, _anchor: Anchor, _reported_at: Instant) {}
    /// The focus left the fields: the floating mic goes back to the bottom-right corner of the
    /// screen the pointer is on, not saved as its position. Nothing when it is off screen, held,
    /// or under the pointer.
    fn icon_home(&self) {}
    /// Watches `app_id` for `field_check::CHECK_FOR`: what it reports taking the focus, and what
    /// its focused element is. The result comes back as `PlatformEvent::FieldsChecked`. Systems
    /// that do not bring the mic to fields do nothing.
    fn check_fields(&self, _app_id: &str) {}
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
