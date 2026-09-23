//! The GTK thread: GTK's main loop, which owns the tray (AppIndicator via tray-icon) and, on X11,
//! the floating mic. Other threads hand it work through a queue and `glib::idle_add_once`, the
//! Linux counterpart of the Windows version's posted message.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gtk::glib;
use tray_icon::menu::{CheckMenuItem, MenuEvent};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use super::overlay::Overlay;
use crate::hotkey::HotkeySpec;
use crate::linux_setup::Session;
use crate::menu::{tooltip, TemplateList};
use crate::platform::desktop::{build_menu, to_global_hotkey, tray_icons, update_checks};
use crate::platform::{IconState, MenuAction, PlatformError, PlatformEvent, TrayState, VoiceCue, LIVE_LINES};

/// How long the green check stays after text went in.
const DONE: Duration = Duration::from_millis(800);
/// The bubble goes away this long after the last interim text.
const BUBBLE_IDLE: Duration = Duration::from_millis(1500);
const FULLSCREEN_CHECK: Duration = Duration::from_secs(1);

type Job = Box<dyn FnOnce(&mut Ui) + Send>;

/// Shared between the GTK thread and everyone who sends it work.
#[derive(Default)]
pub struct Shared {
    queue: Mutex<Vec<Job>>,
    /// Set while GTK's loop is up; before that, jobs wait in the queue.
    running: AtomicBool,
    menu_actions: Mutex<HashMap<String, MenuAction>>,
    hotkey_id: AtomicU32,
}

impl Shared {
    /// Runs `job` on the GTK thread (later, if its loop is not up yet).
    pub fn run(self: &Arc<Self>, job: impl FnOnce(&mut Ui) + Send + 'static) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(job));
        if self.running.load(Ordering::Acquire) {
            let shared = self.clone();
            glib::idle_add_once(move || drain(&shared));
        }
    }

    /// Runs `job` on the GTK thread and waits for its answer.
    pub fn call<R: Send + 'static>(self: &Arc<Self>, job: impl FnOnce(&mut Ui) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.run(move |ui| {
            let _ = tx.send(job(ui));
        });
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }
}

pub struct Ui {
    tray: Option<TrayIcon>,
    checks: Vec<(MenuAction, CheckMenuItem)>,
    idle_icon: Option<Icon>,
    recording_icon: Option<Icon>,
    hotkeys: Option<GlobalHotKeyManager>,
    current_hotkey: Option<HotKey>,
    shared: Arc<Shared>,
    /// X11 only; Wayland has no floating mic.
    overlay: Option<Overlay>,
    icon_wanted: bool,
    icon_position: Option<(i32, i32)>,
    hide_on_fullscreen: bool,
    hidden_for_fullscreen: bool,
    recording: bool,
    /// Bumped by each check mark / bubble text, so an older timer knows it is stale.
    done_generation: u64,
    bubble_generation: u64,
    /// Words kept above the mic (plan C11): in the bubble whenever nothing else is.
    kept: Option<String>,
    /// Live text or a message is in the bubble until its timer runs out.
    bubble_busy: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui(f: impl FnOnce(&mut Ui)) {
    UI.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            if let Some(ui) = guard.as_mut() {
                f(ui);
            }
        }
    });
}

/// Runs queued jobs one at a time, each with its own borrow of the UI state.
fn drain(shared: &Shared) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
            if q.is_empty() {
                return;
            }
            q.remove(0)
        };
        with_ui(|ui| job(ui));
    }
}

