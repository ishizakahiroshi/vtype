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
use crate::config::{self, NativeConfig};
use crate::diag::{self, ErrorLog};
use crate::i18n::{t, t_with};
use crate::ipc;
use crate::platform::{
    FieldProbe, IconState, InjectOutcome, MenuAction, Platform, PlatformError, PlatformEvent, TrayState, MESSAGE_HOLD,
};
use crate::protocol::{FromExtension, InputMode, Reply, Request, SessionEvent, ToExtension};
use crate::report::{self, ReportInfo, Surface};

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
    /// The mic beside the field is up (child plan C8).
    beside_shown: bool,
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
            beside_shown: false,
            speech_page: None,
            profile_dir: chrome_launch::profile_dir(),
            launched: None,
            needs_setup: false,
            page_ready: false,
            keep_visible: false,
            grant_in_profile: false,
            last_start_mode: None,
            relaunch: None,
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
        if self.config.icon.visible {
            self.platform.show_icon(IconState::Idle, self.icon_position());
        }
        self.register_hotkey();
        self.platform.watch_fields(&self.config.beside_field);
        self.update_tray();
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
        pending.into_iter().chain(relaunch).min()
    }

    pub fn tick(&mut self) {
        let now = (self.now)();
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
        let Some(page) = self.speech_page.as_deref() else {
            return Reply::error("speech_page_unavailable", "the settings page is not being served");
        };
        let args = chrome_launch::settings_args(&self.profile_dir, &chrome_launch::settings_url(page));
        match self.platform.launch_chrome(&args) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::error("chrome_launch_failed", e.to_string()),
        }
    }

    pub fn diagnostics(&self) -> Value {
        let os = self.platform.os_description();
        let host = self.host.as_ref();
        let notes = self.platform.platform_notes();
        diag::report(&diag::Snapshot {
            os: &os,
            config: &self.config,
            connected: self.connected(),
            extension_version: host.and_then(|h| h.extension_version.as_deref()),
            browser: host.and_then(|h| h.browser.as_deref()),
            errors: &self.errors,
            notes: &notes,
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
            FromExtension::Consent => {
                tracing::info!("the user agreed on the speech page");
                if !self.config.consented {
                    self.config.consented = true;
                    self.save_config();
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
        self.platform.hide_bubble();
        self.show_icon(IconState::Idle);
    }

    fn session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Started => {
                self.recording = true;
                self.last_char = None;
                self.show_icon(IconState::Recording);
                self.update_tray();
            }
            SessionEvent::Interim { text } => {
                if self.config.icon.visible {
                    self.platform.show_bubble(&text);
                }
            }
            SessionEvent::Final { text } => {
                self.platform.hide_bubble();
                self.insert(&text);
            }
            SessionEvent::Ended { reason, code } => {
                tracing::info!(reason = %reason, code = ?code, "session ended");
                if let Some(code) = code {
                    self.errors.push(code);
                }
                self.end_recording_ui();
                self.update_tray();
            }
        }
    }

    /// Puts recognized text into the foreground app. Never into a password field.
    fn insert(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        // Chrome ends a recognition after each utterance, so one recording yields several finals.
        // English words would run together ("helloworld"); Japanese needs no space.
        let text = match (self.last_char, text.chars().next()) {
            (Some(a), Some(b)) if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric() => {
                format!(" {text}")
            }
            _ => text.to_string(),
        };
        let text = text.as_str();
        // End-to-end checks on the developer's machine (standalone plan C4): nothing is typed into
        // whatever app happens to be in front. Not for users; not documented.
        if std::env::var_os("VTYPE_TEST_NO_INJECT").is_some() {
            tracing::info!("test: final len={}", text.chars().count());
            self.last_char = text.chars().last();
            return;
        }
        let field = self.platform.focused_field();
        if field.is_password == Some(true) {
            tracing::info!(len = text.chars().count(), "password field in front; not inserting");
            self.platform.tell(&t("native_notifyPasswordField"), MESSAGE_HOLD, true);
            return;
        }
        match self.platform.inject_text(text, self.config.inject) {
            Ok(InjectOutcome::Typed) | Ok(InjectOutcome::Pasted) => {
                tracing::info!(len = text.chars().count(), "inserted");
                self.last_char = text.chars().last();
                self.show_icon(IconState::Done);
            }
            Ok(InjectOutcome::CopiedOnly) => {
                self.platform.tell(&t("native_notifyPasteManually"), MESSAGE_HOLD, true);
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not insert; copying instead");
                self.errors.push("inject_failed");
                if self.platform.copy_to_clipboard(text).is_ok() {
                    self.platform.tell(&t("native_notifyPasteManually"), MESSAGE_HOLD, true);
                }
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
        let beside_changed = new.beside_field != self.config.beside_field;
        self.config = new;
        if beside_changed {
            self.platform.watch_fields(&self.config.beside_field);
            if !self.config.beside_field.enabled && self.beside_shown {
                self.beside_shown = false;
                self.platform.hide_beside();
            }
        }
        self.save_config();
        if hotkey_changed {
            self.register_hotkey();
        }
        if icon_changed {
            if self.config.icon.visible {
                self.show_icon(if self.recording { IconState::Recording } else { IconState::Idle });
            } else {
                self.platform.hide_icon();
            }
        }
        if mode_changed {
            self.mode = self.config.input_mode;
            self.send(&ToExtension::SetMode { mode: self.mode });
        }
        self.update_tray();
    }

    /// The mic beside the field follows the focus (or the pointer): shown, moved or hidden.
    fn field_changed(&mut self, probe: &FieldProbe, at: Instant) {
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

    fn handle_platform(&mut self, ev: PlatformEvent) -> Flow {
        match ev {
            PlatformEvent::ToggleRequested => {
                let _ = self.toggle(None);
            }
            PlatformEvent::FieldChanged { probe, at } => self.field_changed(&probe, at),
            PlatformEvent::IconMoved { x, y } => {
                self.config.icon.x = Some(x);
                self.config.icon.y = Some(y);
                self.save_config();
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
    use crate::platform::{FieldInfo, Rect};
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
        fn show_icon(&self, state: IconState, _position: Option<(i32, i32)>) {
            self.log(format!("icon {state:?}"));
        }
        fn hide_icon(&self) {
            self.log("hide icon".into());
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
        fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
            self.log(format!("autostart {enabled}"));
            Ok(())
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
            FieldInfo { is_password: Some(true), caret_rect: Some(Rect::default()), app_id: None };
        h.ext(json!({"type":"session","event":{"kind":"final","text":"secret words"}}));
        let calls = h.fake.take();
        assert!(!calls.iter().any(|c| c.starts_with("inject")));
        assert!(calls.contains(&format!("tell {} 6s notify=true", t("native_notifyPasswordField"))));
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
}
