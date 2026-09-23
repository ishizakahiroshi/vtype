//! The resident process. It owns the tray and the floating mic, listens on the IPC endpoint for
//! the command line, starts its own Chrome on the speech page (speech_host.rs, chrome_launch.rs),
//! and turns what the page recognizes into text in the foreground app.
//!
//! All decisions live in `Core`, which runs on one worker thread and reaches the OS only through
//! `Platform`; the main thread runs the platform's event loop.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use interprocess::local_socket::{prelude::*, Listener, Stream};
use serde_json::Value;

use crate::beside_field;
use crate::chrome_launch::{self, WindowPlacement};
use crate::config::{self, BesideFieldTrigger, NativeConfig};
use crate::diag::{self, ErrorLog};
use crate::field_check::{self, FieldCheck};
use crate::i18n::{t, t_with};
use crate::ipc;
use crate::menu;
use crate::overlay_logic;
use crate::platform::{
    Anchor, EditKeys, FieldInfo, FieldProbe, IconState, InjectOutcome, KeptButton, MenuAction, MicButton, MicPart,
    Platform, PlatformError, PlatformEvent, Rect, TrayState, VoiceCue, MESSAGE_HOLD,
};
use crate::protocol::{FromExtension, InputMode, Reply, Request, SessionEvent, ToExtension};
use crate::report::{self, ReportInfo, Surface};

/// After a menu closes, how long the app it gave the focus back to gets before keys arrive.
#[cfg(not(test))]
const MENU_SETTLE: Duration = Duration::from_millis(150);
#[cfg(test)]
const MENU_SETTLE: Duration = Duration::ZERO;

/// How long a start request waits for Chrome to come up and connect (parent plan D13).
pub const CHROME_WAIT: Duration = Duration::from_secs(10);

pub type ConnId = u64;

pub enum Event {
    Request { conn: ConnId, req: Request, out: Sender<Reply> },
    Closed { conn: ConnId },
    Platform(PlatformEvent),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

struct HostLink {
    conn: ConnId,
    out: Sender<Reply>,
    /// Set once the extension said hello through this host.
    extension_version: Option<String>,
    browser: Option<String>,
}

struct Pending {
    start_mode: Option<InputMode>,
    /// `None` while the user is on the first-run page: that takes as long as it takes.
    deadline: Option<Instant>,
}

/// How long the setup window has to be gone before Chrome is started again off screen. Chrome
/// rewrites the profile's Preferences as it exits; the microphone grant is written after that.
pub const RELAUNCH_DELAY: Duration = Duration::from_secs(3);

/// How long the first-run request is held in the bubble.
const SETUP_HOLD: Duration = Duration::from_secs(30);

/// How long the pointer rests on a part of the floating mic before the bubble says what it does
/// (sooner, and the bubble would flicker as the pointer passes over), and how long that stays
/// when the pointer does not leave.
const HINT_DELAY: Duration = Duration::from_millis(700);
const HINT_HOLD: Duration = Duration::from_secs(10);

/// How long the focus is off the fields before the floating mic that came to one goes back to the
/// corner. Measured on the development machine: of 48 times the focus came back to a field, 13
/// were within 0.6 s (passing through, e.g. Tab over a button) and the rest 1.2 s or more.
const HOME_DELAY: Duration = Duration::from_millis(700);

/// How long the result of a look at an app's text fields stays in the bubble: longer than most
/// messages, as it is read once and is two sentences.
const FIELD_CHECK_HOLD: Duration = Duration::from_secs(12);

/// How long "Copied" stays after the kept words were copied.
const COPIED_HOLD: Duration = Duration::from_secs(2);

/// What became of words handed to `Core::put_in`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Put {
    /// Typed or pasted into the foreground app.
    In,
    /// No text field had the focus: kept above the mic (plan C11).
    Kept,
    /// Put nowhere: nothing to put, or a password field in front.
    Refused,
    /// On the clipboard for the user to paste (they could not be typed).
    Copied,
}

/// Whether `field` is a password field or a security credential dialog that must not receive text.
fn is_protected_field(field: &FieldInfo) -> bool {
    if field.is_password == Some(true) {
        return true;
    }
    if let Some(app) = field.app_id.as_deref() {
        let app_lower = app.to_ascii_lowercase();
        if app_lower == "credentialuibroker.exe" || app_lower == "consent.exe" || app_lower == "logonui.exe" {
            return true;
        }
    }
    false
}

pub struct Core {
    platform: Arc<dyn Platform>,
    config: NativeConfig,
    config_path: Option<PathBuf>,
    host: Option<HostLink>,
    mode: InputMode,
    recording: bool,
    pending: Option<Pending>,
    errors: ErrorLog,
    now: Box<dyn Fn() -> Instant + Send>,
    /// Last character typed in this recording (to space English finals apart).
    last_char: Option<char>,
    /// The send button was pressed while recording: the recording stops, and the send key is
    /// pressed once the last text went in (when the session ends).
    send_after_stop: bool,
    /// The template deleted last from the templates menu, and where it was, so that the menu can
    /// put it back (there is no confirmation before deleting).
    deleted_template: Option<(usize, String)>,
    /// The mic beside the field is up (child plan C8).
    beside_shown: bool,
    /// The field the floating mic came to last (app and bounds), so that the focus coming back
    /// to it (a template menu closing, a window switched back to) does not move the mic again.
    followed_field: Option<(Option<String>, Option<Rect>)>,
    /// The focus left the fields after the floating mic came to one: it goes back to the corner at
    /// this time, or once a recording ends or the pointer leaves it, if that is later.
    home_at: Option<Instant>,
    /// The pointer is on the floating mic (it may be held).
    mic_hovered: bool,
    /// The app other than vtype (and the taskbar) that had the focus last: the one "the mic does
    /// not come to this app's text fields" looks at. Not the foreground window, which is the
    /// taskbar when the menu is opened from the tray.
    last_app: Option<String>,
    /// The last look at an app's text fields, and when (UTC), for the diagnostic info.
    last_field_check: Option<(String, FieldCheck)>,
    /// The daemon's own speech page (speech_host.rs), when it could listen.
    speech_page: Option<String>,
    /// The daemon's own Chrome profile (chrome_launch.rs).
    profile_dir: PathBuf,
    /// Where the last Chrome for the speech page was put.
    launched: Option<WindowPlacement>,
    /// The page said consent or the microphone is missing: the next launch shows the window.
    needs_setup: bool,
    /// The page has consent and the microphone.
    page_ready: bool,
    /// The microphone grant did not hold off screen: use a window that stays on screen for the
    /// rest of this run instead of asking again and again.
    keep_visible: bool,
    /// Write the microphone grant into the profile before launching (off in tests).
    grant_in_profile: bool,
    /// The mode of the last recording asked of a page, for a page started again in its place.
    last_start_mode: Option<InputMode>,
    /// Start Chrome again at this time, placed like this (after the setup window closed).
    relaunch: Option<(Instant, WindowPlacement)>,
    /// The pointer rests on this part of the floating mic: its hint shows at this time.
    hint_due: Option<(MicPart, Instant)>,
    /// A hint is in the bubble until this time (leaving the mic takes it away sooner).
    hint_until: Option<Instant>,
    /// No new words since: the recording stops at this time ("stop after you finish speaking").
    /// Set by the first words of a recording, not before, so a user who has not started speaking
    /// is left to the speech page's own end.
    silence_stop_at: Option<Instant>,
    /// A stop was sent for this recording: words still arriving (the page waits for the last
    /// one) do not set `silence_stop_at` again, so the stop is not sent twice.
    stop_sent: bool,
    /// Words said while no text field had the focus, kept above the mic until the user copies,
    /// inserts or throws them away (plan C11). Empty when there are none.
    kept: String,
}

impl Core {
    pub fn new(platform: Arc<dyn Platform>, config: NativeConfig, config_path: Option<PathBuf>) -> Core {
        let mode = config.input_mode;
        Core {
            platform,
            config,
            config_path,
            host: None,
            mode,
            recording: false,
            pending: None,
            errors: ErrorLog::default(),
            now: Box::new(Instant::now),
            last_char: None,
            send_after_stop: false,
            deleted_template: None,
            beside_shown: false,
            followed_field: None,
            home_at: None,
            mic_hovered: false,
            last_app: None,
            last_field_check: None,
            speech_page: None,
            profile_dir: chrome_launch::profile_dir(),
            launched: None,
            needs_setup: false,
            page_ready: false,
            keep_visible: false,
            grant_in_profile: false,
            last_start_mode: None,
            relaunch: None,
            hint_due: None,
            hint_until: None,
            silence_stop_at: None,
            stop_sent: false,
            kept: String::new(),
        }
    }

    /// Lets the daemon write the microphone grant into its Chrome profile (chrome_launch.rs).
    pub fn enable_profile_grant(&mut self) {
        self.grant_in_profile = true;
    }

    /// Where the speech page is served (`http://127.0.0.1:<port>/t/<token>/speech`).
    pub fn set_speech_page(&mut self, url: String) {
        self.speech_page = Some(url);
    }

    /// Replaces the clock, for tests.
    #[cfg(test)]
    pub fn set_clock(&mut self, now: Box<dyn Fn() -> Instant + Send>) {
        self.now = now;
    }

    pub fn connected(&self) -> bool {
        self.host.as_ref().is_some_and(|h| h.extension_version.is_some())
    }

    /// Connected to a page that can record: not the first-run window (it only asks for consent),
    /// and a kept window only once it has the microphone.
    fn ready(&self) -> bool {
        self.connected()
            && match self.launched {
                Some(WindowPlacement::Visible) => false,
                Some(WindowPlacement::Kept) => self.page_ready,
                _ => true,
            }
    }

    #[cfg(test)]
    pub fn config(&self) -> &NativeConfig {
        &self.config
    }

    /// Everything that happens once, when the daemon comes up.
    pub fn start(&mut self) {
        // The mic first: a shortcut that cannot be registered is said in its bubble.
        self.platform.set_icon_scale(self.config.icon.scale);
        if self.config.icon.visible {
            self.platform.show_icon(IconState::Idle, self.icon_position());
        }
        self.register_hotkey();
        self.platform.watch_fields(&self.config.beside_field);
        self.update_tray();
        // A copy moved or built elsewhere: starting at sign-in follows it (plan C12).
        self.platform.autostart_here();
    }

    /// Whether vtype starts at sign-in, for the settings page (plan C12).
    fn autostart(&self) -> Reply {
        match self.platform.autostart() {
            Ok(autostart) => Reply::Autostart { autostart },
            Err(e) => Reply::error("autostart_unavailable", e.to_string()),
        }
    }

    /// Switches starting at sign-in; the reply is the state after.
    fn set_autostart(&mut self, enabled: bool) -> Reply {
        if let Err(e) = self.platform.set_autostart(enabled) {
            tracing::warn!(error = %e, enabled, "could not switch starting at sign-in");
            self.errors.push("autostart_failed");
            return Reply::error("autostart_failed", e.to_string());
        }
        tracing::info!(enabled, "starting at sign-in switched");
        self.autostart()
    }

