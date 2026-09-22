//! The main thread: NSApplication's run loop, which owns the menu bar item, the global shortcut
//! and the panels (AppKit allows them only there). Other threads hand it work through a queue and
//! the main dispatch queue, the macOS counterpart of the Windows version's posted message.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{MainThreadMarker, NSPoint};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::overlay::{screen_frames, BesideMic, Overlay};
use crate::hotkey::HotkeySpec;
use crate::menu::tooltip;
use crate::platform::desktop::{build_menu, to_global_hotkey, tray_icons, update_checks, Timing};
use crate::platform::{IconState, MenuAction, PlatformError, PlatformEvent, TrayState, LIVE_LINES};

/// How long the green check stays after text went in.
const DONE: Duration = Duration::from_millis(800);
/// The bubble goes away this long after the last interim text.
const BUBBLE_IDLE: Duration = Duration::from_millis(1500);
const FULLSCREEN_CHECK: Duration = Duration::from_secs(1);

type Job = Box<dyn FnOnce(&mut Ui) + Send>;

/// Shared between the main thread and everyone who sends it work.
#[derive(Default)]
pub struct Shared {
    queue: Mutex<Vec<Job>>,
    /// Set while the run loop is up; before that, jobs wait in the queue.
    running: AtomicBool,
    menu_actions: Mutex<HashMap<String, MenuAction>>,
    hotkey_id: AtomicU32,
    /// Bumped by each check mark / bubble text, so an older timer knows it is stale.
    done_generation: AtomicU64,
    bubble_generation: AtomicU64,
    /// How fast the beside mic appears after the OS reports a field (child plan C8).
    beside_timing: Mutex<Timing>,
}

impl Shared {
    /// Runs `job` on the main thread (later, if its loop is not up yet).
    pub fn run(self: &Arc<Self>, job: impl FnOnce(&mut Ui) + Send + 'static) {
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(job));
        if self.running.load(Ordering::Acquire) {
            let shared = self.clone();
            DispatchQueue::main().exec_async(move || drain(&shared));
        }
    }

    /// Runs `job` on the main thread and waits for its answer.
    pub fn call<R: Send + 'static>(self: &Arc<Self>, job: impl FnOnce(&mut Ui) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.run(move |ui| {
            let _ = tx.send(job(ui));
        });
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    pub fn beside_timing(&self) -> Option<String> {
        self.beside_timing.lock().unwrap_or_else(|e| e.into_inner()).describe()
    }

    /// Runs `job` on the main thread after `delay`.
    fn later(self: &Arc<Self>, delay: Duration, job: impl FnOnce(&mut Ui) + Send + 'static) {
        let shared = self.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            shared.run(job);
        });
    }
}

pub struct Ui {
    shared: Arc<Shared>,
    mtm: MainThreadMarker,
    tray: Option<TrayIcon>,
    menu: Menu,
    checks: Vec<(MenuAction, CheckMenuItem)>,
    idle_icon: Option<Icon>,
    recording_icon: Option<Icon>,
    hotkeys: Option<GlobalHotKeyManager>,
    current_hotkey: Option<HotKey>,
    overlay: Overlay,
    beside: BesideMic,
    icon_wanted: bool,
    icon_position: Option<(i32, i32)>,
    hide_on_fullscreen: bool,
    hidden_for_fullscreen: bool,
    recording: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
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
        UI.with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                if let Some(ui) = guard.as_mut() {
                    job(ui);
                }
            }
        });
    }
}

impl Ui {
    pub fn set_tray(&mut self, state: TrayState) {
        update_checks(&self.checks, &state);
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
                self.overlay.show(self.overlay.look(), self.icon_position);
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
        if self.hidden_for_fullscreen {
            self.overlay.set_look(look);
            return;
        }
        if self.overlay.is_shown() {
            self.overlay.set_look(look);
        } else {
            self.overlay.show(look, position);
        }
        if look == IconState::Done {
            let generation = self.shared.done_generation.fetch_add(1, Ordering::AcqRel) + 1;
            let shared = self.shared.clone();
            self.shared.later(DONE, move |ui| {
                if shared.done_generation.load(Ordering::Acquire) == generation && ui.overlay.look() == IconState::Done
                {
                    ui.overlay.set_look(IconState::Idle);
                }
            });
        }
    }

    pub fn hide_icon(&mut self) {
        self.icon_wanted = false;
        self.overlay.hide();
    }

    pub fn show_bubble(&mut self, text: &str) {
        self.show_bubble_for(text, BUBBLE_IDLE, LIVE_LINES);
    }

