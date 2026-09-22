//! The resident process. It owns the tray and the floating mic, listens on the IPC endpoint for
//! the command line and for the Native Messaging host, and turns what the extension recognizes
//! into text in the foreground app.
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
use crate::config::{self, NativeConfig};
use crate::diag::{self, ErrorLog};
use crate::i18n::{t, t_with};
use crate::ipc;
use crate::platform::{
    FieldProbe, IconState, InjectOutcome, MenuAction, Platform, PlatformError, PlatformEvent, TrayState,
};
use crate::protocol::{FromExtension, InputMode, Reply, Request, SessionEvent, ToExtension};
use crate::report::{self, ReportInfo, Surface};

/// How long a start request waits for Chrome to come up and connect (parent plan D13).
pub const CHROME_WAIT: Duration = Duration::from_secs(10);

/// The store build of the extension (parent plan D10).
pub const STORE_EXTENSION_ID: &str = "nngfilimeplngdjdmgkddlhbdjpmikgn";

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
    origin: String,
    /// Set once the extension said hello through this host.
    extension_version: Option<String>,
    browser: Option<String>,
}

struct Pending {
    start_mode: Option<InputMode>,
    deadline: Instant,
}

pub struct Core {
    platform: Arc<dyn Platform>,
    config: NativeConfig,
    config_path: Option<PathBuf>,
    host: Option<HostLink>,
    mode: InputMode,
    /// A mode chosen while no extension was connected, sent once one says hello.
    pending_mode: Option<InputMode>,
    recording: bool,
    pending: Option<Pending>,
    errors: ErrorLog,
    now: Box<dyn Fn() -> Instant + Send>,
    /// Last character typed in this recording (to space English finals apart).
    last_char: Option<char>,
    /// The mic beside the field is up (child plan C8).
    beside_shown: bool,
}

impl Core {
    pub fn new(platform: Arc<dyn Platform>, config: NativeConfig, config_path: Option<PathBuf>) -> Core {
        Core {
            platform,
            config,
            config_path,
            host: None,
            mode: InputMode::Normal,
            pending_mode: None,
            recording: false,
            pending: None,
            errors: ErrorLog::default(),
            now: Box::new(Instant::now),
            last_char: None,
            beside_shown: false,
        }
    }

    /// Replaces the clock, for tests.
    #[cfg(test)]
    pub fn set_clock(&mut self, now: Box<dyn Fn() -> Instant + Send>) {
        self.now = now;
    }

    pub fn connected(&self) -> bool {
        self.host.as_ref().is_some_and(|h| h.extension_version.is_some())
    }

    #[cfg(test)]
    pub fn config(&self) -> &NativeConfig {
        &self.config
    }

    /// Everything that happens once, when the daemon comes up.
    pub fn start(&mut self) {
        self.register_hotkey();
        self.platform.watch_fields(&self.config.beside_field);
        if self.config.icon.visible {
            self.platform.show_icon(IconState::Idle, self.icon_position());
        }
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
                self.platform.notify("vtype", &t_with("native_notifyHotkeyFailed", &[("hotkey", &spec)]));
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

    fn send(&self, msg: &ToExtension) -> bool {
        let Some(host) = &self.host else { return false };
        match serde_json::to_value(msg) {
            Ok(message) => host.out.send(Reply::ToExtension { message }).is_ok(),
            Err(_) => false,
        }
    }

    /// When the loop should wake up even if nothing arrives.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.deadline)
    }

    pub fn tick(&mut self) {
        if let Some(p) = &self.pending {
            if (self.now)() >= p.deadline {
                self.pending = None;
                tracing::warn!("Chrome did not connect in time");
                self.errors.push("chrome_not_connected");
                self.platform.notify("vtype", &t("native_notifyChromeNotConnected"));
            }
        }
    }

