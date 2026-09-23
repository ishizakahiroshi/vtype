//! Windows (child plan C5): tray, global shortcut, typing with SendInput, UI Automation for the
//! focused field, and the floating mic. The UI parts live on one thread (`ui`); the rest is
//! called directly from the daemon's worker thread.

mod beside;
mod inject;
mod overlay;
mod startup_task;
mod system;
mod template_menu;
mod ui;
mod uia;

pub mod debug;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::desktop::BesideControl;
use super::{FieldInfo, IconState, InjectOutcome, Platform, PlatformError, PlatformEvent, TrayState, MESSAGE_LINES};
use crate::config::{BesideFieldConfig, InjectMethod};

pub struct WindowsPlatform {
    shared: Arc<ui::Shared>,
    beside: Mutex<BesideControl<beside::Watcher>>,
}

impl WindowsPlatform {
    pub fn new() -> Self {
        WindowsPlatform {
            shared: Arc::new(ui::Shared::default()),
            beside: Mutex::new(BesideControl::new(beside::Watcher::start)),
        }
    }

    fn beside(&self) -> std::sync::MutexGuard<'_, BesideControl<beside::Watcher>> {
        self.beside.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The executable name of a process, e.g. `chrome.exe`.
pub(crate) fn system_process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(|s| s.to_ascii_lowercase())
    }
}

impl Platform for WindowsPlatform {
    fn run_event_loop(&self, events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
        self.beside().set_events(events.clone());
        let result = ui::run(self.shared.clone(), events);
        self.beside().stop();
        result
    }

    fn quit(&self) {
        self.shared.run(|_| ui::quit());
    }

    fn set_tray(&self, state: &TrayState) {
        let state = state.clone();
        self.shared.run(move |ui| ui.set_tray(state));
    }

    fn register_hotkey(&self, spec: &str) -> Result<(), PlatformError> {
        let parsed = crate::hotkey::parse(spec).map_err(PlatformError::Failed)?;
        self.shared
            .call(move |ui| ui.register_hotkey(&parsed))
            .unwrap_or_else(|| Err(PlatformError::Failed("the UI thread did not answer".into())))
    }

    fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
        inject::inject(text, method)
    }

    fn press_keys(&self, keys: super::EditKeys) -> Result<(), PlatformError> {
        inject::press_keys(keys)
    }

    fn copy_selection(&self) -> Result<Option<String>, PlatformError> {
        super::desktop::copy_selection_with(inject::press_copy)
    }

    fn show_templates(&self, list: &crate::menu::TemplateList) {
        let list = list.clone();
        self.shared.run(move |ui| ui.show_templates(list));
    }

    fn refresh_templates(&self, list: &crate::menu::TemplateList) {
        let list = list.clone();
        self.shared.run(move |ui| ui.refresh_templates(list));
    }

    fn focused_field(&self) -> FieldInfo {
        uia::focused_field()
    }

    fn show_icon(&self, state: IconState, position: Option<(i32, i32)>) {
        self.shared.run(move |ui| ui.show_icon(state, position));
    }

    fn hide_icon(&self) {
        self.shared.run(|ui| ui.hide_icon());
    }

    fn set_icon_scale(&self, percent: u16) {
        self.shared.run(move |ui| ui.set_icon_scale(percent));
    }

    fn voice_cue(&self, cue: super::VoiceCue) {
        self.shared.run(move |ui| ui.voice_cue(cue));
    }

    fn show_bubble(&self, text: &str) {
        let text = text.to_string();
        self.shared.run(move |ui| ui.show_bubble(&text));
    }

    fn hide_bubble(&self) {
        self.shared.run(|ui| ui.hide_bubble());
    }

    fn tell(&self, text: &str, hold: Duration, or_notify: bool) {
        let text = text.to_string();
        self.shared.run(move |ui| {
            if !ui.show_bubble_for(&text, hold, MESSAGE_LINES) && or_notify {
                // Off the UI thread: showing a notification may take a moment.
                std::thread::spawn(move || system::notify("vtype", &text));
            }
        });
    }

    fn keeps_words(&self) -> bool {
        true
    }

    fn show_kept(&self, text: Option<&str>) {
        let text = text.map(str::to_string);
        self.shared.run(move |ui| ui.show_kept(text));
    }

    fn autostart(&self) -> Result<super::Autostart, PlatformError> {
        system::autostart()
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        system::set_autostart(enabled)
    }

    fn autostart_here(&self) {
        system::autostart_here()
    }

    fn launch_chrome(&self, args: &[String]) -> Result<(), PlatformError> {
        system::launch_chrome(args)
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        system::open_url(url)
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), PlatformError> {
        system::copy_to_clipboard(text)
    }

    fn os_description(&self) -> String {
        system::os_description()
    }

    fn ui_language(&self) -> String {
        system::ui_language()
    }

    fn platform_notes(&self) -> Vec<(String, String)> {
        self.shared.beside_timing().map(|t| vec![("beside_field_timing".to_string(), t)]).unwrap_or_default()
    }

    fn watch_fields(&self, config: &BesideFieldConfig) {
        self.beside().set_config(config);
    }

    fn show_beside(&self, pos: (i32, i32), look: IconState, reported_at: Instant) {
        self.shared.run(move |ui| ui.show_beside(pos, look, reported_at));
    }

    fn hide_beside(&self) {
        self.shared.run(|ui| ui.hide_beside());
    }

    fn set_beside_look(&self, look: IconState) {
        self.shared.run(move |ui| ui.set_beside_look(look));
    }

    fn icon_follows_fields(&self) -> bool {
        true
    }

    fn icon_to_field(&self, anchor: crate::platform::Anchor, reported_at: Instant) {
        self.shared.run(move |ui| ui.icon_to_field(anchor, reported_at));
    }

    fn icon_home(&self) {
        self.shared.run(|ui| ui.icon_home());
    }

    fn check_fields(&self, app_id: &str) {
        match self.beside().watcher_mut() {
            Some(watcher) => watcher.check(app_id.to_string()),
            None => tracing::warn!("field check: the fields are not being watched"),
        }
    }
}

/// A daemon started from the Run key or by double-click gets a console window of its own
/// (vtype is a console program, so that `vtype status` prints). When this process is the only one
/// on its console, start a detached copy without a window and let this one end.
/// Returns true when the caller should exit.
pub fn leave_own_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut ids = [0u32; 2];
    let count = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    if count != 1 {
        return false;
    }
    match crate::ipc::spawn_daemon() {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "could not restart without a console; keeping this one");
            false
        }
    }
}
