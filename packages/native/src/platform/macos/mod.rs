//! macOS (child plan C6): the menu bar item, the global shortcut, typing with CGEvent, the
//! Accessibility API for the focused field, and the floating mic in a non-activating panel. The UI
//! parts live on the main thread (`ui`), which AppKit requires; the rest is called directly from
//! the daemon's worker thread.
//!
//! There is no Mac to try this on: CI builds, lints and tests it, and the parent plan's
//! checklist lists what only a Mac can show.

mod ax;
mod fullscreen;
mod inject;
mod overlay;
mod system;
mod ui;

use std::sync::mpsc::Sender;
use std::sync::Arc;

use super::{FieldInfo, IconState, InjectOutcome, Platform, PlatformError, PlatformEvent, TrayState};
use crate::config::InjectMethod;

pub struct MacPlatform {
    shared: Arc<ui::Shared>,
}

impl MacPlatform {
    pub fn new() -> Self {
        MacPlatform { shared: Arc::new(ui::Shared::default()) }
    }
}

impl Platform for MacPlatform {
    fn run_event_loop(&self, events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
        ui::run(self.shared.clone(), events)
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
            system::notify_accessibility_once();
            return Err(PlatformError::Failed("vtype is not allowed to use Accessibility".into()));
        }
        inject::inject(text, method)
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

    fn show_bubble(&self, text: &str) {
        let text = text.to_string();
        self.shared.run(move |ui| ui.show_bubble(&text));
    }

    fn hide_bubble(&self) {
        self.shared.run(|ui| ui.hide_bubble());
    }

    fn notify(&self, title: &str, body: &str) {
        system::notify(title, body);
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
}
