//! Linux (child plan C7): the tray (AppIndicator, on GTK's loop), the global shortcut (X11 only;
//! GNOME on Wayland gets a custom shortcut from `vtype install`), typing (XTest on X11, the
//! three-step fallback on Wayland), AT-SPI for password fields, and the floating mic on X11.
//!
//! There is no Linux desktop to try this on: CI builds, lints and tests it, and the parent plan's
//! checklist lists what only a Linux desktop can show.

mod atspi_focus;
pub mod gnome;
mod overlay;
mod system;
mod ui;
mod wayland;
mod x11;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{FieldInfo, IconState, InjectOutcome, Platform, PlatformError, PlatformEvent, TrayState, MESSAGE_LINES};
use crate::config::InjectMethod;
use crate::linux_setup::{current_session, Session};

pub struct LinuxPlatform {
    shared: Arc<ui::Shared>,
    session: Session,
    focus: atspi_focus::Tracker,
    notes: Mutex<Vec<(String, String)>>,
}

impl LinuxPlatform {
    pub fn new() -> Self {
        LinuxPlatform {
            shared: Arc::new(ui::Shared::default()),
            session: current_session(),
            focus: atspi_focus::Tracker::default(),
            notes: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, name: &str, value: String) {
        let mut notes = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        notes.retain(|(k, _)| k != name);
        notes.push((name.to_string(), value));
    }
}

impl Platform for LinuxPlatform {
    fn run_event_loop(&self, events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
        self.note("session", format!("{:?}", self.session));
        self.focus.start();
        ui::run(self.shared.clone(), events, self.session)
    }

    fn quit(&self) {
        self.shared.run(|_| ui::quit());
    }

    fn set_tray(&self, state: &TrayState) {
        let state = state.clone();
        self.shared.run(move |ui| ui.set_tray(state));
    }

    fn register_hotkey(&self, spec: &str) -> Result<(), PlatformError> {
        // Wayland does not let an app grab keys; GNOME gets a custom shortcut at install instead.
        if self.session != Session::X11 {
            return Err(PlatformError::Unsupported);
        }
        let parsed = crate::hotkey::parse(spec).map_err(PlatformError::Failed)?;
        self.shared
            .call(move |ui| ui.register_hotkey(&parsed))
            .unwrap_or_else(|| Err(PlatformError::Failed("the UI thread did not answer".into())))
    }

    fn inject_text(&self, text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
        match self.session {
            Session::X11 => x11::inject(text, method),
            Session::Wayland => {
                let result = crate::wayland_inject::inject_wayland(text, &mut wayland::Routes);
                self.note("wayland_inject", result.trail);
                result.outcome
            }
            Session::Unknown => Err(PlatformError::Unsupported),
        }
    }

    fn press_keys(&self, keys: super::EditKeys) -> Result<(), PlatformError> {
        // Wayland does not let an app press keys in another one (typing there has its own fallback).
        match self.session {
            Session::X11 => x11::press_keys(keys),
            _ => Err(PlatformError::Unsupported),
        }
    }

    fn copy_selection(&self) -> Result<Option<String>, PlatformError> {
        match self.session {
            Session::X11 => super::desktop::copy_selection_with(x11::press_copy),
            _ => Err(PlatformError::Unsupported),
        }
    }

    fn show_templates(&self, items: &[crate::menu::MenuItem]) {
        let items = items.to_vec();
        self.shared.run(move |ui| ui.show_templates(items));
    }

    fn focused_field(&self) -> FieldInfo {
        self.focus.field()
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
                // Off GTK's thread: the notification goes over D-Bus and may take a moment.
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
        super::language_from_env()
    }

    fn platform_notes(&self) -> Vec<(String, String)> {
        let mut notes = self.notes.lock().unwrap_or_else(|e| e.into_inner()).clone();
        notes.push(("atspi".to_string(), self.focus.status()));
        notes
    }
}
