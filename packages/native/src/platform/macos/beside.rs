//! What the beside mic needs from the Accessibility API (child plan C8-C2): a thread of its own
//! that either follows the front app's focused element through an AXObserver (moved to the new
//! app whenever the front app changes), or looks at what is under the pointer every 150 ms.
//! Each text field is reported as a `FieldChanged` event. AppKit is not touched here.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use objc2_application_services::{AXError, AXObserver, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{
    kCFRunLoopDefaultMode, CFBoolean, CFRange, CFRetained, CFRunLoop, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::CGEvent;

use super::ax::{attribute, bundle_id, point_or_size};
use crate::beside_field::{FieldProbe, HoverChange, HoverTracker, Pointed, HOVER_POLL};
use crate::config::BesideFieldTrigger;
use crate::platform::{PlatformEvent, Rect};

pub(super) const TEXT_ROLES: [&str; 3] = ["AXTextField", "AXTextArea", "AXComboBox"];
/// How often the focus thread looks for a new front app (and lets the observer's callbacks run).
const FRONT_APP_POLL: Duration = Duration::from_millis(250);

fn string_attribute(element: &AXUIElement, name: &'static str) -> Option<String> {
    attribute(element, name)?.downcast::<CFString>().ok().map(|s| s.to_string())
}

fn to_rect(origin: CGPoint, size: CGSize) -> Rect {
    Rect {
        x: origin.x.round() as i32,
        y: origin.y.round() as i32,
        width: size.width.round() as i32,
        height: size.height.round() as i32,
    }
}

/// The caret: the bounds of the selected range (an empty selection is the caret itself).
fn caret_rect(element: &AXUIElement) -> Option<Rect> {
    let range = attribute(element, "AXSelectedTextRange")?.downcast::<AXValue>().ok()?;
    let mut selected = CFRange { location: 0, length: 0 };
    if !unsafe { range.value(AXValueType::CFRange, NonNull::from(&mut selected).cast::<c_void>()) } {
        return None;
    }
    let param = unsafe { AXValue::new(AXValueType::CFRange, NonNull::from(&mut selected).cast::<c_void>()) }?;
    let name = CFString::from_static_str("AXBoundsForRange");
    let mut out: *const CFType = std::ptr::null();
    let err = unsafe { element.copy_parameterized_attribute_value(&name, &param, NonNull::from(&mut out)) };
    if err != AXError::Success || out.is_null() {
        return None;
    }
    let bounds = unsafe { CFRetained::from_raw(NonNull::new(out as *mut CFType)?) }.downcast::<AXValue>().ok()?;
    let mut rect = CGRect::default();
    if !unsafe { bounds.value(AXValueType::CGRect, NonNull::from(&mut rect).cast::<c_void>()) } {
        return None;
    }
    Some(to_rect(rect.origin, rect.size)).filter(|r| r.height > 0)
}

/// What an element is, for the beside mic.
pub fn probe(element: &AXUIElement, with_caret: bool) -> FieldProbe {
    let role = string_attribute(element, "AXRole");
    let subrole = string_attribute(element, "AXSubrole");
    let is_text_field = role.as_deref().is_some_and(|r| TEXT_ROLES.contains(&r));
    let position: Option<CGPoint> = point_or_size(element, "AXPosition", AXValueType::CGPoint);
    let size: Option<CGSize> = point_or_size(element, "AXSize", AXValueType::CGSize);
    FieldProbe {
        is_text_field,
        is_password: role.as_ref().map(|_| subrole.as_deref() == Some("AXSecureTextField")),
        app_id: bundle_id(element),
        caret: if is_text_field && with_caret { caret_rect(element) } else { None },
        bounds: position.zip(size).map(|(p, s)| to_rect(p, s)),
    }
}

fn pid_of(element: &AXUIElement) -> Option<i32> {
    let mut pid: i32 = 0;
    let err = unsafe { element.pid(NonNull::from(&mut pid)) };
    (err == AXError::Success && pid > 0).then_some(pid)
}

/// Where the observer's callback sends its reports (a C callback has no closure to carry them).
static EVENTS: Mutex<Option<Sender<PlatformEvent>>> = Mutex::new(None);

fn report(element: &AXUIElement, at: Instant) {
    let probe = probe(element, true);
    if let Some(events) = EVENTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = events.send(PlatformEvent::FieldChanged { probe, at });
    }
}

unsafe extern "C-unwind" fn focus_changed(
    _observer: NonNull<AXObserver>,
    element: NonNull<AXUIElement>,
    _notification: NonNull<CFString>,
    _refcon: *mut c_void,
) {
    report(unsafe { element.as_ref() }, Instant::now());
}

struct Attached {
    pid: i32,
    observer: CFRetained<AXObserver>,
    app: CFRetained<AXUIElement>,
}