    /// Shows `text` above the mic for `hold`; false when there is no mic on screen to speak from
    /// (switched off, or hidden over a full-screen app).
    pub fn show_bubble_for(&mut self, text: &str, hold: Duration, max_lines: i32) -> bool {
        if !self.overlay.is_shown() {
            return false;
        }
        self.overlay.show_bubble(text, max_lines);
        let generation = self.shared.bubble_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let shared = self.shared.clone();
        self.shared.later(hold, move |ui| {
            if shared.bubble_generation.load(Ordering::Acquire) == generation {
                ui.overlay.hide_bubble();
            }
        });
        true
    }

    pub fn hide_bubble(&mut self) {
        self.overlay.hide_bubble();
    }

    pub fn show_beside(&mut self, pos: (i32, i32), look: IconState, reported_at: Instant) {
        self.beside.show(pos, look);
        let ms = reported_at.elapsed().as_millis();
        tracing::info!(ms, "beside mic shown");
        self.shared.beside_timing.lock().unwrap_or_else(|e| e.into_inner()).record(ms);
    }

    pub fn hide_beside(&mut self) {
        self.beside.hide();
    }

    pub fn set_beside_look(&mut self, look: IconState) {
        self.beside.set_look(look);
    }

    /// Hides the mic while the front app covers a whole screen. `fullScreenAuxiliary` lets the
    /// panel float over it; this is for the users who asked not to see it there.
    fn check_fullscreen(&mut self) {
        if !self.hide_on_fullscreen || !self.icon_wanted {
            return;
        }
        let busy = super::fullscreen::front_app_covers_a_screen(&screen_frames(self.mtm));
        if busy && !self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = true;
            self.overlay.hide();
        } else if !busy && self.hidden_for_fullscreen {
            self.hidden_for_fullscreen = false;
            self.overlay.show(self.overlay.look(), self.icon_position);
        }
    }

    /// The menu bar item. AppKit wants the run loop running before it is made.
    fn create_tray(&mut self) {
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(self.menu.clone()))
            .with_menu_on_left_click(false)
            .with_tooltip(tooltip());
        if let Some(icon) = self.idle_icon.clone() {
            builder = builder.with_icon(icon);
        }
        self.tray = match builder.build() {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(error = %e, "menu bar item failed");
                None
            }
        };
    }
}

pub fn run(shared: Arc<Shared>, events: Sender<PlatformEvent>) -> Result<(), PlatformError> {
    let mtm = MainThreadMarker::new().ok_or_else(|| PlatformError::Failed("not on the main thread".into()))?;
    let app = NSApplication::sharedApplication(mtm);
    // A menu bar app: no Dock icon, no menu bar of its own, never the active app by itself.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // Asks once for the permission to type (macOS shows its own dialog, only when not allowed).
    if !super::ax::trusted(true) {
        tracing::info!("Accessibility is not allowed yet; asked the user");
    }

    let (menu, checks) = build_menu(&shared.menu_actions);
    let (idle_icon, recording_icon) = tray_icons();
    {
        let events = events.clone();
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = events.send(PlatformEvent::ToggleRequested);
            }
        }));
    }
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

    let beside = BesideMic::new(events.clone(), mtm);
    let overlay = Overlay::new(events, mtm, menu.clone());
    UI.with(|slot| {
        *slot.borrow_mut() = Some(Ui {
            shared: shared.clone(),
            mtm,
            tray: None,
            menu,
            checks,
            idle_icon,
            recording_icon,
            hotkeys: None,
            current_hotkey: None,
            overlay,
            beside,
            icon_wanted: false,
            icon_position: None,
            hide_on_fullscreen: true,
            hidden_for_fullscreen: false,
            recording: false,
        });
    });

    // First job once the loop runs: the menu bar item, then whatever queued up before.
    {
        let mut q = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.insert(0, Box::new(|ui: &mut Ui| ui.create_tray()));
    }
    shared.running.store(true, Ordering::Release);
    {
        let shared = shared.clone();
        DispatchQueue::main().exec_async(move || drain(&shared));
    }
    {
        let shared = shared.clone();
        thread::spawn(move || {
            while shared.running.load(Ordering::Acquire) {
                thread::sleep(FULLSCREEN_CHECK);
                shared.run(|ui| ui.check_fullscreen());
            }
        });
    }

    app.run();

    shared.running.store(false, Ordering::Release);
    UI.with(|slot| slot.borrow_mut().take());
    Ok(())
}

/// Ends `app.run()`. `stop` takes effect after the next event, so post one.
pub fn quit() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::new(0.0, 0.0),
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(wake) = wake {
        app.postEvent_atStart(&wake, true);
    }
}