impl Ui {
    pub fn set_tray(&mut self, state: TrayState) {
        update_checks(&self.checks, &state);
        if let Some(overlay) = &mut self.overlay {
            overlay.set_mode(state.mode);
        }
        self.hide_on_fullscreen = state.hide_on_fullscreen;
        if state.recording != self.recording {
            self.recording = state.recording;
            if let Some(tray) = &self.tray {
                let icon = if state.recording { self.recording_icon.clone() } else { self.idle_icon.clone() };
                let _ = tray.set_icon(icon);
            }
        }
        if !self.hide_on_fullscreen && self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = false;
            if self.icon_wanted {
                if let Some(overlay) = &mut self.overlay {
                    overlay.show(overlay.look(), self.icon_position);
                }
                self.bubble_free();
            }
        }
    }

    pub fn register_hotkey(&mut self, spec: &HotkeySpec) -> Result<(), PlatformError> {
        let manager = match &self.hotkeys {
            Some(m) => m,
            None => {
                self.hotkeys = Some(GlobalHotKeyManager::new().map_err(PlatformError::failed)?);
                self.hotkeys.as_ref().unwrap()
            }
        };
        if let Some(old) = self.current_hotkey.take() {
            let _ = manager.unregister(old);
        }
        let hotkey = to_global_hotkey(spec).ok_or_else(|| PlatformError::Failed(format!("{spec} is not supported")))?;
        manager.register(hotkey).map_err(PlatformError::failed)?;
        self.shared.hotkey_id.store(hotkey.id(), Ordering::Release);
        self.current_hotkey = Some(hotkey);
        Ok(())
    }

    pub fn show_icon(&mut self, look: IconState, position: Option<(i32, i32)>) {
        self.icon_wanted = true;
        self.icon_position = position;
        let Some(overlay) = &mut self.overlay else { return };
        if self.hidden_for_fullscreen || overlay.is_shown() {
            overlay.set_look(look);
        } else {
            overlay.show(look, position);
            self.bubble_free();
        }
        if look == IconState::Done && !self.hidden_for_fullscreen {
            self.done_generation += 1;
            let generation = self.done_generation;
            glib::timeout_add_local_once(DONE, move || {
                with_ui(|ui| {
                    if ui.done_generation == generation {
                        if let Some(o) = &mut ui.overlay {
                            if o.look() == IconState::Done {
                                o.set_look(IconState::Idle);
                            }
                        }
                    }
                })
            });
        }
    }

    pub fn hide_icon(&mut self) {
        self.icon_wanted = false;
        if let Some(overlay) = &mut self.overlay {
            overlay.hide();
        }
    }

    pub fn set_icon_scale(&mut self, percent: u16) {
        if let Some(overlay) = &mut self.overlay {
            overlay.set_scale(percent);
        }
    }

    /// Opens the templates list at the mic from GTK's loop (not inside this job).
    pub fn show_templates(&mut self, list: TemplateList) {
        let Some(overlay) = &mut self.overlay else { return };
        overlay.set_templates(list);
        glib::idle_add_local_once(super::overlay::show_templates_menu);
    }

    /// The daemon's list after a deletion, for the list if it is still open.
    pub fn refresh_templates(&mut self, list: TemplateList) {
        super::template_menu::refresh(list);
    }

    pub fn voice_cue(&mut self, cue: VoiceCue) {
        if let Some(overlay) = &mut self.overlay {
            overlay.voice_cue(cue);
        }
    }

    pub fn show_bubble(&mut self, text: &str) {
        self.show_bubble_for(text, BUBBLE_IDLE, LIVE_LINES);
    }

    /// Shows `text` above the mic for `hold`; false when there is no mic on screen to speak from
    /// (Wayland, switched off, or hidden over a full-screen app).
    pub fn show_bubble_for(&mut self, text: &str, hold: Duration, max_lines: i32) -> bool {
        let Some(overlay) = &mut self.overlay else { return false };
        if !overlay.is_shown() {
            return false;
        }
        overlay.show_bubble(text, max_lines);
        self.bubble_busy = true;
        self.bubble_generation += 1;
        let generation = self.bubble_generation;
        glib::timeout_add_local_once(hold, move || {
            with_ui(|ui| {
                if ui.bubble_generation == generation {
                    ui.hide_bubble();
                }
            })
        });
        true
    }

    /// Takes live text or a message away; kept words come back in their place.
    pub fn hide_bubble(&mut self) {
        self.bubble_busy = false;
        self.bubble_free();
    }

    /// Words kept above the mic, or (`None`) none any more. Shown now unless live text or a
    /// message is in the bubble; then when it goes.
    pub fn show_kept(&mut self, words: Option<String>) {
        self.kept = words;
        if !self.bubble_busy {
            self.bubble_free();
        }
    }

    /// Nothing else is in the bubble: the kept words are, if there are any.
    fn bubble_free(&mut self) {
        let Some(overlay) = &mut self.overlay else { return };
        match &self.kept {
            Some(words) if overlay.is_shown() => overlay.show_kept(words),
            _ => overlay.hide_bubble(),
        }
    }

    /// Hides the mic while the active window is full screen (a video, a game, a presentation).
    fn check_fullscreen(&mut self) {
        if !self.hide_on_fullscreen || !self.icon_wanted {
            return;
        }
        let Some(overlay) = &mut self.overlay else { return };
        let busy = super::x11::active_window_is_fullscreen();
        if busy && !self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = true;
            overlay.hide();
        } else if !busy && self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = false;
            overlay.show(overlay.look(), self.icon_position);
            self.bubble_free();
        }
    }
}

pub fn run(shared: Arc<Shared>, events: Sender<PlatformEvent>, session: Session) -> Result<(), PlatformError> {
    gtk::init().map_err(|e| PlatformError::Failed(format!("GTK could not start: {e}")))?;

    let (menu, checks) = build_menu(&shared.menu_actions);
    let (idle_icon, recording_icon) = tray_icons();
    // AppIndicator reports no clicks; the menu's first item starts and stops instead.
    let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_tooltip(tooltip());
    if let Some(icon) = idle_icon.clone() {
        builder = builder.with_icon(icon);
    }
    let tray = match builder.build() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(error = %e, "tray icon failed");
            None
        }
    };
    {
        let events = events.clone();
        let shared = shared.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let action = shared.menu_actions.lock().unwrap_or_else(|p| p.into_inner()).get(&e.id.0).copied();
            if let Some(action) = action {
                let _ = events.send(PlatformEvent::Menu(action));
            }
        }));
    }
    {
        let events = events.clone();
        let shared = shared.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed && e.id == shared.hotkey_id.load(Ordering::Acquire) {
                let _ = events.send(PlatformEvent::ToggleRequested);
            }
        }));
    }

    let overlay = (session == Session::X11).then(|| Overlay::new(events, menu.clone()));
    let has_overlay = overlay.is_some();
    UI.with(|slot| {
        *slot.borrow_mut() = Some(Ui {
            tray,
            checks,
            idle_icon,
            recording_icon,
            hotkeys: None,
            current_hotkey: None,
            shared: shared.clone(),
            overlay,
            icon_wanted: false,
            icon_position: None,
            hide_on_fullscreen: true,
            hidden_for_fullscreen: false,
            recording: false,
            done_generation: 0,
            bubble_generation: 0,
            kept: None,
            bubble_busy: false,
        });
    });

    shared.running.store(true, Ordering::Release);
    drain(&shared);
    if has_overlay {
        glib::timeout_add_local(FULLSCREEN_CHECK, || {
            with_ui(|ui| ui.check_fullscreen());
            glib::ControlFlow::Continue
        });
    }

    gtk::main();

    shared.running.store(false, Ordering::Release);
    UI.with(|slot| slot.borrow_mut().take());
    Ok(())
}

pub fn quit() {
    gtk::main_quit();
}
