//! macOS. Filled in by the macOS child plan (C6); until then every UI call is a no-op.

use std::sync::mpsc::Sender;

use super::{FieldInfo, IconState, IdleLoop, InjectOutcome, Platform, PlatformError, PlatformEvent, TrayState};
use crate::config::InjectMethod;

pub struct MacPlatform {
    idle: IdleLoop,
}

impl MacPlatform {
    pub fn new() -> Self {
        MacPlatform { idle: IdleLoop::default() }
    }
}

impl Platform for MacPlatform {
    fn run_event_loop(&self, _events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
        self.idle.run();
        Ok(())
    }
    fn quit(&self) {
        self.idle.quit();
    }
    fn set_tray(&self, _state: &TrayState) {}
    fn register_hotkey(&self, _spec: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn inject_text(&self, _text: &str, _method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn focused_field(&self) -> FieldInfo {
        FieldInfo::default()
    }
    fn show_icon(&self, _state: IconState, _position: Option<(i32, i32)>) {}
    fn hide_icon(&self) {}
    fn show_bubble(&self, _text: &str) {}
    fn hide_bubble(&self) {}
    fn notify(&self, _title: &str, _body: &str) {}
    fn set_autostart(&self, _enabled: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn launch_chrome(&self, _args: &[String]) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn open_url(&self, _url: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn copy_to_clipboard(&self, _text: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn os_description(&self) -> String {
        "macOS".to_string()
    }
    fn ui_language(&self) -> String {
        super::language_from_env()
    }
}