    pub fn handle(&mut self, event: Event) -> Flow {
        match event {
            Event::Request { conn, req, out } => self.handle_request(conn, req, out),
            Event::Closed { conn } => {
                if self.host.as_ref().is_some_and(|h| h.conn == conn) {
                    tracing::info!("extension disconnected");
                    self.host = None;
                    self.end_recording_ui();
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
            Request::Quit => {
                let _ = out.send(Reply::Ok);
                return Flow::Quit;
            }
            Request::HostHello { origin } => {
                tracing::info!("host attached");
                self.host = Some(HostLink { conn, out: out.clone(), origin, extension_version: None, browser: None });
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
        if self.connected() {
            self.send(&ToExtension::Start { mode });
            return Reply::Ok;
        }
        let first = self.pending.is_none();
        self.pending = Some(Pending { start_mode: mode, deadline: (self.now)() + CHROME_WAIT });
        if first {
            tracing::info!("not connected; starting Chrome");
            if let Err(e) = self.platform.launch_chrome(&["--no-startup-window".to_string()]) {
                tracing::warn!(error = %e, "could not start Chrome");
                self.errors.push("chrome_launch_failed");
            }
        }
        Reply::Ok
    }

    fn stop_recording(&mut self) -> Reply {
        self.pending = None;
        if self.send(&ToExtension::Stop) {
            Reply::Ok
        } else {
            Reply::error("not_connected", "the vtype extension is not connected")
        }
    }

    fn set_mode(&mut self, mode: InputMode) -> Reply {
        self.mode = mode;
        if self.connected() {
            self.send(&ToExtension::SetMode { mode });
        } else {
            self.pending_mode = Some(mode);
        }
        self.update_tray();
        Reply::Ok
    }

    fn extension_id(&self) -> String {
        self.host
            .as_ref()
            .and_then(|h| extension_id_from_origin(&h.origin))
            .unwrap_or_else(|| STORE_EXTENSION_ID.to_string())
    }

    fn open_settings(&mut self) -> Reply {
        if self.connected() {
            self.send(&ToExtension::OpenOptions);
            return Reply::Ok;
        }
        let url = format!("chrome-extension://{}/options.html", self.extension_id());
        match self.platform.launch_chrome(&[url]) {
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
                self.send(&ToExtension::NativeConfig { config: self.config.clone() });
                if let Some(mode) = self.pending_mode.take() {
                    self.send(&ToExtension::SetMode { mode });
                }
                self.send(&ToExtension::GetState);
                if let Some(p) = self.pending.take() {
                    self.send(&ToExtension::Start { mode: p.start_mode });
                }
                self.update_tray();
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
        let field = self.platform.focused_field();
        if field.is_password == Some(true) {
            tracing::info!(len = text.chars().count(), "password field in front; not inserting");
            self.platform.notify("vtype", &t("native_notifyPasswordField"));
            return;
        }
        match self.platform.inject_text(text, self.config.inject) {
            Ok(InjectOutcome::Typed) | Ok(InjectOutcome::Pasted) => {
                tracing::info!(len = text.chars().count(), "inserted");
                self.last_char = text.chars().last();
                self.show_icon(IconState::Done);
            }
            Ok(InjectOutcome::CopiedOnly) => {
                self.platform.notify("vtype", &t("native_notifyPasteManually"));
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not insert; copying instead");
                self.errors.push("inject_failed");
                if self.platform.copy_to_clipboard(text).is_ok() {
                    self.platform.notify("vtype", &t("native_notifyPasteManually"));
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

/// `chrome-extension://<id>/` to `<id>`.
pub fn extension_id_from_origin(origin: &str) -> Option<String> {
    let id = origin.strip_prefix("chrome-extension://")?.trim_end_matches('/');
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase())).then(|| id.to_string())
}

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

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
    use std::sync::Mutex;

    /// Records every call; `inject` and `field` decide what the OS "does".
    #[derive(Default)]
    pub struct FakePlatform {
        pub calls: Mutex<Vec<String>>,
        pub field: Mutex<FieldInfo>,
        pub inject: Mutex<Option<Result<InjectOutcome, PlatformError>>>,
        pub hotkey: Mutex<Option<Result<(), PlatformError>>>,
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
        fn notify(&self, _title: &str, body: &str) {
            self.log(format!("notify {body}"));
        }
        fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
            self.log(format!("autostart {enabled}"));
            Ok(())
        }
        fn launch_chrome(&self, args: &[String]) -> Result<(), PlatformError> {
            self.log(format!("chrome {}", args.join(" ")));
            Ok(())
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

    #[test]
    fn toggle_while_disconnected_launches_chrome_and_starts_once_it_connects() {
        let mut h = Harness::new();
        assert_eq!(h.cli(Request::Toggle { mode: Some(InputMode::En) }), Reply::Ok);
        assert!(h.fake.take().contains(&"chrome --no-startup-window".to_string()));
        // A second press while waiting does not start Chrome again.
        h.cli(Request::Toggle { mode: Some(InputMode::En) });
        assert!(!h.fake.take().iter().any(|c| c.starts_with("chrome")));

        h.attach_host();
        h.ext(json!({"type":"hello","extensionVersion":"0.1.0"}));
        let sent = h.sent();
        assert_eq!(sent[0]["type"], "hello");
        assert_eq!(sent.last().unwrap(), &json!({"type":"start","mode":"en"}));
        assert!(h.core.next_deadline().is_none());
    }

    #[test]
    fn gives_up_after_ten_seconds_and_says_so() {
        let mut h = Harness::new();
        h.cli(Request::Start { mode: None });
        h.fake.take();
        h.advance(Duration::from_secs(9));
        h.core.tick();
        assert!(h.fake.take().is_empty());
        h.advance(Duration::from_secs(2));
        h.core.tick();
        let calls = h.fake.take();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("notify "));
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
        assert!(calls.iter().any(|c| c.starts_with("notify")));
    }

    #[test]
    fn copies_when_typing_fails() {
        let mut h = Harness::new();
        h.connect();
        *h.fake.inject.lock().unwrap() = Some(Err(PlatformError::Failed("x".into())));
        h.ext(json!({"type":"session","event":{"kind":"final","text":"abc"}}));
        let calls = h.fake.take();
        assert!(calls.contains(&"copy 3".to_string()));
        assert!(calls.iter().any(|c| c.starts_with("notify")));
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
        assert!(h.fake.take().iter().any(|c| c.starts_with("notify ")));
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
    fn open_settings_uses_the_connected_extension_or_starts_chrome() {
        let mut h = Harness::new();
        h.cli(Request::OpenSettings);
        assert_eq!(h.fake.take(), vec![format!("chrome chrome-extension://{STORE_EXTENSION_ID}/options.html")]);
        h.connect();
        h.cli(Request::OpenSettings);
        assert_eq!(h.sent(), vec![json!({"type":"open-options"})]);
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

    #[test]
    fn origin_to_id() {
        assert_eq!(
            extension_id_from_origin("chrome-extension://nngfilimeplngdjdmgkddlhbdjpmikgn/").as_deref(),
            Some(STORE_EXTENSION_ID)
        );
        assert_eq!(extension_id_from_origin("https://example.com/"), None);
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
