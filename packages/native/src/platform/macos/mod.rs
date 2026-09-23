//! macOS (child plan C6): the menu bar item, the global shortcut, typing with CGEvent, the
//! Accessibility API for the focused field, and the floating mic in a non-activating panel. The UI
//! parts live on the main thread (`ui`), which AppKit requires; the rest is called directly from
//! the daemon's worker thread.
//!
//! There is no Mac to try this on: CI builds, lints and tests it, and the parent plan's
//! checklist lists what only a Mac can show.

mod ax;
mod beside;
mod fullscreen;
mod inject;
mod overlay;
mod system;
mod template_menu;
mod ui;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::desktop::BesideControl;
use super::{
    FieldInfo, IconState, InjectOutcome, Platform, PlatformError, PlatformEvent, TrayState, MESSAGE_HOLD, MESSAGE_LINES,
};
use crate::config::{BesideFieldConfig, InjectMethod};
use crate::i18n::t;

pub struct MacPlatform {
    shared: Arc<ui::Shared>,
    beside: Mutex<BesideControl<beside::Watcher>>,
}

impl MacPlatform {
    pub fn new() -> Self {
        MacPlatform {
            shared: Arc::new(ui::Shared::default()),
            beside: Mutex::new(BesideControl::new(beside::Watcher::start)),
        }
    }

    fn beside(&self) -> std::sync::MutexGuard<'_, BesideControl<beside::Watcher>> {
        self.beside.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Platform for MacPlatform {
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
            .unwrap_or_else(|| Err(PlatformError::Failed("the main thread did not answer".into())))
    }

    fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
        if !ax::trusted(false) {
            if system::first_accessibility_notice() {
                self.tell(&t("native_notifyAccessibility"), MESSAGE_HOLD, true);
            }
            return Err(PlatformError::Failed("vtype is not allowed to use Accessibility".into()));
        }
        inject::inject(text, method)
    }

    fn press_keys(&self, keys: super::EditKeys) -> Result<(), PlatformError> {
        if !ax::trusted(false) {
            return Err(PlatformError::Failed("vtype is not allowed to use Accessibility".into()));
        }
        inject::press_keys(keys)
    }

    fn copy_selection(&self) -> Result<Option<String>, PlatformError> {
        if !ax::trusted(false) {
            return Err(PlatformError::Failed("vtype is not allowed to use Accessibility".into()));
        }
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
        ax::focused_field()
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
                // Off the main thread: showing a notification may take a moment.
                std::thread::spawn(move || system::notify("vtype", &text));
            }
        });
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        system::set_autostart(enabled)
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
}