    fn icon_position(&self) -> Option<(i32, i32)> {
        match (self.config.icon.x, self.config.icon.y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
    }

    fn register_hotkey(&mut self) {
        let spec = self.config.effective_hotkey().to_string();
        match self.platform.register_hotkey(&spec) {
            Ok(()) => tracing::info!(hotkey = %spec, "hotkey registered"),
            Err(PlatformError::Unsupported) => tracing::info!("no global shortcut on this system"),
            Err(PlatformError::Failed(e)) => {
                tracing::warn!(hotkey = %spec, error = %e, "hotkey registration failed");
                self.errors.push("hotkey_failed");
                self.platform.tell(&t_with("native_notifyHotkeyFailed", &[("hotkey", &spec)]), MESSAGE_HOLD, true);
            }
        }
    }

    pub fn tray_state(&self) -> TrayState {
        TrayState {
            connected: self.connected(),
            recording: self.recording,
            mode: self.mode,
            icon_visible: self.config.icon.visible,
            hide_on_fullscreen: self.config.icon.hide_on_fullscreen,
        }
    }

    fn update_tray(&self) {
        self.platform.set_tray(&self.tray_state());
    }

    fn send_start(&mut self, mode: Option<InputMode>) {
        self.last_start_mode = mode;
        self.send(&ToExtension::Start { mode });
    }

    fn send(&self, msg: &ToExtension) -> bool {
        let Some(host) = &self.host else { return false };
        match serde_json::to_value(msg) {
            Ok(message) => host.out.send(Reply::ToExtension { message }).is_ok(),
            Err(_) => false,
        }
    }

    /// When the loop should wake up even if nothing arrives.
    pub fn next_deadline(&self) -> Option<Instant> {
        let pending = self.pending.as_ref().and_then(|p| p.deadline);
        let relaunch = self.relaunch.map(|(at, _)| at);
        let hint = self.hint_due.map(|(_, at)| at);
        // Held by a recording or the pointer: those ending say when, not the clock (a time
        // already past would wake the loop again and again).
        let home = self.home_at.filter(|_| self.mic_free());
        pending.into_iter().chain(relaunch).chain(hint).chain(self.silence_stop_at).chain(home).min()
    }

    pub fn tick(&mut self) {
        let now = (self.now)();
        self.go_home_if_due();
        if self.silence_stop_at.is_some_and(|at| now >= at) {
            tracing::info!(seconds = self.config.silence_stop_sec, "no new words; stopping the recording");
            // As the mic does: the send key is pressed only by the send button.
            let _ = self.stop_recording();
        }
        if let Some((part, at)) = self.hint_due {
            if now >= at {
                self.hint_due = None;
                if self.hint_fits() {
                    self.platform.tell(&self.hint_text(part), HINT_HOLD, false);
                    self.hint_until = Some(now + HINT_HOLD);
                }
            }
        }
        if let Some((at, placement)) = self.relaunch {
            if now >= at {
                self.relaunch = None;
                if self.pending.is_some() {
                    self.launch_speech_page(placement);
                }
            }
        }
        if let Some(Pending { deadline: Some(deadline), .. }) = &self.pending {
            if now >= *deadline {
                self.pending = None;
                tracing::warn!("Chrome did not connect in time");
                self.errors.push("chrome_not_connected");
                self.platform.tell(&t("native_notifySpeechPageNotConnected"), MESSAGE_HOLD, true);
            }
        }
    }

    /// Starts the daemon's own Chrome on the speech page. On screen it waits for the user (no
    /// deadline); off screen it waits CHROME_WAIT for the page to connect.
    fn launch_speech_page(&mut self, placement: WindowPlacement) {
        let Some(base) = self.speech_page.clone() else {
            self.fail_launch("speech_page_unavailable", &t("native_notifySpeechPageNotConnected"));
            return;
        };
        if self.config.consented && self.grant_in_profile {
            if let Some(origin) = chrome_launch::origin_of(&base) {
                if let Err(e) = chrome_launch::grant_microphone_in_profile(&self.profile_dir, origin) {
                    tracing::warn!(error = %e, "could not allow the microphone in the Chrome profile");
                }
            }
        }
        let url = chrome_launch::page_url(&base, self.config.consented, placement);
        let args = chrome_launch::speech_args(&self.profile_dir, &url, placement);
        tracing::info!(?placement, "not connected; starting Chrome");
        if let Err(e) = self.platform.launch_chrome(&args) {
            tracing::warn!(error = %e, "could not start Chrome");
            if e.to_string().contains("Chrome was not found") {
                self.fail_launch("chrome_missing", &t("native_notifyChromeMissing"));
            } else {
                self.fail_launch("chrome_launch_failed", &t("native_notifySpeechPageNotConnected"));
            }
            return;
        }
        self.launched = Some(placement);
        self.page_ready = false;
        let deadline = match placement {
            WindowPlacement::Visible | WindowPlacement::Kept => {
                self.platform.tell(&t("native_bubbleConsent"), SETUP_HOLD, true);
                None
            }
            WindowPlacement::Hidden => {
                // Otherwise nothing shows for up to CHROME_WAIT. Held a little past it, so the bubble
                // is still up when the outcome replaces it; no notification for a passing state.
                self.platform.tell(&t("native_bubbleConnecting"), CHROME_WAIT + Duration::from_secs(2), false);
                Some((self.now)() + CHROME_WAIT)
            }
        };
        if let Some(p) = self.pending.as_mut() {
            p.deadline = deadline;
        }
    }

    fn fail_launch(&mut self, code: &str, message: &str) {
        self.pending = None;
        self.errors.push(code);
        self.platform.tell(message, MESSAGE_HOLD, true);
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        match event {
            Event::Request { conn, req, out } => self.handle_request(conn, req, out),
            Event::Closed { conn } => {
                if self.host.as_ref().is_some_and(|h| h.conn == conn) {
                    tracing::info!("extension disconnected");
                    self.host = None;
                    self.end_recording_ui();
                    self.page_closed();
                    self.update_tray();
                }
                Flow::Continue
            }
            Event::Platform(ev) => self.handle_platform(ev),
        }
    }

    fn handle_request(&mut self, conn: ConnId, req: Request, out: Sender<Reply>) -> Flow {
        let reply = match req {
            Request::Status => Some(Reply::Status {
                connected: self.connected(),
                recording: self.recording,
                mode: self.mode,
                version: env!("CARGO_PKG_VERSION").to_string(),
            }),
            Request::Toggle { mode } => Some(self.toggle(mode)),
            Request::Start { mode } => Some(self.start_recording(mode)),
            Request::Stop => Some(self.stop_recording()),
            Request::SetMode { mode } => Some(self.set_mode(mode)),
            Request::OpenSettings => Some(self.open_settings()),
            Request::Diagnostics => Some(Reply::Diagnostics { report: self.diagnostics() }),
            Request::GetConfig => Some(Reply::Config { config: self.config.clone() }),
            Request::SetConfig { config } => {
                self.apply_config(config);
                // The speech page takes the replacement table from here.
                self.send(&ToExtension::NativeConfig { config: self.config.clone() });
                Some(Reply::Config { config: self.config.clone() })
            }
            Request::OpenLink { link } => Some(match self.platform.open_url(&link.url(env!("CARGO_PKG_VERSION"))) {
                Ok(()) => Reply::Ok,
                Err(e) => Reply::error("open_failed", e.to_string()),
            }),
            Request::GetAutostart => Some(self.autostart()),
            Request::SetAutostart { enabled } => Some(self.set_autostart(enabled)),
            Request::Quit => {
                let _ = out.send(Reply::Ok);
                return Flow::Quit;
            }
            Request::HostHello { origin } => {
                tracing::info!(%origin, "host attached");
                self.host = Some(HostLink { conn, out: out.clone(), extension_version: None, browser: None });
                None
            }
            Request::FromExtension { message } => {
                if self.host.as_ref().is_some_and(|h| h.conn == conn) {
                    self.on_extension_message(message);
                }
                None
            }
        };
        if let Some(reply) = reply {
            let _ = out.send(reply);
        }
        Flow::Continue
    }

    /// The small buttons around the floating mic.
    fn mic_button(&mut self, button: MicButton) {
        match button {
            MicButton::Templates => self.platform.show_templates(&self.template_list()),
            MicButton::Mode => {
                let next = match self.mode {
                    InputMode::Normal => InputMode::En,
                    InputMode::En => InputMode::Kana,
                    InputMode::Kana => InputMode::Normal,
                };
                let _ = self.set_mode(next);
            }
            MicButton::Send => {
                if self.recording {
                    // Send what is being said, not half of it.
                    self.send_after_stop = true;
                    let _ = self.stop_recording();
                } else {
                    self.press(EditKeys::Send(self.config.send_key));
                }
            }
            MicButton::Clear => {
                let field = self.platform.focused_field();
                // Only an editable text field: Ctrl+A in a file list or a document selects far more.
                if field.is_text_field == Some(true) && !is_protected_field(&field) {
                    self.press(EditKeys::ClearField);
                } else {
                    self.platform.tell(&t("native_bubbleClearNotField"), MESSAGE_HOLD, false);
                }
            }
        }
    }

    /// A hint may take the bubble: not while it shows what is being said, the wait for Chrome, or
    /// kept words (the pointer passes over the mic on its way to their buttons).
    fn hint_fits(&self) -> bool {
        !self.recording && self.pending.is_none() && self.kept.is_empty()
    }

    /// What the bubble says for the part of the floating mic the pointer rests on.
    fn hint_text(&self, part: MicPart) -> String {
        match part {
            MicPart::Mic => t("native_hintMic"),
            MicPart::Button(MicButton::Templates) => t("native_hintTemplates"),
            MicPart::Button(MicButton::Mode) => {
                let mode = match self.mode {
                    InputMode::Normal => t("native_trayModeNormal"),
                    InputMode::En => t("native_trayModeEn"),
                    InputMode::Kana => t("native_trayModeKana"),
                };
                t_with("native_hintMode", &[("mode", &mode)])
            }
            MicPart::Button(MicButton::Send) => {
                // The keys `EditKeys::Send` presses (Ctrl+Enter is ⌘+Return on macOS).
                let key = match self.config.send_key {
                    config::SendKey::Enter => "Enter",
                    config::SendKey::CtrlEnter if cfg!(target_os = "macos") => "⌘+Return",
                    config::SendKey::CtrlEnter => "Ctrl+Enter",
                };
                t_with("native_hintSend", &[("key", key)])
            }
            MicPart::Button(MicButton::Clear) => t("native_hintClear"),
        }
    }

    /// Forgets the hint waiting to show, and takes the one showing out of the bubble.
    fn drop_hint(&mut self) {
        self.hint_due = None;
        let showing = self.hint_until.take().is_some_and(|until| (self.now)() < until);
        if showing && self.hint_fits() {
            self.platform.hide_bubble();
        }
    }

    /// A template chosen from the menu: in it goes, and is sent when the user asked for that.
    fn insert_template(&mut self, index: usize) {
        let Some(text) = self.config.templates.get(index).cloned() else { return };
        // The menu gave the focus back to the app just now; let it settle first.
        std::thread::sleep(MENU_SETTLE);
        // A template is a text of its own: no space in front of it for the words said before.
        self.last_char = None;
        if self.insert(&text) && self.config.template_send_immediate {
            self.press(EditKeys::Send(self.config.send_key));
        }
    }

    /// The selection of the foreground app becomes a template.
    fn add_selection_as_template(&mut self) {
        std::thread::sleep(MENU_SETTLE);
        let copied = match self.platform.copy_selection() {
            Ok(copied) => copied,
            Err(e) => {
                tracing::warn!(error = %e, "could not copy the selection");
                self.platform.tell(&t("native_bubbleKeysUnsupported"), MESSAGE_HOLD, false);
                return;
            }
        };
        // Tidied the way every template is (trimmed, cut to length).
        let text = config::normalize_templates(copied).into_iter().next();
        let message = match text {
            None => t("native_bubbleNoSelection"),
            Some(text) if self.config.templates.contains(&text) => t("native_bubbleTemplateDuplicate"),
            Some(_) if self.config.templates.len() >= config::MAX_TEMPLATES => t("native_bubbleTemplateFull"),
            Some(text) => {
                self.config.templates.push(text);
                self.save_config();
                t("native_bubbleTemplateAdded")
            }
        };
        self.platform.tell(&message, MESSAGE_HOLD, false);
    }

    /// The templates list as it stands, with the one deleted last where it was.
    fn template_list(&self) -> menu::TemplateList {
        let deleted = self.deleted_template.as_ref().map(|(index, text)| (*index, text.as_str()));
        menu::template_list(&self.config.templates, deleted)
    }

    /// Deletes a template from the templates list at once (it does not ask); the list stays open
    /// and offers to put it back, until the next deletion.
    fn delete_template(&mut self, index: usize) {
        if index < self.config.templates.len() {
            let text = self.config.templates.remove(index);
            self.save_config();
            self.deleted_template = Some((index, text));
        }
        // Always: on Windows the click closed the menu, and this opens it again.
        self.platform.refresh_templates(&self.template_list());
    }

    /// Puts the template deleted last back where it was (or last, when the list got shorter).
    fn undo_delete_template(&mut self) {
        if let Some((index, text)) = self.deleted_template.take() {
            if self.config.templates.contains(&text) {
                self.platform.tell(&t("native_bubbleTemplateDuplicate"), MESSAGE_HOLD, false);
            } else if self.config.templates.len() >= config::MAX_TEMPLATES {
                self.platform.tell(&t("native_bubbleTemplateFull"), MESSAGE_HOLD, false);
            } else {
                let at = index.min(self.config.templates.len());
                self.config.templates.insert(at, text);
                self.save_config();
            }
        }
        self.platform.refresh_templates(&self.template_list());
    }

    fn press(&self, keys: EditKeys) {
        if let Err(e) = self.platform.press_keys(keys) {
            tracing::warn!(error = %e, ?keys, "could not press the keys");
            self.platform.tell(&t("native_bubbleKeysUnsupported"), MESSAGE_HOLD, false);
        }
    }

    fn toggle(&mut self, mode: Option<InputMode>) -> Reply {
        if self.recording {
            self.stop_recording()
        } else {
            self.start_recording(mode)
        }
    }

    fn start_recording(&mut self, mode: Option<InputMode>) -> Reply {
        if self.ready() {
            self.send_start(mode);
            return Reply::Ok;
        }
        if let Some(p) = self.pending.as_mut() {
            // Already starting (or the first-run window is open): only the mode changes.
            p.start_mode = mode;
            return Reply::Ok;
        }
        self.pending = Some(Pending { start_mode: mode, deadline: None });
        if self.connected() || self.relaunch.is_some() {
            return Reply::Ok; // the first-run window is up, or Chrome is about to start again
        }
        let placement = if !self.config.consented {
            WindowPlacement::Visible
        } else if self.keep_visible {
            WindowPlacement::Kept
        } else {
            WindowPlacement::Hidden
        };
        self.launch_speech_page(placement);
        Reply::Ok
    }

    /// The page's connection is gone. After the first-run window closed itself (consent given),
    /// Chrome is started again off screen. A hidden page that found consent or the microphone
    /// missing comes back on screen: for consent as the first-run window, for the microphone as a
    /// window that stays (the grant did not hold, so hiding it again would only ask again).
    fn page_closed(&mut self) {
        let next = match self.launched.take() {
            Some(WindowPlacement::Visible) if self.config.consented => Some(WindowPlacement::Hidden),
            Some(WindowPlacement::Hidden) if self.needs_setup => {
                if self.config.consented {
                    self.keep_visible = true;
                    Some(WindowPlacement::Kept)
                } else {
                    Some(WindowPlacement::Visible)
                }
            }
            Some(WindowPlacement::Visible | WindowPlacement::Kept) => {
                // The user closed it: nothing to wait for any more.
                if self.pending.take().is_some() {
                    self.platform.hide_bubble();
                }
                None
            }
            _ => None,
        };
        self.page_ready = false;
        if let Some(placement) = next {
            if self.pending.is_none() {
                // The page that went away may have been asked to record: its replacement is.
                self.pending = Some(Pending { start_mode: self.last_start_mode, deadline: None });
            }
            self.relaunch = Some(((self.now)() + RELAUNCH_DELAY, placement));
        }
    }

    fn stop_recording(&mut self) -> Reply {
        self.silence_stop_at = None;
        self.stop_sent = true;
        if self.pending.take().is_some() {
            self.relaunch = None;
            self.platform.hide_bubble(); // "connecting…"
        }
        if self.send(&ToExtension::Stop) {
            Reply::Ok
        } else {
            Reply::error("not_connected", "the vtype extension is not connected")
        }
    }

    fn set_mode(&mut self, mode: InputMode) -> Reply {
        self.mode = mode;
        if self.config.input_mode != mode {
            // Kept across restarts (standalone plan C5); a page hears it when it says hello.
            self.config.input_mode = mode;
            self.save_config();
        }
        self.send(&ToExtension::SetMode { mode });
        self.update_tray();
        Reply::Ok
    }

    /// The daemon's own settings page (standalone plan C5), in its own Chrome profile.
    fn open_settings(&mut self) -> Reply {
        self.open_settings_at(None)
    }

    /// The settings page at `part` when there is one: `template-<index>` opens that template for
    /// editing, `about` shows "About vtype".
    fn open_settings_at(&mut self, part: Option<&str>) -> Reply {
        let Some(page) = self.speech_page.as_deref() else {
            return Reply::error("speech_page_unavailable", "the settings page is not being served");
        };
        let mut url = chrome_launch::settings_url(page);
        if let Some(part) = part {
            url.push_str(&format!("#{part}"));
        }
        let args = chrome_launch::settings_args(&self.profile_dir, &url);
        match self.platform.launch_chrome(&args) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::error("chrome_launch_failed", e.to_string()),
        }
    }

    pub fn diagnostics(&self) -> Value {
        let os = self.platform.os_description();
        let host = self.host.as_ref();
        let notes = self.platform.platform_notes();
        let field_check = self.last_field_check.as_ref().map(|(at, check)| field_check::report(at, check));
        diag::report(&diag::Snapshot {
            os: &os,
            config: &self.config,
            connected: self.connected(),
            extension_version: host.and_then(|h| h.extension_version.as_deref()),
            browser: host.and_then(|h| h.browser.as_deref()),
            errors: &self.errors,
            notes: &notes,
            field_check: field_check.as_ref(),
        })
    }