fn attach(pid: i32, run_loop: &CFRunLoop) -> Option<Attached> {
    let mut raw: *mut AXObserver = std::ptr::null_mut();
    let err = unsafe { AXObserver::create(pid, Some(focus_changed), NonNull::from(&mut raw)) };
    if err != AXError::Success {
        return None;
    }
    let observer = unsafe { CFRetained::from_raw(NonNull::new(raw)?) };
    let app = unsafe { AXUIElement::new_application(pid) };
    // Electron apps only expose their elements when asked to; failing is fine.
    let manual = CFString::from_static_str("AXManualAccessibility");
    let _ = unsafe { app.set_attribute_value(&manual, CFBoolean::new(true)) };
    let notification = CFString::from_static_str("AXFocusedUIElementChanged");
    if unsafe { observer.add_notification(&app, &notification, std::ptr::null_mut()) } != AXError::Success {
        return None;
    }
    let source = unsafe { observer.run_loop_source() };
    run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
    Some(Attached { pid, observer, app })
}

fn detach(attached: Attached, run_loop: &CFRunLoop) {
    let notification = CFString::from_static_str("AXFocusedUIElementChanged");
    let _ = unsafe { attached.observer.remove_notification(&attached.app, &notification) };
    let source = unsafe { attached.observer.run_loop_source() };
    run_loop.remove_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
}

fn watch_focus(stop: &AtomicBool, events: Sender<PlatformEvent>) {
    *EVENTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(events);
    let Some(run_loop) = CFRunLoop::current() else { return };
    let system = unsafe { AXUIElement::new_system_wide() };
    let own = std::process::id() as i32;
    let mut attached: Option<Attached> = None;
    tracing::info!("beside mic: following the focused element");
    while !stop.load(Ordering::Acquire) {
        let front = attribute(&system, "AXFocusedApplication")
            .and_then(|v| v.downcast::<AXUIElement>().ok())
            .and_then(|app| pid_of(&app))
            .filter(|pid| *pid != own);
        if front != attached.as_ref().map(|a| a.pid) {
            if let Some(old) = attached.take() {
                detach(old, &run_loop);
            }
            if let Some(pid) = front {
                attached = attach(pid, &run_loop);
                // A new front app brings its own focused element; say what it is.
                if let Some(a) = &attached {
                    if let Some(focused) =
                        attribute(&a.app, "AXFocusedUIElement").and_then(|v| v.downcast::<AXUIElement>().ok())
                    {
                        report(&focused, Instant::now());
                    }
                }
            }
        }
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, FRONT_APP_POLL.as_secs_f64(), false);
    }
    if let Some(old) = attached.take() {
        detach(old, &run_loop);
    }
    *EVENTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The pointer, in points from the top-left of the main screen (the space AX uses).
fn pointer() -> Option<CGPoint> {
    let event = CGEvent::new(None)?;
    Some(CGEvent::location(Some(&event)))
}

fn watch_hover(stop: &AtomicBool, events: Sender<PlatformEvent>) {
    let system = unsafe { AXUIElement::new_system_wide() };
    let own = std::process::id() as i32;
    let mut tracker: HoverTracker<(i32, Rect)> = HoverTracker::default();
    let mut current: Option<((i32, Rect), FieldProbe, Instant)> = None;
    tracing::info!("beside mic: watching the pointer");
    while !stop.load(Ordering::Acquire) {
        thread::sleep(HOVER_POLL);
        let now = Instant::now();
        let pointed = match pointer() {
            None => Pointed::Other,
            Some(p) => {
                let mut raw: *const AXUIElement = std::ptr::null();
                let err = unsafe { system.copy_element_at_position(p.x as f32, p.y as f32, NonNull::from(&mut raw)) };
                match NonNull::new(raw as *mut AXUIElement).filter(|_| err == AXError::Success) {
                    None => Pointed::Other,
                    Some(ptr) => {
                        let element = unsafe { CFRetained::from_raw(ptr) };
                        let pid = pid_of(&element).unwrap_or(0);
                        if pid == own {
                            Pointed::Ours
                        } else {
                            let probe = probe(&element, false);
                            match (probe.is_text_field, probe.bounds) {
                                (true, Some(b)) => {
                                    let key = (pid, b);
                                    if current.as_ref().map(|(k, _, _)| *k) != Some(key) {
                                        current = Some((key, probe, now));
                                    }
                                    Pointed::Field(key)
                                }
                                _ => Pointed::Other,
                            }
                        }
                    }
                }
            }
        };
        match tracker.observe(now, pointed) {
            Some(HoverChange::Show(key)) => {
                if let Some((_, probe, since)) = current.as_ref().filter(|(k, _, _)| *k == key) {
                    let _ = events.send(PlatformEvent::FieldChanged { probe: probe.clone(), at: *since });
                }
            }
            Some(HoverChange::Hide) => {
                current = None;
                let _ = events.send(PlatformEvent::FieldChanged { probe: FieldProbe::default(), at: now });
            }
            None => {}
        }
    }
}

/// The watching thread; dropping it stops it.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    pub fn start(trigger: BesideFieldTrigger, events: Sender<PlatformEvent>) -> Watcher {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("beside-field".into())
            .spawn(move || match trigger {
                BesideFieldTrigger::Focus => watch_focus(&flag, events),
                BesideFieldTrigger::Hover => watch_hover(&flag, events),
            })
            .ok();
        Watcher { stop, thread }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