    fn on_extension_message(&mut self, message: Value) {
        let msg = match serde_json::from_value::<FromExtension>(message) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable message from the extension");
                return;
            }
        };
        match msg {
            FromExtension::Hello { extension_version, browser } => {
                tracing::info!(version = %extension_version, "extension connected");
                if let Some(h) = self.host.as_mut() {
                    h.extension_version = Some(extension_version);
                    h.browser = browser;
                }
                self.send(&ToExtension::Hello {
                    native_version: env!("CARGO_PKG_VERSION").to_string(),
                    os: self.platform.os_description(),
                });
                // The replacement table travels in the config; the mode is said on its own.
                self.send(&ToExtension::NativeConfig { config: self.config.clone() });
                self.send(&ToExtension::SetMode { mode: self.mode });
                self.send(&ToExtension::GetState);
                // The first-run window only asks; the recording starts once it is hidden.
                if self.ready() {
                    if let Some(p) = self.pending.take() {
                        self.platform.hide_bubble(); // "connecting…"
                        self.send_start(p.start_mode);
                    }
                }
                self.update_tray();
            }
            FromExtension::Consent { autostart } => {
                tracing::info!("the user agreed on the speech page");
                if !self.config.consented {
                    self.config.consented = true;
                    self.save_config();
                }
                // The first-run screen's "start vtype when you sign in".
                if let Some(enabled) = autostart {
                    let _ = self.set_autostart(enabled);
                }
            }
            FromExtension::PageState { consented, mic_granted } => {
                tracing::info!(consented, mic_granted, "speech page state");
                self.page_ready = consented && mic_granted;
                self.needs_setup = !self.page_ready;
                if self.launched == Some(WindowPlacement::Hidden) && self.needs_setup {
                    // The page closes itself; page_closed() starts it again on screen.
                    self.platform.tell(&t("native_bubbleConsent"), SETUP_HOLD, true);
                }
                // A kept window just got the microphone: the waiting recording starts.
                if self.launched == Some(WindowPlacement::Kept) && self.page_ready {
                    if let Some(p) = self.pending.take() {
                        self.platform.hide_bubble();
                        self.send_start(p.start_mode);
                    }
                }
            }
            FromExtension::State { mode, recording } => {
                self.mode = mode;
                if self.recording != recording {
                    self.recording = recording;
                    if recording {
                        self.show_icon(IconState::Recording);
                        self.hide_kept_while_recording();
                    } else {
                        self.end_recording_ui();
                    }
                }
                self.update_tray();
            }
            FromExtension::Session { event } => self.session_event(event),
            FromExtension::SetNativeConfig { config } => {
                self.apply_config(config);
                self.send(&ToExtension::NativeConfig { config: self.config.clone() });
            }
            FromExtension::GetNativeConfig => {
                self.send(&ToExtension::NativeConfig { config: self.config.clone() });
            }
            FromExtension::Error { code } => {
                tracing::warn!(code = %code, "extension error");
                self.errors.push(code);
            }
            FromExtension::OpenSettings => {
                let _ = self.open_settings();
            }
        }
    }

    fn show_icon(&self, state: IconState) {
        if self.beside_shown {
            self.platform.set_beside_look(state);
        }
        if self.config.icon.visible {
            self.platform.show_icon(state, self.icon_position());
        }
    }

    fn end_recording_ui(&mut self) {
        self.recording = false;
        self.silence_stop_at = None;
        self.stop_sent = false;
        self.platform.hide_bubble();
        self.show_icon(IconState::Idle);
        // The focus left the fields during the recording: the mic goes back now.
        self.go_home_if_due();
        self.show_kept();
    }

    /// The bubble is for what is being said: kept words step aside (they stay kept).
    fn hide_kept_while_recording(&self) {
        if !self.kept.is_empty() {
            self.platform.show_kept(None);
        }
    }

    /// The kept words in their bubble at the mic's current place, when there are any and no
    /// recording needs the bubble.
    fn show_kept(&self) {
        if !self.kept.is_empty() && !self.recording {
            self.platform.show_kept(Some(&self.kept));
        }
    }

    /// Words can be kept only above a mic that can be on screen; elsewhere they are typed as before.
    fn can_keep(&self) -> bool {
        self.config.icon.visible && self.platform.keeps_words()
    }

    /// Adds words to the kept ones (English words spaced apart, as `put_in` does) and shows them,
    /// unless a recording is on: then they show when it ends.
    fn keep(&mut self, text: &str) {
        let text = if self.kept.is_empty() { text.trim_start() } else { text };
        if let (Some(a), Some(b)) = (self.kept.chars().last(), text.chars().next()) {
            if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric() {
                self.kept.push(' ');
            }
        }
        self.kept.push_str(text);
        self.show_kept();
    }

    /// A button of the kept words' bubble (plan C11).
    fn kept_button(&mut self, button: KeptButton) {
        if self.kept.is_empty() {
            self.platform.show_kept(None);
            return;
        }
        match button {
            KeptButton::Copy => match self.platform.copy_to_clipboard(&self.kept) {
                Ok(()) => {
                    self.kept.clear();
                    self.platform.show_kept(None);
                    self.platform.tell(&t("native_bubbleKeptCopied"), COPIED_HOLD, false);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not copy the kept words");
                    self.errors.push("kept_copy_failed");
                    self.platform.tell(&t("native_bubbleKeptCopyFailed"), MESSAGE_HOLD, false);
                }
            },
            KeptButton::Insert => {
                let text = std::mem::take(&mut self.kept);
                self.platform.show_kept(None);
                // Words of their own: no space in front of them for what was typed before.
                self.last_char = None;
                // The user chose where they go, so not only into what the OS takes for a text
                // field. Not into a password field: then they stay.
                if self.put_in(&text, false) == Put::Refused {
                    self.kept = text;
                    self.show_kept();
                }
            }
            KeptButton::Close => {
                self.kept.clear();
                self.platform.show_kept(None);
            }
        }
    }

    /// New words (or the recognizer hearing speech again after them): the recording stops
    /// `silence_stop_sec` from now, unless it is off or a stop was sent already.
    fn words_heard(&mut self) {
        let seconds = self.config.silence_stop_sec;
        if seconds > 0 && self.recording && !self.stop_sent {
            self.silence_stop_at = Some((self.now)() + Duration::from_secs(seconds.into()));
        }
    }

    fn session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Started => {
                self.recording = true;
                self.last_char = None;
                self.silence_stop_at = None;
                self.stop_sent = false;
                self.show_icon(IconState::Recording);
                self.hide_kept_while_recording();
                self.update_tray();
            }
            SessionEvent::Interim { text } => {
                if self.config.icon.visible {
                    self.platform.voice_cue(VoiceCue::Text);
                    self.platform.show_bubble(&text);
                }
                if !text.trim().is_empty() {
                    self.words_heard();
                }
            }
            SessionEvent::Activity { activity } => {
                if let (true, Some(cue)) = (self.config.icon.visible, VoiceCue::from_activity(&activity)) {
                    self.platform.voice_cue(cue);
                }
                // Speech again after words: the words it brings take a moment to show. Not before
                // the first words, which would count the silence before speaking.
                if activity == "speechstart" && self.silence_stop_at.is_some() {
                    self.words_heard();
                }
            }
            SessionEvent::Final { text } => {
                self.platform.hide_bubble();
                let _ = self.insert(&text);
                // Chrome sends an empty final for silence: that is not words.
                if !text.trim().is_empty() {
                    self.words_heard();
                }
            }
            SessionEvent::Ended { reason, code } => {
                tracing::info!(reason = %reason, code = ?code, "session ended");
                if let Some(code) = code {
                    self.errors.push(code);
                }
                self.end_recording_ui();
                self.update_tray();
                if std::mem::take(&mut self.send_after_stop) {
                    self.press(EditKeys::Send(self.config.send_key));
                }
            }
        }
    }

    /// Puts recognized text into the foreground app. Never into a password field; words for
    /// something the OS says is not a text field are kept above the mic instead. True when the
    /// text went in.
    fn insert(&mut self, text: &str) -> bool {
        self.put_in(text, true) == Put::In
    }

    /// Puts words into the foreground app, never into a password field. With `keep_off_fields`,
    /// words for something the OS says is not a text field are kept above the mic instead, where
    /// the user can copy or insert them (plan C11). When the OS cannot say, they are typed.
    fn put_in(&mut self, text: &str, keep_off_fields: bool) -> Put {
        if text.trim().is_empty() {
            return Put::Refused;
        }
        // Chrome ends a recognition after each utterance, so one recording yields several finals.
        // English words would run together ("helloworld"); Japanese needs no space.
        let spaced = match (self.last_char, text.chars().next()) {
            (Some(a), Some(b)) if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric() => {
                format!(" {text}")
            }
            _ => text.to_string(),
        };
        // End-to-end checks on the developer's machine (standalone plan C4): nothing is typed into
        // whatever app happens to be in front. Not for users; not documented.
        if std::env::var_os("VTYPE_TEST_NO_INJECT").is_some() {
            tracing::info!("test: final len={}", spaced.chars().count());
            self.last_char = spaced.chars().last();
            return Put::In;
        }
        let field = self.platform.focused_field();
        if is_protected_field(&field) {
            tracing::info!(len = text.chars().count(), "password field in front; not inserting");
            self.platform.tell(&t("native_notifyPasswordField"), MESSAGE_HOLD, true);
            return Put::Refused;
        }
        if keep_off_fields && field.is_text_field == Some(false) && self.can_keep() {
            tracing::info!(len = text.chars().count(), app = ?field.app_id, "no text field has the focus; keeping");
            self.keep(text);
            return Put::Kept;
        }
        let text = spaced.as_str();
        match self.platform.inject_text(text, self.config.inject) {
            Ok(InjectOutcome::Typed) | Ok(InjectOutcome::Pasted) => {
                tracing::info!(len = text.chars().count(), "inserted");
                self.last_char = text.chars().last();
                self.show_icon(IconState::Done);
                Put::In
            }
            Ok(InjectOutcome::CopiedOnly) => {
                self.platform.tell(&t("native_notifyPasteManually"), MESSAGE_HOLD, true);
                Put::Copied
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not insert; copying instead");
                self.errors.push("inject_failed");
                if self.platform.copy_to_clipboard(text).is_ok() {
                    self.platform.tell(&t("native_notifyPasteManually"), MESSAGE_HOLD, true);
                }
                Put::Copied
            }
        }
    }

    fn save_config(&mut self) {
        if let Some(path) = &self.config_path {
            if let Err(e) = config::save(path, &self.config) {
                tracing::warn!(error = %e, "could not save config.json");
                self.errors.push("config_save_failed");
            }
        }
    }

    fn apply_config(&mut self, mut new: NativeConfig) {
        // The extension does not know where the icon was dragged to; keep that here.
        if new.icon.x.is_none() && new.icon.y.is_none() {
            new.icon.x = self.config.icon.x;
            new.icon.y = self.config.icon.y;
        }
        // Consent is given on the speech page only; a settings page does not take it back.
        new.consented = self.config.consented;
        let mode_changed = new.input_mode != self.config.input_mode;
        let hotkey_changed = new.effective_hotkey() != self.config.effective_hotkey();
        let icon_changed = new.icon.visible != self.config.icon.visible;
        let scale_changed = new.icon.scale != self.config.icon.scale;
        let beside_changed = new.beside_field != self.config.beside_field;
        self.config = new;
        // Turned off during a recording: the stop already counting does not come either.
        if self.config.silence_stop_sec == 0 {
            self.silence_stop_at = None;
        }
        if beside_changed {
            self.platform.watch_fields(&self.config.beside_field);
            self.followed_field = None;
            self.home_at = None;
            if (!self.config.beside_field.enabled || self.icon_follows_fields()) && self.beside_shown {
                self.beside_shown = false;
                self.platform.hide_beside();
            }
        }
        self.save_config();
        if hotkey_changed {
            self.register_hotkey();
        }
        if scale_changed {
            self.platform.set_icon_scale(self.config.icon.scale);
        }
        if icon_changed {
            if self.config.icon.visible {
                self.show_icon(if self.recording { IconState::Recording } else { IconState::Idle });
            } else {
                self.platform.hide_icon();
            }
        }
        if scale_changed || icon_changed {
            self.show_kept();
        }
        if mode_changed {
            self.mode = self.config.input_mode;
            self.send(&ToExtension::SetMode { mode: self.mode });
        }
        self.update_tray();
    }

    /// A field taking the focus brings the floating mic there (where the platform can move it).
    fn icon_follows_fields(&self) -> bool {
        self.config.beside_field.trigger == BesideFieldTrigger::Focus && self.platform.icon_follows_fields()
    }

    /// Neither recording nor under the pointer: the floating mic may go back to the corner.
    fn mic_free(&self) -> bool {
        !self.recording && !self.mic_hovered
    }

    /// The focus has been off the fields long enough: the floating mic goes back to the corner.
    /// The same field taking the focus again then brings it back without a click.
    fn go_home_if_due(&mut self) {
        if self.home_at.is_some_and(|at| (self.now)() >= at) && self.mic_free() {
            self.home_at = None;
            self.followed_field = None;
            self.platform.icon_home();
            self.show_kept();
        }
    }

    /// The mic beside the field follows the focus (or the pointer): shown, moved or hidden. Or the
    /// floating mic comes to the field, and goes back to the corner once the focus has been off
    /// the fields for `HOME_DELAY`.
    fn field_changed(&mut self, probe: &FieldProbe, at: Instant) {
        if let Some(app) = probe.app_id.as_ref().filter(|_| !probe.on_taskbar) {
            self.last_app = Some(app.clone());
        }
        if self.icon_follows_fields() {
            let Some(anchor) = beside_field::anchor(&self.config.beside_field, probe) else {
                // Only a mic that came to a field goes back; one the focus never brought stays
                // where it started.
                if self.followed_field.is_some() && self.home_at.is_none() {
                    self.home_at = Some((self.now)() + HOME_DELAY);
                }
                return;
            };
            self.home_at = None;
            let field = (probe.app_id.clone(), probe.bounds);
            // The focus coming back to the field the mic is at: only a click moves it again.
            if self.followed_field.as_ref() == Some(&field) && !matches!(anchor, Anchor::Pointer(..)) {
                return;
            }
            self.followed_field = Some(field);
            self.platform.icon_to_field(anchor, at);
            // Kept words come along: click a field, then "Insert" right there.
            self.show_kept();
            return;
        }
        match beside_field::decide(&self.config.beside_field, probe) {
            Some(pos) => {
                let look = if self.recording { IconState::Recording } else { IconState::Idle };
                self.platform.show_beside(pos, look, at);
                self.beside_shown = true;
            }
            None if self.beside_shown => {
                self.platform.hide_beside();
                self.beside_shown = false;
            }
            None => {}
        }
    }

    /// "The mic does not come to this app's text fields" (plan C1): the app the user was in is
    /// watched for `field_check::CHECK_FOR` while the user clicks into its text field. Nothing is
    /// learnt from it yet; the bubble says what was found.
    fn check_fields(&mut self) {
        // Nothing reports fields while the mic beside them is off, so there would be nothing to see.
        if !self.config.beside_field.enabled {
            self.platform.tell(&t("native_bubbleCheckFieldsOff"), MESSAGE_HOLD, true);
            return;
        }
        let Some(app) = self.last_app.clone() else {
            self.platform.tell(&t("native_bubbleCheckFieldsNoApp"), MESSAGE_HOLD, true);
            return;
        };
        tracing::info!(app = %app, "checking the app's text fields");
        self.platform.check_fields(&app);
        let seconds = field_check::CHECK_FOR.as_secs().to_string();
        let text = t_with("native_bubbleCheckFieldsStart", &[("seconds", &seconds), ("app", &app)]);
        // Held until the result takes its place.
        self.platform.tell(&text, field_check::CHECK_FOR + Duration::from_secs(2), true);
    }

    /// The look at an app's text fields is over: the bubble says what was found, and the
    /// diagnostic info keeps it.
    fn fields_checked(&mut self, check: FieldCheck) {
        let verdict = field_check::classify(&check.seen);
        tracing::info!(app = %check.app_id, ?verdict, seen = check.seen.len(), "checked the app's text fields");
        self.platform.tell(&field_check::message(verdict, &check.app_id), FIELD_CHECK_HOLD, true);
        self.last_field_check = Some((diag::utc_now(), check));
    }

    fn handle_platform(&mut self, ev: PlatformEvent) -> Flow {
        match ev {
            PlatformEvent::MicHover(_) | PlatformEvent::FieldChanged { .. } => {}
            // The mic (or a menu) was used: its hint goes first, as the action may say something in
            // the bubble. It comes back only when the pointer moves onto another part.
            _ => self.drop_hint(),
        }
        match ev {
            PlatformEvent::MicHover(part) => {
                self.drop_hint();
                self.hint_due = part.map(|p| (p, (self.now)() + HINT_DELAY));
                // The focus left the fields while the pointer was on the mic: it goes back once
                // the pointer is off it.
                self.mic_hovered = part.is_some();
                self.go_home_if_due();
            }
            PlatformEvent::ToggleRequested => {
                let _ = self.toggle(None);
            }
            PlatformEvent::MicButton(button) => self.mic_button(button),
            PlatformEvent::KeptButton(button) => self.kept_button(button),
            PlatformEvent::FieldChanged { probe, at } => self.field_changed(&probe, at),
            PlatformEvent::FieldsChecked(check) => self.fields_checked(check),
            PlatformEvent::IconMoved { x, y } => {
                self.config.icon.x = Some(x);
                self.config.icon.y = Some(y);
                self.save_config();
                self.show_kept();
            }
            PlatformEvent::IconZoom { steps } => {
                let scale = overlay_logic::zoom_scale(self.config.icon.scale, steps);
                if scale != self.config.icon.scale {
                    self.config.icon.scale = scale;
                    self.save_config();
                    self.platform.set_icon_scale(scale);
                    self.show_kept();
                }
            }
            PlatformEvent::Menu(action) => match action {
                MenuAction::ToggleRecording => {
                    let _ = self.toggle(None);
                }
                MenuAction::ToggleIconVisible => {
                    let mut c = self.config.clone();
                    c.icon.visible = !c.icon.visible;
                    self.apply_config(c);
                    self.send(&ToExtension::NativeConfig { config: self.config.clone() });
                }
                MenuAction::ToggleHideOnFullscreen => {
                    let mut c = self.config.clone();
                    c.icon.hide_on_fullscreen = !c.icon.hide_on_fullscreen;
                    self.apply_config(c);
                    self.send(&ToExtension::NativeConfig { config: self.config.clone() });
                }
                MenuAction::SetMode(mode) => {
                    let _ = self.set_mode(mode);
                }
                MenuAction::OpenSettings => {
                    let _ = self.open_settings();
                }
                MenuAction::InsertTemplate(index) => self.insert_template(index),
                MenuAction::EditTemplate(index) => {
                    let _ = self.open_settings_at(Some(&format!("template-{index}")));
                }
                MenuAction::DeleteTemplate(index) => self.delete_template(index),
                MenuAction::UndoDeleteTemplate => self.undo_delete_template(),
                MenuAction::AddSelectionAsTemplate => self.add_selection_as_template(),
                MenuAction::ReportBug => {
                    let os = self.platform.os_description();
                    let browser = self.host.as_ref().and_then(|h| h.browser.clone()).unwrap_or_default();
                    let url = report::bug_report_url(&ReportInfo {
                        surface: Surface::Desktop,
                        version: env!("CARGO_PKG_VERSION"),
                        os: &os,
                        browser: &browser,
                    });
                    if let Err(e) = self.platform.open_url(&url) {
                        tracing::warn!(error = %e, "could not open the issue form");
                    }
                }
                MenuAction::CopyDiagnostics => {
                    let text = serde_json::to_string_pretty(&self.diagnostics()).unwrap_or_default();
                    if let Err(e) = self.platform.copy_to_clipboard(&text) {
                        tracing::warn!(error = %e, "could not copy the diagnostic info");
                    }
                }
                MenuAction::CheckFields => self.check_fields(),
                MenuAction::About => {
                    let _ = self.open_settings_at(Some("about"));
                }
                MenuAction::Quit => return Flow::Quit,
            },
        }
        Flow::Continue
    }

    /// Runs until a quit request or until every sender is gone.
    pub fn run(&mut self, rx: Receiver<Event>) {
        loop {
            let event = match self.next_deadline() {
                Some(deadline) => {
                    let wait = deadline.saturating_duration_since((self.now)());
                    match rx.recv_timeout(wait) {
                        Ok(ev) => Some(ev),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match rx.recv() {
                    Ok(ev) => Some(ev),
                    Err(_) => return,
                },
            };
            if let Some(ev) = event {
                if self.handle(ev) == Flow::Quit {
                    return;
                }
            }
            self.tick();
        }
    }
}

/// Shared with speech_host.rs, whose page connections are hosts too.
pub(crate) static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

fn serve_connection(stream: Stream, tx: Sender<Event>) {
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (recv, mut send) = stream.split();
    let (out_tx, out_rx) = mpsc::channel::<Reply>();
    thread::spawn(move || {
        for reply in out_rx {
            if ipc::write_line(&mut send, &reply).is_err() {
                break;
            }
        }
    });
    let mut reader = BufReader::new(recv);
    loop {
        match ipc::read_line::<_, Request>(&mut reader) {
            Ok(Some(req)) => {
                if tx.send(Event::Request { conn, req, out: out_tx.clone() }).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                let _ = out_tx.send(Reply::error("bad_request", e.to_string()));
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(Event::Closed { conn });
}

fn accept_loop(listener: Listener, tx: Sender<Event>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let tx = tx.clone();
                thread::spawn(move || serve_connection(stream, tx));
            }
            Err(e) => tracing::warn!(error = %e, "incoming connection failed"),
        }
    }
}

pub fn run() -> Result<()> {
    #[cfg(windows)]
    if crate::platform::windows::leave_own_console() {
        return Ok(());
    }
    crate::log::init_file("daemon");
    let endpoint = crate::paths::ipc_endpoint();
    if ipc::daemon_answers(&endpoint) {
        println!("vtype is already running");
        return Ok(());
    }
    let listener = ipc::listen_at(&endpoint).with_context(|| format!("could not listen on {endpoint}"))?;

    let platform: Arc<dyn Platform> = Arc::from(crate::platform::current());
    crate::i18n::init(&platform.ui_language());

    let config_path = crate::paths::config_file();
    let (cfg, outcome) = config::load(&config_path);
    tracing::info!(?outcome, "config");

    let (tx, rx) = mpsc::channel::<Event>();
    let accept_tx = tx.clone();
    thread::spawn(move || accept_loop(listener, accept_tx));
    let speech_page = match crate::speech_host::start(tx.clone()) {
        Ok(host) => Some(host.page_url()),
        Err(e) => {
            tracing::warn!(error = %e, "could not serve the speech page");
            None
        }
    };

    let (ptx, prx) = mpsc::channel::<PlatformEvent>();
    let forward_tx = tx.clone();
    thread::spawn(move || {
        for ev in prx {
            if forward_tx.send(Event::Platform(ev)).is_err() {
                break;
            }
        }
    });
    drop(tx);

    let core_platform = platform.clone();
    thread::spawn(move || {
        let mut core = Core::new(core_platform.clone(), cfg, Some(config_path));
        if let Some(url) = speech_page {
            core.set_speech_page(url);
        }
        core.enable_profile_grant();
        core.start();
        core.run(rx);
        tracing::info!("quit");
        core_platform.quit();
    });

    platform.run_event_loop(ptx).map_err(|e| anyhow::anyhow!("event loop: {e}"))?;
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::InjectMethod;
    use crate::platform::{Autostart, FieldInfo};
    use serde_json::json;
    use std::path::Path;
    use std::sync::Mutex;

    /// Records every call; `inject` and `field` decide what the OS "does".
    #[derive(Default)]
    pub struct FakePlatform {
        pub calls: Mutex<Vec<String>>,
        pub field: Mutex<FieldInfo>,
        pub inject: Mutex<Option<Result<InjectOutcome, PlatformError>>>,
        pub hotkey: Mutex<Option<Result<(), PlatformError>>>,
        pub chrome: Mutex<Option<Result<(), PlatformError>>>,
        /// What `copy_selection` finds selected.
        pub selection: Mutex<Option<String>>,
        /// Acts like Windows: the floating mic comes to fields.
        pub icon_follows: Mutex<bool>,
        /// Acts like Wayland: no mic to keep words above.
        pub cannot_keep: Mutex<bool>,
        /// Whether vtype starts at sign-in (off, and changeable, until set).
        pub autostart: Mutex<Option<Autostart>>,
        /// Switching starting at sign-in fails.
        pub autostart_fails: Mutex<bool>,
    }

    impl FakePlatform {
        fn log(&self, s: String) {
            self.calls.lock().unwrap().push(s);
        }
        pub fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.lock().unwrap())
        }
    }

    impl Platform for FakePlatform {
        fn run_event_loop(&self, _events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
            Ok(())
        }
        fn quit(&self) {}
        fn set_tray(&self, state: &TrayState) {
            self.log(format!("tray connected={} recording={}", state.connected, state.recording));
        }
        fn register_hotkey(&self, spec: &str) -> Result<(), PlatformError> {
            self.log(format!("hotkey {spec}"));
            self.hotkey.lock().unwrap().clone().unwrap_or(Ok(()))
        }
        fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
            self.log(format!("inject {text} {method:?}"));
            self.inject.lock().unwrap().clone().unwrap_or(Ok(InjectOutcome::Typed))
        }
        fn focused_field(&self) -> FieldInfo {
            self.field.lock().unwrap().clone()
        }
        fn press_keys(&self, keys: EditKeys) -> Result<(), PlatformError> {
            self.log(format!("keys {keys:?}"));
            Ok(())
        }
        fn copy_selection(&self) -> Result<Option<String>, PlatformError> {
            self.log("copy selection".into());
            Ok(self.selection.lock().unwrap().clone())
        }
        fn show_templates(&self, list: &menu::TemplateList) {
            self.log(format!("templates {}", describe_list(list)));
        }
        fn refresh_templates(&self, list: &menu::TemplateList) {
            self.log(format!("refresh {}", describe_list(list)));
        }
        fn show_icon(&self, state: IconState, _position: Option<(i32, i32)>) {
            self.log(format!("icon {state:?}"));
        }
        fn hide_icon(&self) {
            self.log("hide icon".into());
        }
        fn set_icon_scale(&self, percent: u16) {
            self.log(format!("icon scale {percent}"));
        }
        fn voice_cue(&self, cue: VoiceCue) {
            self.log(format!("voice {cue:?}"));
        }
        fn show_bubble(&self, text: &str) {
            self.log(format!("bubble {text}"));
        }
        fn hide_bubble(&self) {
            self.log("hide bubble".into());
        }
        fn tell(&self, text: &str, hold: Duration, or_notify: bool) {
            self.log(format!("tell {text} {}s notify={or_notify}", hold.as_secs()));
        }
        fn keeps_words(&self) -> bool {
            !*self.cannot_keep.lock().unwrap()
        }
        fn show_kept(&self, text: Option<&str>) {
            match text {
                Some(text) => self.log(format!("kept {text}")),
                None => self.log("kept gone".into()),
            }
        }
        fn autostart(&self) -> Result<Autostart, PlatformError> {
            Ok(self.autostart.lock().unwrap().unwrap_or(Autostart { enabled: false, can_change: true }))
        }
        fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
            self.log(format!("autostart {enabled}"));
            if *self.autostart_fails.lock().unwrap() {
                return Err(PlatformError::Failed("no".into()));
            }
            *self.autostart.lock().unwrap() = Some(Autostart { enabled, can_change: true });
            Ok(())
        }
        fn autostart_here(&self) {
            self.log("autostart here".into());
        }
        fn launch_chrome(&self, args: &[String]) -> Result<(), PlatformError> {
            self.log(format!("chrome {}", args.join(" ")));
            self.chrome.lock().unwrap().clone().unwrap_or(Ok(()))
        }
        fn open_url(&self, url: &str) -> Result<(), PlatformError> {
            self.log(format!("open {url}"));
            Ok(())
        }
        fn copy_to_clipboard(&self, text: &str) -> Result<(), PlatformError> {
            self.log(format!("copy {}", text.len()));
            Ok(())
        }
        fn os_description(&self) -> String {
            "TestOS 1".into()
        }
        fn ui_language(&self) -> String {
            "en".into()
        }
        fn watch_fields(&self, config: &crate::config::BesideFieldConfig) {
            self.log(format!("watch {} {:?}", config.enabled, config.trigger));
        }
        fn show_beside(&self, pos: (i32, i32), look: IconState, _reported_at: Instant) {
            self.log(format!("beside {look:?} {},{}", pos.0, pos.1));
        }
        fn hide_beside(&self) {
            self.log("hide beside".into());
        }
        fn set_beside_look(&self, look: IconState) {
            self.log(format!("beside look {look:?}"));
        }
        fn icon_follows_fields(&self) -> bool {
            *self.icon_follows.lock().unwrap()
        }
        fn icon_to_field(&self, anchor: Anchor, _reported_at: Instant) {
            self.log(format!("icon to {anchor:?}"));
        }
        fn icon_home(&self) {
            self.log("icon home".into());
        }
        fn check_fields(&self, app_id: &str) {
            self.log(format!("check fields {app_id}"));
        }
    }

    pub struct Harness {
        pub core: Core,
        pub fake: Arc<FakePlatform>,
        pub host_rx: Receiver<Reply>,
        host_tx: Sender<Reply>,
        pub clock: Arc<Mutex<Instant>>,
    }

    impl Harness {
        pub fn new() -> Harness {
            let fake = Arc::new(FakePlatform::default());
            let mut core = Core::new(fake.clone(), NativeConfig::default(), None);
            core.set_speech_page(PAGE.into());
            core.profile_dir = PathBuf::from("/p");
            let clock = Arc::new(Mutex::new(Instant::now()));
            let c = clock.clone();
            core.set_clock(Box::new(move || *c.lock().unwrap()));
            let (host_tx, host_rx) = mpsc::channel();
            Harness { core, fake, host_rx, host_tx, clock }
        }

        pub fn cli(&mut self, req: Request) -> Reply {
            let (tx, rx) = mpsc::channel();
            self.core.handle(Event::Request { conn: 999, req, out: tx });
            rx.try_recv().expect("a reply")
        }

        /// The daemon's own speech page connects (speech_host.rs) and says hello.
        pub fn page_hello(&mut self) {
            self.core.handle(Event::Request {
                conn: 1,
                req: Request::HostHello { origin: "http://127.0.0.1:47213".into() },
                out: self.host_tx.clone(),
            });
            self.ext(json!({"type":"hello","extensionVersion":"desktop-page"}));
        }

        pub fn attach_host(&mut self) {
            self.core.handle(Event::Request {
                conn: 1,
                req: Request::HostHello { origin: "chrome-extension://abcdefghijklmnopabcdefghijklmnop/".into() },
                out: self.host_tx.clone(),
            });
        }

        pub fn ext(&mut self, message: Value) {
            self.core.handle(Event::Request {
                conn: 1,
                req: Request::FromExtension { message },
                out: self.host_tx.clone(),
            });
        }

        pub fn connect(&mut self) {
            self.attach_host();
            self.ext(json!({"type":"hello","extensionVersion":"0.1.0","browser":"Chrome 140"}));
            self.sent();
            self.fake.take();
        }

        /// Messages sent to the extension since the last call.
        pub fn sent(&self) -> Vec<Value> {
            self.host_rx
                .try_iter()
                .filter_map(|r| match r {
                    Reply::ToExtension { message } => Some(message),
                    _ => None,
                })
                .collect()
        }

        pub fn advance(&self, d: Duration) {
            let mut now = self.clock.lock().unwrap();
            *now += d;
        }
    }

    #[test]
    fn status_reports_not_connected_before_any_extension() {
        let mut h = Harness::new();
        assert_eq!(
            h.cli(Request::Status),
            Reply::Status {
                connected: false,
                recording: false,
                mode: InputMode::Normal,
                version: env!("CARGO_PKG_VERSION").into()
            }
        );
    }

    const PAGE: &str = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/speech";

    fn chrome_call(url: &str, placement: WindowPlacement) -> String {
        format!("chrome {}", chrome_launch::speech_args(Path::new("/p"), url, placement).join(" "))
    }

    #[test]
    fn the_first_press_opens_the_setup_window_and_records_once_it_is_hidden() {
        let mut h = Harness::new();
        assert_eq!(h.cli(Request::Toggle { mode: Some(InputMode::En) }), Reply::Ok);
        let calls = h.fake.take();
        assert!(calls.contains(&chrome_call(&format!("{PAGE}?setup=1"), WindowPlacement::Visible)), "{calls:?}");
        assert!(calls.contains(&format!("tell {} 30s notify=true", t("native_bubbleConsent"))), "{calls:?}");
        // The user takes as long as they take: no 10-second wait.
        assert!(h.core.next_deadline().is_none());

        // The first-run page connects: it is not asked to record.
        h.page_hello();
        assert!(!h.sent().iter().any(|m| m["type"] == "start"));
        h.ext(json!({"type":"consent"}));
        assert!(h.core.config().consented);
        // Consent is all the first-run window asks: the microphone is allowed in the profile.
        h.ext(json!({"type":"page-state","consented":true,"micGranted":false}));
        // The page closes itself; Chrome comes back off screen a moment later.
        h.core.handle(Event::Closed { conn: 1 });
        h.fake.take();
        assert!(h.core.next_deadline().is_some());
        h.advance(RELAUNCH_DELAY);
        h.core.tick();
        let calls = h.fake.take();
        assert!(calls.contains(&chrome_call(&format!("{PAGE}?consent=1"), WindowPlacement::Hidden)), "{calls:?}");
        h.page_hello();
        assert_eq!(h.sent().last().unwrap(), &json!({"type":"start","mode":"en"}));
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn once_agreed_chrome_starts_off_screen_and_gives_up_after_ten_seconds() {
        let mut h = Harness::new();
        h.core.config.consented = true;
        h.cli(Request::Start { mode: None });
        let calls = h.fake.take();
        assert!(calls.contains(&chrome_call(&format!("{PAGE}?consent=1"), WindowPlacement::Hidden)), "{calls:?}");
        // The wait is said in the bubble (held past CHROME_WAIT), never as a notification.
        assert!(calls.contains(&format!("tell {} 12s notify=false", t("native_bubbleConnecting"))), "{calls:?}");
        // A second press while waiting does not start Chrome again.
        h.cli(Request::Start { mode: None });
        assert!(!h.fake.take().iter().any(|c| c.starts_with("chrome") || c.starts_with("tell ")));
        h.advance(Duration::from_secs(9));
        h.core.tick();
        assert!(h.fake.take().is_empty());
        h.advance(Duration::from_secs(2));
        h.core.tick();
        assert_eq!(h.fake.take(), vec![format!("tell {} 6s notify=true", t("native_notifySpeechPageNotConnected"))]);
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn a_missing_chrome_is_said_at_once() {
        let mut h = Harness::new();
        *h.fake.chrome.lock().unwrap() = Some(Err(PlatformError::Failed("Chrome was not found".into())));
        h.cli(Request::Start { mode: None });
        let calls = h.fake.take();
        assert!(calls.contains(&format!("tell {} 6s notify=true", t("native_notifyChromeMissing"))), "{calls:?}");
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn a_microphone_grant_that_does_not_hold_keeps_the_window_up_instead_of_asking_again() {
        let mut h = Harness::new();
        h.core.config.consented = true;
        h.cli(Request::Start { mode: Some(InputMode::Kana) });
        h.page_hello();
        h.sent(); // the off-screen page was asked to record; it has no microphone
        h.ext(json!({"type":"page-state","consented":true,"micGranted":false}));
        assert!(h.fake.take().contains(&format!("tell {} 30s notify=true", t("native_bubbleConsent"))));
        h.core.handle(Event::Closed { conn: 1 });
        h.advance(RELAUNCH_DELAY);
        h.core.tick();
        let kept = chrome_call(&format!("{PAGE}?consent=1&stay=1"), WindowPlacement::Kept);
        let calls = h.fake.take();
        assert!(calls.contains(&kept), "{calls:?}");
        // The kept window asks for the microphone; once it has it, the recording starts there.
        h.page_hello();
        assert!(!h.sent().iter().any(|m| m["type"] == "start"));
        h.ext(json!({"type":"page-state","consented":true,"micGranted":true}));
        assert_eq!(h.sent().last().unwrap(), &json!({"type":"start","mode":"kana"}));
        // Closed by the user: the next press opens the kept window again, not the off-screen one.
        h.core.handle(Event::Closed { conn: 1 });
        assert!(h.core.next_deadline().is_none());
        h.fake.take();
        h.cli(Request::Start { mode: None });
        assert!(h.fake.take().contains(&kept));
    }

    #[test]
    fn closing_the_setup_window_before_agreeing_stops_waiting() {
        let mut h = Harness::new();
        h.cli(Request::Start { mode: None });
        h.page_hello();
        h.fake.take();
        h.core.handle(Event::Closed { conn: 1 });
        assert!(h.fake.take().contains(&"hide bubble".to_string()));
        assert!(h.core.next_deadline().is_none());
        // The next press opens the setup window again.
        h.cli(Request::Start { mode: None });
        assert!(h.fake.take().contains(&chrome_call(&format!("{PAGE}?setup=1"), WindowPlacement::Visible)));
    }

    #[test]
    fn the_mode_is_kept_in_the_config_and_told_to_every_page() {
        let mut h = Harness::new();
        h.cli(Request::SetMode { mode: InputMode::Kana });
        assert_eq!(h.core.config().input_mode, InputMode::Kana);
        h.connect();
        // A page started later hears the mode, and the table with the config.
        h.core.config.replacements =
            vec![config::ReplacementRule { from: "ブイタイプ".into(), to: "vtype".into() }];
        h.page_hello();
        let sent = h.sent();
        assert!(sent.contains(&json!({"type":"set-mode","mode":"kana"})), "{sent:?}");
        let cfg = sent.iter().find(|m| m["type"] == "native-config").expect("native-config");
        assert_eq!(cfg["config"]["replacements"], json!([{"from":"ブイタイプ","to":"vtype"}]));
        // A daemon started with this config starts in that mode.
        let restarted = Core::new(h.fake.clone(), h.core.config().clone(), None);
        assert_eq!(restarted.mode, InputMode::Kana);
    }

    #[test]
    fn a_settings_page_does_not_take_the_consent_back() {
        let mut h = Harness::new();
        h.core.config.consented = true;
        h.connect();
        h.ext(json!({"type":"set-native-config","config": NativeConfig::default()}));
        assert!(h.core.config().consented);
    }
    #[test]
    fn stopping_while_waiting_takes_the_connecting_bubble_away() {
        let mut h = Harness::new();
        h.cli(Request::Start { mode: None });
        h.fake.take();
        h.cli(Request::Stop);
        assert!(h.fake.take().contains(&"hide bubble".to_string()));
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn toggle_while_connected_starts_then_stops() {
        let mut h = Harness::new();
        h.connect();
        h.cli(Request::Toggle { mode: None });
        assert_eq!(h.sent(), vec![json!({"type":"start"})]);
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.cli(Request::Toggle { mode: None });
        assert_eq!(h.sent(), vec![json!({"type":"stop"})]);
    }

    #[test]
    fn shows_interim_and_types_only_the_final_text() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.ext(json!({"type":"session","event":{"kind":"interim","text":"hel"}}));
        h.ext(json!({"type":"session","event":{"kind":"final","text":"hello"}}));
        h.ext(json!({"type":"session","event":{"kind":"ended","reason":"stopped"}}));
        let calls = h.fake.take();
        let inject: Vec<_> = calls.iter().filter(|c| c.starts_with("inject")).collect();
        assert_eq!(inject, vec!["inject hello Auto"]);
        assert!(calls.contains(&"bubble hel".to_string()));
        assert!(calls.contains(&"icon Done".to_string()));
        assert!(!h.core.recording);
    }

    #[test]
    fn the_mode_button_goes_round_the_modes() {
        let mut h = Harness::new();
        h.connect();
        for expected in [InputMode::En, InputMode::Kana, InputMode::Normal] {
            h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Mode)));
            assert_eq!(h.core.mode, expected);
        }
    }

    #[test]
    fn the_send_button_presses_the_chosen_key() {
        let mut h = Harness::new();
        h.connect();
        h.fake.take();
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Send)));
        assert!(h.fake.take().contains(&"keys Send(Enter)".to_string()));
        h.core.config.send_key = crate::config::SendKey::CtrlEnter;
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Send)));
        assert!(h.fake.take().contains(&"keys Send(CtrlEnter)".to_string()));
    }

    fn hover(h: &mut Harness, part: Option<MicPart>) {
        h.core.handle(Event::Platform(PlatformEvent::MicHover(part)));
    }

    fn hint_call(text: &str) -> String {
        format!("tell {text} 10s notify=false")
    }

    #[test]
    fn resting_on_the_mic_says_what_it_does_and_leaving_takes_it_away() {
        let mut h = Harness::new();
        h.connect();
        hover(&mut h, Some(MicPart::Mic));
        // Passing over the mic says nothing.
        assert!(h.fake.take().is_empty());
        h.advance(HINT_DELAY - Duration::from_millis(1));
        h.core.tick();
        assert!(h.fake.take().is_empty());
        h.advance(Duration::from_millis(1));
        h.core.tick();
        assert_eq!(h.fake.take(), vec![hint_call(&t("native_hintMic"))]);
        assert!(h.core.next_deadline().is_none());
        hover(&mut h, None);
        assert_eq!(h.fake.take(), vec!["hide bubble"]);
        // Left before the hint showed: nothing to take away, nothing shows later.
        hover(&mut h, Some(MicPart::Mic));
        hover(&mut h, None);
        h.advance(HINT_DELAY);
        h.core.tick();
        assert!(h.fake.take().is_empty());
    }

    #[test]
    fn each_corner_button_says_what_it_does() {
        let mut h = Harness::new();
        h.connect();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::SetMode(InputMode::Kana))));
        h.fake.take();
        let expected = [
            (MicButton::Templates, t("native_hintTemplates")),
            (MicButton::Mode, t_with("native_hintMode", &[("mode", &t("native_trayModeKana"))])),
            (MicButton::Send, t_with("native_hintSend", &[("key", "Enter")])),
            (MicButton::Clear, t("native_hintClear")),
        ];
        for (button, text) in expected {
            hover(&mut h, Some(MicPart::Button(button)));
            h.advance(HINT_DELAY);
            h.core.tick();
            let calls = h.fake.take();
            // Moving on from the previous button takes its hint away first.
            assert_eq!(calls.last(), Some(&hint_call(&text)), "{button:?}");
        }
        // The placeholders are the dictionary's own: nothing is left unfilled.
        for button in [MicButton::Mode, MicButton::Send] {
            let text = h.core.hint_text(MicPart::Button(button));
            assert!(!text.contains('{'), "{text}");
        }
    }

    #[test]
    fn using_the_mic_takes_its_hint_away_before_the_button_speaks_and_it_does_not_come_back() {
        let mut h = Harness::new();
        h.connect();
        hover(&mut h, Some(MicPart::Button(MicButton::Clear)));
        h.advance(HINT_DELAY);
        h.core.tick();
        h.fake.take();
        // The focus is not in a text field: the button says so, after the hint went.
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Clear)));
        let calls = h.fake.take();
        assert_eq!(calls.first().map(String::as_str), Some("hide bubble"), "{calls:?}");
        assert!(calls.iter().any(|c| c.contains(&t("native_bubbleClearNotField"))), "{calls:?}");
        // Still on the button: its hint stays away (the message stays up).
        h.advance(HINT_DELAY * 2);
        h.core.tick();
        assert!(h.fake.take().is_empty());
        // A click before the hint showed cancels it.
        hover(&mut h, Some(MicPart::Button(MicButton::Mode)));
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Mode)));
        h.fake.take();
        h.advance(HINT_DELAY);
        h.core.tick();
        assert!(!h.fake.take().iter().any(|c| c.starts_with("tell ")));
    }

    #[test]
    fn no_hint_while_the_bubble_shows_what_is_being_said() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.fake.take();
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HINT_DELAY);
        h.core.tick();
        assert!(h.fake.take().is_empty());
        hover(&mut h, None);
        assert!(h.fake.take().is_empty(), "the live text stays");
        // A hint that ran out by itself is not taken away again (the bubble may say something else).
        h.ext(json!({"type":"session","event":{"kind":"ended","reason":"stopped"}}));
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HINT_DELAY);
        h.core.tick();
        h.fake.take();
        h.advance(HINT_HOLD);
        hover(&mut h, None);
        assert!(h.fake.take().is_empty());
    }

    #[test]
    fn ctrl_wheel_and_the_settings_page_resize_the_mic_within_the_limits() {
        let mut h = Harness::new();
        h.core.start();
        assert!(h.fake.take().contains(&"icon scale 100".to_string()));
        h.core.handle(Event::Platform(PlatformEvent::IconZoom { steps: 2 }));
        assert_eq!(h.core.config.icon.scale, 120);
        assert_eq!(h.fake.take(), vec!["icon scale 120"]);
        h.core.handle(Event::Platform(PlatformEvent::IconZoom { steps: -20 }));
        assert_eq!(h.core.config.icon.scale, 50);
        h.fake.take();
        // Already the smallest: nothing to redo.
        h.core.handle(Event::Platform(PlatformEvent::IconZoom { steps: -1 }));
        assert!(h.fake.take().is_empty());
        h.connect();
        let mut c = h.core.config.clone();
        c.icon.scale = 100;
        h.ext(json!({"type":"set-native-config","config": c}));
        assert!(h.fake.take().contains(&"icon scale 100".to_string()));
        assert_eq!(h.core.config.icon.scale, 100);
    }

    #[test]
    fn sending_while_recording_stops_first_and_sends_after_the_last_text() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.fake.take();
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Send)));
        assert_eq!(h.sent(), vec![json!({"type":"stop"})]);
        assert!(!h.fake.take().iter().any(|c| c.starts_with("keys")), "not before the text is in");
        h.ext(json!({"type":"session","event":{"kind":"final","text":"hello"}}));
        h.ext(json!({"type":"session","event":{"kind":"ended","reason":"user"}}));
        let calls = h.fake.take();
        let inject = calls.iter().position(|c| c.starts_with("inject")).unwrap();
        let keys = calls.iter().position(|c| c == "keys Send(Enter)").unwrap();
        assert!(inject < keys, "{calls:?}");
        // Only once.
        h.ext(json!({"type":"session","event":{"kind":"ended","reason":"user"}}));
        assert!(!h.fake.take().iter().any(|c| c.starts_with("keys")));
    }

    /// Connected, and a recording under way (with the default 3 seconds to stop after speech).
    fn recording(h: &mut Harness) {
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.sent();
        h.fake.take();
    }

    fn session(h: &mut Harness, event: Value) {
        h.ext(json!({"type":"session","event": event}));
    }

    /// How many stops went to the page since the last look.
    fn stops_sent(h: &Harness) -> usize {
        h.sent().iter().filter(|m| m["type"] == "stop").count()
    }

    fn now(h: &Harness) -> Instant {
        *h.clock.lock().unwrap()
    }

    #[test]
    fn stops_the_set_seconds_after_the_last_words_and_not_before() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"interim","text":"hel"}));
        assert_eq!(h.core.next_deadline(), Some(now(&h) + Duration::from_secs(3)));
        h.advance(Duration::from_millis(2999));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
        h.advance(Duration::from_millis(1));
        h.core.tick();
        assert_eq!(stops_sent(&h), 1);
        assert!(h.core.next_deadline().is_none());
        // Another setting, another wait.
        let mut h = Harness::new();
        h.core.config.silence_stop_sec = 7;
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        assert_eq!(h.core.next_deadline(), Some(now(&h) + Duration::from_secs(7)));
    }

    #[test]
    fn new_words_start_the_count_again() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"interim","text":"hel"}));
        h.advance(Duration::from_secs(2));
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.advance(Duration::from_secs(2));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
        // Speech heard again after the words: counted from there, as its words take a moment.
        session(&mut h, json!({"kind":"activity","activity":"speechstart"}));
        h.advance(Duration::from_millis(2500));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
        h.advance(Duration::from_millis(500));
        h.core.tick();
        assert_eq!(stops_sent(&h), 1);
    }

    #[test]
    fn no_stop_before_the_user_starts_speaking() {
        let mut h = Harness::new();
        recording(&mut h);
        // Sound and speech without words yet, and the empty finals Chrome sends for silence.
        session(&mut h, json!({"kind":"activity","activity":"speechstart"}));
        session(&mut h, json!({"kind":"final","text":""}));
        session(&mut h, json!({"kind":"interim","text":"  "}));
        assert!(h.core.next_deadline().is_none());
        h.advance(Duration::from_secs(60));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
        // Nor do empty finals after words start the count again.
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.advance(Duration::from_secs(2));
        session(&mut h, json!({"kind":"final","text":""}));
        h.advance(Duration::from_secs(1));
        h.core.tick();
        assert_eq!(stops_sent(&h), 1);
    }

    #[test]
    fn no_stop_when_the_setting_is_off() {
        let mut h = Harness::new();
        h.core.config.silence_stop_sec = 0;
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        session(&mut h, json!({"kind":"activity","activity":"speechstart"}));
        assert!(h.core.next_deadline().is_none());
        h.advance(Duration::from_secs(60));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
    }

    #[test]
    fn turning_it_off_during_a_recording_drops_the_stop_counting() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        assert!(h.core.next_deadline().is_some());
        let mut c = h.core.config.clone();
        c.silence_stop_sec = 0;
        h.ext(json!({"type":"set-native-config","config": c}));
        assert!(h.core.next_deadline().is_none());
        h.advance(Duration::from_secs(10));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
    }

    #[test]
    fn words_after_a_stop_do_not_send_it_again() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"interim","text":"hel"}));
        h.advance(Duration::from_secs(3));
        h.core.tick();
        assert_eq!(stops_sent(&h), 1);
        // The page waits for the last words before it ends.
        session(&mut h, json!({"kind":"final","text":"hello"}));
        session(&mut h, json!({"kind":"activity","activity":"speechstart"}));
        assert!(h.core.next_deadline().is_none());
        h.advance(Duration::from_secs(10));
        h.core.tick();
        assert_eq!(stops_sent(&h), 0);
        // The same after the user stopped.
        session(&mut h, json!({"kind":"ended","reason":"user"}));
        session(&mut h, json!({"kind":"started"}));
        session(&mut h, json!({"kind":"interim","text":"hel"}));
        h.cli(Request::Stop);
        assert_eq!(stops_sent(&h), 1);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        assert!(h.core.next_deadline().is_none());
        // The next recording counts again.
        session(&mut h, json!({"kind":"ended","reason":"user"}));
        session(&mut h, json!({"kind":"started"}));
        session(&mut h, json!({"kind":"final","text":"again"}));
        assert!(h.core.next_deadline().is_some());
    }

    #[test]
    fn the_end_of_the_recording_or_of_the_page_forgets_the_stop() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        session(&mut h, json!({"kind":"ended","reason":"silence"}));
        assert!(h.core.next_deadline().is_none());
        session(&mut h, json!({"kind":"started"}));
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.core.handle(Event::Closed { conn: 1 });
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn stopping_on_silence_does_not_press_the_send_key() {
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"interim","text":"hel"}));
        h.advance(Duration::from_secs(3));
        h.core.tick();
        assert_eq!(stops_sent(&h), 1);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        session(&mut h, json!({"kind":"ended","reason":"user"}));
        let calls = h.fake.take();
        assert!(calls.contains(&"inject hello Auto".to_string()), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("keys")), "{calls:?}");
    }

    #[test]
    fn the_clear_button_empties_only_an_editable_text_field() {
        let mut h = Harness::new();
        h.connect();
        h.fake.take();
        let clear = |h: &mut Harness, field: FieldInfo| {
            *h.fake.field.lock().unwrap() = field;
            h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Clear)));
            h.fake.take()
        };
        let text = FieldInfo { is_text_field: Some(true), is_password: Some(false), ..FieldInfo::default() };
        assert!(clear(&mut h, text.clone()).contains(&"keys ClearField".to_string()));
        for field in [
            FieldInfo::default(),
            FieldInfo { is_text_field: Some(false), ..FieldInfo::default() },
            FieldInfo { is_password: Some(true), ..text },
        ] {
            let calls = clear(&mut h, field.clone());
            assert!(!calls.iter().any(|c| c.starts_with("keys")), "{field:?}");
            assert!(calls.iter().any(|c| c.contains(&t("native_bubbleClearNotField"))), "{calls:?}");
        }
    }

    #[test]
    fn the_templates_button_opens_the_menu_and_a_chosen_template_goes_in() {
        let mut h = Harness::new();
        h.connect();
        h.core.config.templates = vec!["お世話になっております。".into(), "hello".into()];
        h.fake.take();
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Templates)));
        assert_eq!(h.fake.take(), vec!["templates お世話になっております。,hello"]);
        // No space in front of a template, even after English words.
        h.core.last_char = Some('d');
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::InsertTemplate(1))));
        let calls = h.fake.take();
        assert!(calls.contains(&"inject hello Auto".to_string()), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("keys")), "not sent unless asked");
        // Sent right away when the user asked for that.
        h.core.config.template_send_immediate = true;
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::InsertTemplate(0))));
        let calls = h.fake.take();
        let inject = calls.iter().position(|c| c.starts_with("inject お世話")).unwrap();
        let keys = calls.iter().position(|c| c == "keys Send(Enter)").unwrap();
        assert!(inject < keys);
        // Not into a password field, and then not sent either.
        *h.fake.field.lock().unwrap() = FieldInfo { is_password: Some(true), ..FieldInfo::default() };
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::InsertTemplate(0))));
        assert!(!h.fake.take().iter().any(|c| c.starts_with("inject") || c.starts_with("keys")));
        // An index that is gone does nothing.
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::InsertTemplate(9))));
    }

    #[test]
    fn a_template_deleted_from_the_list_can_be_put_back_where_it_was() {
        let mut h = Harness::new();
        h.connect();
        h.core.config.templates = vec!["a".into(), "b".into(), "c".into()];
        h.fake.take();
        let told = |calls: &[String], key: &str| calls.iter().any(|c| c.contains(&t(key)));
        let refreshed = |calls: &[String], rows: &str| calls.iter().any(|c| *c == format!("refresh {rows}"));
        // Gone at once, and the open list shows where it was (no bubble: the list says it).
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::DeleteTemplate(1))));
        let calls = h.fake.take();
        assert!(refreshed(&calls, "a,undo,c"), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("tell")), "{calls:?}");
        assert_eq!(h.core.config.templates, vec!["a".to_string(), "c".to_string()]);
        // Still offered the next time the list opens.
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Templates)));
        assert_eq!(h.fake.take(), vec!["templates a,undo,c"]);
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::UndoDeleteTemplate)));
        assert!(refreshed(&h.fake.take(), "a,b,c"));
        assert_eq!(h.core.config.templates, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        // Once only.
        h.core.handle(Event::Platform(PlatformEvent::MicButton(MicButton::Templates)));
        assert_eq!(h.fake.take(), vec!["templates a,b,c"]);
        // Put back last when the list got shorter meanwhile; not twice when it came back already.
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::DeleteTemplate(2))));
        h.core.config.templates = vec!["x".into()];
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::UndoDeleteTemplate)));
        assert_eq!(h.core.config.templates, vec!["x".to_string(), "c".to_string()]);
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::DeleteTemplate(1))));
        h.core.config.templates.push("c".into());
        h.fake.take();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::UndoDeleteTemplate)));
        assert!(told(&h.fake.take(), "native_bubbleTemplateDuplicate"));
        assert_eq!(h.core.config.templates, vec!["x".to_string(), "c".to_string()]);
        // An index that is gone deletes nothing, but the list (closed by the click on Windows)
        // is shown again.
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::DeleteTemplate(9))));
        assert_eq!(h.fake.take(), vec!["refresh x,c"]);
        assert_eq!(h.core.config.templates, vec!["x".to_string(), "c".to_string()]);
    }

    /// The rows of a templates list for the fake's log: each template's label, `undo` for the
    /// deleted one.
    fn describe_list(list: &menu::TemplateList) -> String {
        let rows: Vec<&str> = list
            .rows
            .iter()
            .map(|row| match row {
                menu::TemplateRow::Template { label, .. } => label.as_str(),
                menu::TemplateRow::Deleted { .. } => "undo",
            })
            .collect();
        rows.join(",")
    }

    #[test]
    fn editing_a_template_opens_the_settings_page_at_it() {
        let mut h = Harness::new();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::EditTemplate(2))));
        let calls = h.fake.take();
        assert!(calls.iter().any(|c| c.starts_with("chrome ") && c.contains("/settings#template-2")), "{calls:?}");
    }

    #[test]
    fn the_selection_becomes_a_template_once() {
        let mut h = Harness::new();
        h.connect();
        let add = |h: &mut Harness, selected: Option<&str>| {
            *h.fake.selection.lock().unwrap() = selected.map(str::to_string);
            h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::AddSelectionAsTemplate)));
            h.fake.take()
        };
        let told = |calls: &[String], key: &str| calls.iter().any(|c| c.contains(&t(key)));
        assert!(told(&add(&mut h, Some("  よろしくお願いします。\n")), "native_bubbleTemplateAdded"));
        assert_eq!(h.core.config.templates, vec!["よろしくお願いします。".to_string()]);
        assert!(told(&add(&mut h, Some("よろしくお願いします。")), "native_bubbleTemplateDuplicate"));
        assert!(told(&add(&mut h, None), "native_bubbleNoSelection"));
        assert!(told(&add(&mut h, Some("   ")), "native_bubbleNoSelection"));
        h.core.config.templates = (0..config::MAX_TEMPLATES).map(|i| format!("t{i}")).collect();
        assert!(told(&add(&mut h, Some("one more")), "native_bubbleTemplateFull"));
        assert_eq!(h.core.config.templates.len(), config::MAX_TEMPLATES);
    }

    #[test]
    fn what_the_recognizer_hears_moves_the_ripple() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.fake.take();
        h.ext(json!({"type":"session","event":{"kind":"activity","activity":"speechstart"}}));
        h.ext(json!({"type":"session","event":{"kind":"activity","activity":"audiostart"}}));
        h.ext(json!({"type":"session","event":{"kind":"interim","text":"hel"}}));
        h.ext(json!({"type":"session","event":{"kind":"activity","activity":"speechend"}}));
        let voice: Vec<_> = h.fake.take().into_iter().filter(|c| c.starts_with("voice")).collect();
        assert_eq!(voice, vec!["voice Speech", "voice Text", "voice SpeechEnd"]);
    }

    #[test]
    fn spaces_english_finals_apart_but_not_japanese() {
        let mut h = Harness::new();
        h.connect();
        let final_text = |h: &mut Harness, t: &str| {
            h.ext(json!({"type":"session","event":{"kind":"final","text": t}}));
        };
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        final_text(&mut h, "hello");
        final_text(&mut h, "world");
        final_text(&mut h, "こんにちは");
        final_text(&mut h, "です");
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        final_text(&mut h, "again");
        let injected: Vec<String> = h.fake.take().into_iter().filter(|c| c.starts_with("inject ")).collect();
        assert_eq!(
            injected,
            vec![
                "inject hello Auto",
                "inject  world Auto",
                "inject こんにちは Auto",
                "inject です Auto",
                "inject again Auto"
            ]
        );
    }

    #[test]
    fn never_types_into_a_password_field() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() =
            FieldInfo { is_password: Some(true), caret_rect: Some(Rect::default()), app_id: None, is_text_field: None };
        h.ext(json!({"type":"session","event":{"kind":"final","text":"secret words"}}));
        let calls = h.fake.take();
        assert!(!calls.iter().any(|c| c.starts_with("inject")));
        assert!(calls.contains(&format!("tell {} 6s notify=true", t("native_notifyPasswordField"))));
    }

    #[test]
    fn never_types_into_a_credential_dialog() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() = FieldInfo {
            is_password: None,
            caret_rect: Some(Rect::default()),
            app_id: Some("CredentialUIBroker.exe".into()),
            is_text_field: Some(true),
        };
        h.ext(json!({"type":"session","event":{"kind":"final","text":"secret words"}}));
        let calls = h.fake.take();
        assert!(!calls.iter().any(|c| c.starts_with("inject")));
        assert!(calls.contains(&format!("tell {} 6s notify=true", t("native_notifyPasswordField"))));
    }

    fn not_a_field() -> FieldInfo {
        FieldInfo { is_text_field: Some(false), is_password: Some(false), ..FieldInfo::default() }
    }

    fn kept_button(h: &mut Harness, button: KeptButton) {
        h.core.handle(Event::Platform(PlatformEvent::KeptButton(button)));
    }

    #[test]
    fn words_off_the_fields_are_kept_and_shown_when_the_recording_ends() {
        let mut h = Harness::new();
        *h.fake.field.lock().unwrap() = not_a_field();
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        session(&mut h, json!({"kind":"final","text":"world"}));
        session(&mut h, json!({"kind":"final","text":"です"}));
        let calls = h.fake.take();
        assert!(!calls.iter().any(|c| c.starts_with("inject")), "not typed into what is not a field");
        assert!(!calls.iter().any(|c| c.starts_with("kept")), "the bubble is for the live text meanwhile");
        session(&mut h, json!({"kind":"ended","reason":"stopped"}));
        assert!(h.fake.take().contains(&"kept hello worldです".to_string()));
        // Kept until the user decides; the next recording steps them aside and adds to them.
        session(&mut h, json!({"kind":"started"}));
        assert!(h.fake.take().contains(&"kept gone".to_string()));
        session(&mut h, json!({"kind":"final","text":"again"}));
        session(&mut h, json!({"kind":"ended","reason":"stopped"}));
        assert!(h.fake.take().contains(&"kept hello worldですagain".to_string()));
    }

    #[test]
    fn words_are_typed_when_the_os_cannot_say_or_no_mic_can_keep_them() {
        // The OS cannot say what has the focus: typed, as before.
        let mut h = Harness::new();
        recording(&mut h);
        session(&mut h, json!({"kind":"final","text":"hello"}));
        assert!(h.fake.take().contains(&"inject hello Auto".to_string()));
        // No mic on screen (switched off), or none this system can show (Wayland).
        for wayland in [false, true] {
            let mut h = Harness::new();
            *h.fake.field.lock().unwrap() = not_a_field();
            if wayland {
                *h.fake.cannot_keep.lock().unwrap() = true;
            } else {
                h.core.config.icon.visible = false;
            }
            recording(&mut h);
            session(&mut h, json!({"kind":"final","text":"hello"}));
            session(&mut h, json!({"kind":"ended","reason":"stopped"}));
            let calls = h.fake.take();
            assert!(calls.contains(&"inject hello Auto".to_string()));
            assert!(!calls.iter().any(|c| c.starts_with("kept")));
        }
    }

    #[test]
    fn a_template_off_the_fields_is_kept_and_not_sent() {
        let mut h = Harness::new();
        h.connect();
        h.core.config.templates = vec!["お世話になっております。".into()];
        h.core.config.template_send_immediate = true;
        *h.fake.field.lock().unwrap() = not_a_field();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::InsertTemplate(0))));
        let calls = h.fake.take();
        assert!(calls.contains(&"kept お世話になっております。".to_string()));
        assert!(!calls.iter().any(|c| c.starts_with("inject") || c.starts_with("keys")));
    }

    #[test]
    fn copy_puts_the_kept_words_on_the_clipboard() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() = not_a_field();
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.fake.take();
        kept_button(&mut h, KeptButton::Copy);
        let calls = h.fake.take();
        assert_eq!(
            calls,
            vec![
                "copy 5".to_string(),
                "kept gone".into(),
                format!("tell {} 2s notify=false", t("native_bubbleKeptCopied"))
            ]
        );
        assert!(h.core.kept.is_empty());
    }

    #[test]
    fn insert_types_the_kept_words_wherever_the_focus_is() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() = not_a_field();
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.core.last_char = Some('x');
        h.fake.take();
        // Still not what the OS takes for a text field: the user chose, so in they go, unspaced.
        kept_button(&mut h, KeptButton::Insert);
        let calls = h.fake.take();
        assert!(calls.contains(&"kept gone".to_string()));
        assert!(calls.contains(&"inject hello Auto".to_string()));
        assert!(h.core.kept.is_empty());
        // Nothing kept: nothing to do.
        kept_button(&mut h, KeptButton::Insert);
        assert!(!h.fake.take().iter().any(|c| c.starts_with("inject")));
    }

    #[test]
    fn insert_keeps_the_words_when_a_password_field_is_in_front() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() = not_a_field();
        session(&mut h, json!({"kind":"final","text":"hello"}));
        *h.fake.field.lock().unwrap() = FieldInfo { is_password: Some(true), ..FieldInfo::default() };
        h.fake.take();
        kept_button(&mut h, KeptButton::Insert);
        let calls = h.fake.take();
        assert!(!calls.iter().any(|c| c.starts_with("inject")));
        assert_eq!(calls.last().map(String::as_str), Some("kept hello"));
        assert_eq!(h.core.kept, "hello");
    }

    #[test]
    fn close_throws_the_kept_words_away() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.field.lock().unwrap() = not_a_field();
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.fake.take();
        kept_button(&mut h, KeptButton::Close);
        assert_eq!(h.fake.take(), vec!["kept gone".to_string()]);
        assert!(h.core.kept.is_empty());
    }

    #[test]
    fn kept_words_keep_hints_away_and_follow_the_mic() {
        let mut h = following_fields();
        *h.fake.field.lock().unwrap() = not_a_field();
        session(&mut h, json!({"kind":"final","text":"hello"}));
        h.fake.take();
        // No hint over the kept words: the pointer crosses the mic on its way to their buttons.
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HINT_DELAY);
        h.core.tick();
        assert!(!h.fake.take().iter().any(|c| c.starts_with("tell")));
        hover(&mut h, None);
        // The mic comes to a field: the words come along, to be inserted right there.
        focus(&mut h, text_field());
        let calls = h.fake.take();
        let moved = calls.iter().position(|c| c.starts_with("icon to")).expect("the mic moved");
        assert_eq!(calls[moved + 1..], ["kept hello".to_string()]);
        // Dragged elsewhere, or resized: shown at the new place.
        h.core.handle(Event::Platform(PlatformEvent::IconMoved { x: 5, y: 6 }));
        assert_eq!(h.fake.take().last().map(String::as_str), Some("kept hello"));
        h.core.handle(Event::Platform(PlatformEvent::IconZoom { steps: 1 }));
        assert_eq!(h.fake.take().last().map(String::as_str), Some("kept hello"));
    }

    #[test]
    fn copies_when_typing_fails() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.inject.lock().unwrap() = Some(Err(PlatformError::Failed("x".into())));
        h.ext(json!({"type":"session","event":{"kind":"final","text":"abc"}}));
        let calls = h.fake.take();
        assert!(calls.contains(&"copy 3".to_string()));
        assert!(calls.contains(&format!("tell {} 6s notify=true", t("native_notifyPasteManually"))));
    }

    #[test]
    fn a_mode_chosen_offline_is_sent_on_connect() {
        let mut h = Harness::new();
        h.cli(Request::SetMode { mode: InputMode::Kana });
        h.attach_host();
        h.ext(json!({"type":"hello","extensionVersion":"0.1.0"}));
        assert!(h.sent().contains(&json!({"type":"set-mode","mode":"kana"})));
    }

    #[test]
    fn the_options_page_can_change_the_config() {
        let mut h = Harness::new();
        h.connect();
        let mut cfg = NativeConfig { hotkey: Some("Ctrl+Shift+F9".into()), ..NativeConfig::default() };
        cfg.icon.visible = false;
        h.ext(json!({"type":"set-native-config","config": cfg}));
        assert_eq!(h.core.config().hotkey.as_deref(), Some("Ctrl+Shift+F9"));
        let calls = h.fake.take();
        assert!(calls.contains(&"hotkey Ctrl+Shift+F9".to_string()));
        assert!(calls.contains(&"hide icon".to_string()));
        assert_eq!(h.sent()[0]["type"], "native-config");
    }

    #[test]
    fn a_taken_hotkey_is_announced() {
        let mut h = Harness::new();
        *h.fake.hotkey.lock().unwrap() = Some(Err(PlatformError::Failed("taken".into())));
        h.core.start();
        let calls = h.fake.take();
        // The mic is up before the message, so the message can be said in its bubble.
        let icon = calls.iter().position(|c| c == "icon Idle").expect("icon shown");
        let told = calls.iter().position(|c| c.starts_with("tell ") && c.ends_with("6s notify=true")).expect("told");
        assert!(icon < told, "{calls:?}");
    }

    #[test]
    fn disconnect_resets_the_state() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        h.core.handle(Event::Closed { conn: 1 });
        assert!(!h.core.connected());
        assert!(!h.core.recording);
    }

    #[test]
    fn open_settings_opens_the_settings_page_in_its_own_chrome() {
        let mut h = Harness::new();
        assert_eq!(h.cli(Request::OpenSettings), Reply::Ok);
        let settings = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/settings";
        let expected = format!("chrome {}", chrome_launch::settings_args(Path::new("/p"), settings).join(" "));
        assert_eq!(h.fake.take(), vec![expected]);
        // The same with a page connected: nothing goes to the page.
        h.connect();
        h.cli(Request::OpenSettings);
        assert!(h.sent().is_empty());
    }

    #[test]
    fn the_hidden_page_asks_for_the_settings_when_its_taskbar_button_is_clicked() {
        let mut h = Harness::new();
        h.connect();
        h.fake.take();
        h.ext(json!({"type":"open-settings"}));
        let settings = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/settings";
        let expected = format!("chrome {}", chrome_launch::settings_args(Path::new("/p"), settings).join(" "));
        assert_eq!(h.fake.take(), vec![expected]);
    }

    #[test]
    fn report_bug_opens_the_issue_form() {
        let mut h = Harness::new();
        h.connect();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::ReportBug)));
        let calls = h.fake.take();
        assert!(
            calls[0].starts_with("open https://github.com/ishizakahiroshi/vtype/issues/new?template=bug_report.yml")
        );
        assert!(calls[0].contains("browser=Chrome%20140"));
    }

    #[test]
    fn about_opens_the_settings_page_at_about_vtype() {
        let mut h = Harness::new();
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::About)));
        let settings = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/settings#about";
        let expected = format!("chrome {}", chrome_launch::settings_args(Path::new("/p"), settings).join(" "));
        assert_eq!(h.fake.take(), vec![expected]);
    }

    #[test]
    fn the_settings_page_reads_and_switches_starting_at_sign_in() {
        let mut h = Harness::new();
        let off = Autostart { enabled: false, can_change: true };
        assert_eq!(h.cli(Request::GetAutostart), Reply::Autostart { autostart: off });
        let on = Autostart { enabled: true, can_change: true };
        assert_eq!(h.cli(Request::SetAutostart { enabled: true }), Reply::Autostart { autostart: on });
        assert!(h.fake.take().contains(&"autostart true".to_string()));
        // A failure is said as such, not as a state.
        *h.fake.autostart_fails.lock().unwrap() = true;
        let reply = h.cli(Request::SetAutostart { enabled: false });
        assert!(matches!(reply, Reply::Error { ref code, .. } if code == "autostart_failed"), "{reply:?}");
    }

    #[test]
    fn the_first_run_screen_decides_starting_at_sign_in() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"consent","autostart":true}));
        assert!(h.fake.take().contains(&"autostart true".to_string()));
        assert!(h.core.config().consented);
        // A page from before the choice existed says nothing about it: nothing is switched.
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"consent"}));
        assert!(!h.fake.take().iter().any(|c| c.starts_with("autostart")));
        assert!(h.core.config().consented);
    }

    #[test]
    fn starting_up_points_sign_in_at_this_copy() {
        let mut h = Harness::new();
        h.core.start();
        assert!(h.fake.take().contains(&"autostart here".to_string()));
    }

    #[test]
    fn the_settings_page_has_its_links_opened_in_the_usual_browser() {
        let mut h = Harness::new();
        assert_eq!(h.cli(Request::OpenLink { link: crate::about::AboutLink::Homepage }), Reply::Ok);
        assert_eq!(h.fake.take(), vec!["open https://ishizakahiroshi.com/".to_string()]);
        h.cli(Request::OpenLink { link: crate::about::AboutLink::Notices });
        let notices = format!("native-v{}/THIRD_PARTY_NOTICES.txt", env!("CARGO_PKG_VERSION"));
        assert!(h.fake.take()[0].ends_with(&notices));
    }

    #[test]
    fn diagnostics_carry_no_transcript() {
        let mut h = Harness::new();
        h.connect();
        h.ext(json!({"type":"session","event":{"kind":"final","text":"do not keep this sentence"}}));
        let text = serde_json::to_string(&h.core.diagnostics()).unwrap();
        assert!(!text.contains("do not keep"));
    }

    fn text_field() -> FieldProbe {
        FieldProbe {
            is_text_field: true,
            is_password: Some(false),
            app_id: Some("notepad.exe".into()),
            caret: Some(Rect { x: 100, y: 200, width: 1, height: 18 }),
            bounds: None,
            pointer: None,
            on_taskbar: false,
        }
    }

    fn focus(h: &mut Harness, probe: FieldProbe) {
        h.core.handle(Event::Platform(PlatformEvent::FieldChanged { probe, at: Instant::now() }));
    }

    #[test]
    fn the_beside_mic_stays_off_by_default() {
        let mut h = Harness::new();
        h.core.start();
        assert!(h.fake.take().contains(&"watch false Focus".to_string()));
        focus(&mut h, text_field());
        assert!(h.fake.take().iter().all(|c| !c.starts_with("beside")));
    }

    #[test]
    fn the_beside_mic_follows_the_focus_once_enabled() {
        let mut h = Harness::new();
        h.connect();
        let mut cfg = NativeConfig::default();
        cfg.beside_field.enabled = true;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        assert!(h.fake.take().contains(&"watch true Focus".to_string()));

        focus(&mut h, text_field());
        assert_eq!(h.fake.take(), vec![format!("beside Idle {},{}", 100 + 4, 200 - 26)]);

        // Recording turns it orange, and back.
        h.ext(json!({"type":"session","event":{"kind":"started"}}));
        assert!(h.fake.take().contains(&"beside look Recording".to_string()));
        h.ext(json!({"type":"session","event":{"kind":"ended","reason":"stopped"}}));
        assert!(h.fake.take().contains(&"beside look Idle".to_string()));

        // A password field, then something that is not a field: it goes, once.
        focus(&mut h, FieldProbe { is_password: Some(true), ..text_field() });
        assert_eq!(h.fake.take(), vec!["hide beside".to_string()]);
        focus(&mut h, FieldProbe::default());
        assert!(h.fake.take().is_empty());

        // Turning it off hides it and stops watching.
        focus(&mut h, text_field());
        h.fake.take();
        cfg.beside_field.enabled = false;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        let calls = h.fake.take();
        assert!(calls.contains(&"watch false Focus".to_string()));
        assert!(calls.contains(&"hide beside".to_string()));
    }

    /// Acts like Windows with the mic beside the field switched on: the floating mic comes to
    /// fields.
    fn following_fields() -> Harness {
        let mut h = Harness::new();
        *h.fake.icon_follows.lock().unwrap() = true;
        h.connect();
        let mut cfg = NativeConfig::default();
        cfg.beside_field.enabled = true;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        h.fake.take();
        h
    }

    fn field_with_bounds() -> FieldProbe {
        FieldProbe { bounds: Some(Rect { x: 50, y: 190, width: 400, height: 40 }), ..text_field() }
    }

    fn clicked_field() -> FieldProbe {
        FieldProbe { pointer: Some((300, 210)), ..field_with_bounds() }
    }

    fn went_home(calls: &[String]) -> bool {
        calls.iter().any(|c| c == "icon home")
    }

    #[test]
    fn on_windows_the_floating_mic_comes_to_the_field_the_focus_is_in() {
        let mut h = following_fields();
        let mut cfg = h.core.config().clone();
        let field = field_with_bounds();

        // Clicked into: by the pointer, and no small mic.
        focus(&mut h, clicked_field());
        assert_eq!(h.fake.take(), vec!["icon to Pointer(300, 210)".to_string()]);
        // The focus leaves for something that is not a field, and comes back to the same field
        // without a click before HOME_DELAY (a menu closed): the mic stays.
        focus(&mut h, FieldProbe::default());
        assert!(h.fake.take().is_empty());
        focus(&mut h, FieldProbe { pointer: Some((10, 10)), ..field.clone() });
        assert!(h.fake.take().is_empty());
        // A click somewhere else in the same field moves it again.
        focus(&mut h, FieldProbe { pointer: Some((420, 200)), ..field.clone() });
        assert_eq!(h.fake.take(), vec!["icon to Pointer(420, 200)".to_string()]);
        // Tab to another field: by its caret.
        let next = FieldProbe {
            caret: Some(Rect { x: 60, y: 300, width: 1, height: 18 }),
            bounds: Some(Rect { x: 50, y: 290, width: 400, height: 40 }),
            pointer: Some((10, 10)),
            ..text_field()
        };
        focus(&mut h, next);
        assert_eq!(
            h.fake.take(),
            vec![format!("icon to {:?}", Anchor::Caret(Rect { x: 60, y: 300, width: 1, height: 18 }))]
        );
        // A password field: the mic does not come.
        focus(&mut h, FieldProbe { is_password: Some(true), pointer: Some((300, 210)), ..field.clone() });
        assert!(h.fake.take().is_empty());

        // Hover still shows the small mic.
        cfg.beside_field.trigger = BesideFieldTrigger::Hover;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        h.fake.take();
        focus(&mut h, field);
        assert_eq!(h.fake.take(), vec![format!("beside Idle {},{}", 100 + 4, 200 - 26)]);
        // Back to focus: the small mic goes.
        cfg.beside_field.trigger = BesideFieldTrigger::Focus;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        assert!(h.fake.take().contains(&"hide beside".to_string()));
    }

    #[test]
    fn the_floating_mic_goes_back_to_the_corner_once_the_focus_is_off_the_fields_for_a_moment() {
        let mut h = following_fields();
        focus(&mut h, clicked_field());
        h.fake.take();

        // Off the fields, and into another one before HOME_DELAY (Tab over a button): it goes to
        // that field, not to the corner.
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY - Duration::from_millis(1));
        h.core.tick();
        assert!(h.fake.take().is_empty());
        let caret = Rect { x: 60, y: 300, width: 1, height: 18 };
        let next = FieldProbe {
            caret: Some(caret),
            bounds: Some(Rect { x: 50, y: 290, width: 400, height: 40 }),
            pointer: Some((10, 10)),
            ..text_field()
        };
        focus(&mut h, next);
        assert_eq!(h.fake.take(), vec![format!("icon to {:?}", Anchor::Caret(caret))]);
        h.advance(Duration::from_millis(1));
        h.core.tick();
        assert!(h.fake.take().is_empty());
        assert!(h.core.next_deadline().is_none());

        // Off the fields for HOME_DELAY: to the corner, once. The focus moving on off the fields
        // (a password field is not one) does not put it off.
        focus(&mut h, FieldProbe::default());
        assert_eq!(h.core.next_deadline(), Some(now(&h) + HOME_DELAY));
        h.advance(HOME_DELAY / 2);
        focus(&mut h, FieldProbe { is_password: Some(true), ..clicked_field() });
        h.advance(HOME_DELAY / 2);
        h.core.tick();
        assert_eq!(h.fake.take(), vec!["icon home".to_string()]);
        assert!(h.core.next_deadline().is_none());
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY);
        h.core.tick();
        assert!(h.fake.take().is_empty());
    }

    #[test]
    fn a_recording_holds_the_floating_mic_until_it_ends() {
        let mut h = following_fields();
        focus(&mut h, clicked_field());
        session(&mut h, json!({"kind":"started"}));
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY * 3);
        h.core.tick();
        assert!(!went_home(&h.fake.take()));
        session(&mut h, json!({"kind":"ended","reason":"stopped"}));
        assert!(went_home(&h.fake.take()));

        // Ended back in a field: it stays by that field.
        focus(&mut h, clicked_field());
        session(&mut h, json!({"kind":"started"}));
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY);
        focus(&mut h, clicked_field());
        session(&mut h, json!({"kind":"ended","reason":"stopped"}));
        assert!(!went_home(&h.fake.take()));
    }

    #[test]
    fn the_pointer_on_the_floating_mic_holds_it_until_it_leaves() {
        let mut h = following_fields();
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        // On the mic (or holding it) past HOME_DELAY: it stays under the hand.
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HOME_DELAY);
        h.core.tick();
        hover(&mut h, Some(MicPart::Button(MicButton::Templates)));
        assert!(!went_home(&h.fake.take()));
        hover(&mut h, None);
        assert!(went_home(&h.fake.take()));

        // Left before HOME_DELAY: it goes at HOME_DELAY, not when the pointer left.
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HOME_DELAY / 2);
        hover(&mut h, None);
        assert!(!went_home(&h.fake.take()));
        h.advance(HOME_DELAY / 2);
        h.core.tick();
        assert!(went_home(&h.fake.take()));
    }

    #[test]
    fn a_held_floating_mic_does_not_wake_the_loop_again_and_again() {
        let mut h = following_fields();
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        session(&mut h, json!({"kind":"started"}));
        h.advance(HOME_DELAY * 2);
        assert!(h.core.next_deadline().is_none_or(|at| at > now(&h)), "recording");
        session(&mut h, json!({"kind":"ended","reason":"stopped"}));

        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        hover(&mut h, Some(MicPart::Mic));
        h.advance(HOME_DELAY * 2);
        h.core.tick(); // the hint, which was due
        assert!(h.core.next_deadline().is_none_or(|at| at > now(&h)), "pointer on the mic");
    }

    #[test]
    fn after_going_back_the_same_field_brings_it_again_without_a_click() {
        let mut h = following_fields();
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY);
        h.core.tick();
        h.fake.take();
        // Alt+Tab back to the window: the focus is in the same field, the pointer elsewhere.
        focus(&mut h, FieldProbe { pointer: Some((10, 10)), ..field_with_bounds() });
        assert_eq!(
            h.fake.take(),
            vec![format!("icon to {:?}", Anchor::Caret(Rect { x: 100, y: 200, width: 1, height: 18 }))]
        );
    }

    #[test]
    fn a_floating_mic_the_focus_did_not_bring_stays_where_it_is() {
        // Just started: the mic is where it was put, not at a field.
        let mut h = following_fields();
        focus(&mut h, FieldProbe::default());
        assert!(h.core.next_deadline().is_none());
        h.advance(HOME_DELAY);
        h.core.tick();
        assert!(h.fake.take().is_empty());

        // Switched to hover on the way: the small mic's rules, and nothing left to go back.
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        let mut cfg = h.core.config().clone();
        cfg.beside_field.trigger = BesideFieldTrigger::Hover;
        h.ext(json!({"type": "set-native-config", "config": cfg}));
        focus(&mut h, clicked_field());
        focus(&mut h, FieldProbe::default());
        h.advance(HOME_DELAY);
        h.core.tick();
        assert!(!went_home(&h.fake.take()));
        assert!(h.core.next_deadline().is_none());
    }

    fn check_fields(h: &mut Harness) -> Vec<String> {
        h.core.handle(Event::Platform(PlatformEvent::Menu(MenuAction::CheckFields)));
        h.fake.take()
    }

    fn said(calls: &[String], text: &str) -> bool {
        calls.iter().any(|c| c.starts_with(&format!("tell {text} ")))
    }

    #[test]
    fn looking_at_an_app_needs_the_beside_mic_on_and_an_app_to_look_at() {
        // Off (the default): nothing reports fields, so there is nothing to watch.
        let mut h = Harness::new();
        h.connect();
        let calls = check_fields(&mut h);
        assert!(said(&calls, &t("native_bubbleCheckFieldsOff")), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("check fields")), "{calls:?}");
        // On, but no app has had the focus since.
        let mut h = following_fields();
        let calls = check_fields(&mut h);
        assert!(said(&calls, &t("native_bubbleCheckFieldsNoApp")), "{calls:?}");
        assert!(!calls.iter().any(|c| c.starts_with("check fields")), "{calls:?}");
    }

    #[test]
    fn the_app_looked_at_is_the_one_the_user_was_in_not_the_taskbar() {
        let mut h = following_fields();
        focus(&mut h, FieldProbe { app_id: Some("line.exe".into()), ..FieldProbe::default() });
        // The tray was clicked: the taskbar took the focus (vtype's own menu is not reported).
        focus(&mut h, FieldProbe { app_id: Some("explorer.exe".into()), on_taskbar: true, ..FieldProbe::default() });
        // The pointer leaving a field (hover) names no app.
        focus(&mut h, FieldProbe::default());
        h.fake.take();
        let start = t_with("native_bubbleCheckFieldsStart", &[("seconds", "10"), ("app", "line.exe")]);
        assert_eq!(
            check_fields(&mut h),
            vec!["check fields line.exe".to_string(), format!("tell {start} 12s notify=true")]
        );
        // File Explorer itself is an app like any other.
        focus(&mut h, FieldProbe { app_id: Some("explorer.exe".into()), ..FieldProbe::default() });
        assert!(check_fields(&mut h).contains(&"check fields explorer.exe".to_string()));
    }

    #[test]
    fn what_was_found_is_said_and_kept_for_the_diagnostic_info() {
        let mut h = following_fields();
        assert_eq!(h.core.diagnostics()["lastFieldCheck"], Value::Null);
        h.core.handle(Event::Platform(PlatformEvent::FieldsChecked(field_check::tests::line())));
        let text = field_check::message(field_check::Verdict::AfterWindow, "line.exe");
        assert_eq!(h.fake.take(), vec![format!("tell {text} 12s notify=true")]);
        let check = &h.core.diagnostics()["lastFieldCheck"];
        assert_eq!(check["appId"], "line.exe");
        assert_eq!(check["verdict"], "after_window");
        assert_eq!(check["seen"][3]["path"], "recheck");
        assert_eq!(check["seen"][3]["class"], "AutoSuggestTextArea");
    }

    #[test]
    fn diagnostics_carry_no_text_from_the_fields_looked_at() {
        // A look at an app's fields keeps only these facts about each element: there is no room
        // for its name or value, which can hold a chat's text.
        let mut h = following_fields();
        h.ext(json!({"type":"session","event":{"kind":"final","text":"do not keep this sentence"}}));
        h.core.handle(Event::Platform(PlatformEvent::FieldsChecked(field_check::tests::line())));
        let report = h.core.diagnostics();
        assert!(!serde_json::to_string(&report).unwrap().contains("do not keep"));
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let check = &report["lastFieldCheck"];
        assert_eq!(keys(check), ["appId", "at", "seen", "verdict"]);
        let seen = check["seen"].as_array().unwrap();
        assert!(!seen.is_empty());
        for s in seen {
            assert_eq!(
                keys(s),
                [
                    "afterMs",
                    "bounds",
                    "class",
                    "hasText",
                    "hasValue",
                    "keyboardFocusable",
                    "kind",
                    "path",
                    "readOnly",
                    "takesText"
                ]
            );
            assert_eq!(keys(&s["bounds"]), ["height", "width", "x", "y"]);
        }
    }
}
