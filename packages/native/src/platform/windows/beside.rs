//! What the beside mic needs from UI Automation (child plan C8-C1): a thread of its own (MTA)
//! that either listens for focus changes, or looks at what is under the pointer every 150 ms
//! (no input hooks), and reports each text field as a `FieldChanged` event.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use windows::core::{implement, Ref};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationFocusChangedEventHandler,
    IUIAutomationFocusChangedEventHandler_Impl, IUIAutomationTextPattern2, IUIAutomationTextRange,
    IUIAutomationValuePattern, TextUnit_Character, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    UIA_TextPattern2Id, UIA_ValuePatternId,
};

use crate::beside_field::{FieldProbe, HoverChange, HoverTracker, Pointed, HOVER_POLL};
use crate::config::BesideFieldTrigger;
use crate::platform::{PlatformEvent, Rect};

fn to_rect(r: windows::Win32::Foundation::RECT) -> Rect {
    Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
}

/// The rectangles of a text range, as UIA hands them over: x, y, width, height, … as doubles.
unsafe fn first_rect(range: &IUIAutomationTextRange) -> Option<Rect> {
    let array: *mut SAFEARRAY = range.GetBoundingRectangles().ok()?;
    if array.is_null() {
        return None;
    }
    let mut out = None;
    if let Ok(upper) = SafeArrayGetUBound(array, 1) {
        let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
        if upper >= 3 && SafeArrayAccessData(array, &mut data).is_ok() {
            let v = std::slice::from_raw_parts(data as *const f64, 4);
            out = Some(Rect {
                x: v[0].round() as i32,
                y: v[1].round() as i32,
                width: v[2].round() as i32,
                height: v[3].round() as i32,
            });
            let _ = SafeArrayUnaccessData(array);
        }
    }
    let _ = SafeArrayDestroy(array);
    out
}

/// Where the caret is. An empty caret range often has no rectangle; the character at it does.
unsafe fn caret_rect(element: &IUIAutomationElement) -> Option<Rect> {
    let text: IUIAutomationTextPattern2 = element.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
    let mut active = windows::core::BOOL(0);
    let range = text.GetCaretRange(&mut active).ok()?;
    if let Some(r) = first_rect(&range).filter(|r| r.height > 0) {
        return Some(r);
    }
    let wider = range.Clone().ok()?;
    wider.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
    first_rect(&wider).filter(|r| r.height > 0).map(|r| Rect { width: 1, ..r })
}

/// What an element is, for the beside mic. `with_caret` is false for hover (the pointer, not the
/// caret, says where the user is looking).
pub fn probe(element: &IUIAutomationElement, with_caret: bool) -> FieldProbe {
    unsafe {
        let kind = element.CurrentControlType().ok();
        let is_edit_kind = kind == Some(UIA_EditControlTypeId) || kind == Some(UIA_DocumentControlTypeId);
        let focusable = element.CurrentIsKeyboardFocusable().map(|b| b.as_bool()).unwrap_or(false);
        let read_only = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|v| v.CurrentIsReadOnly().ok())
            .is_some_and(|b| b.as_bool());
        let is_text_field = is_edit_kind && focusable && !read_only;
        FieldProbe {
            is_text_field,
            is_password: element.CurrentIsPassword().ok().map(|b| b.as_bool()),
            app_id: element.CurrentProcessId().ok().and_then(|pid| super::system_process_name(pid as u32)),
            caret: if is_text_field && with_caret { caret_rect(element) } else { None },
            bounds: element.CurrentBoundingRectangle().ok().map(to_rect),
        }
    }
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    events: Sender<PlatformEvent>,
    own_pid: u32,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(&self, sender: Ref<IUIAutomationElement>) -> windows::core::Result<()> {
        let at = Instant::now();
        if let Some(element) = sender.as_ref() {
            let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0) as u32;
            if pid != self.own_pid {
                let _ = self.events.send(PlatformEvent::FieldChanged { probe: probe(element, true), at });
            }
        }
        Ok(())
    }
}

fn watch_focus(uia: &IUIAutomation, events: Sender<PlatformEvent>, stop: mpsc::Receiver<()>) {
    let handler: IUIAutomationFocusChangedEventHandler = FocusHandler { events, own_pid: std::process::id() }.into();
    if let Err(e) = unsafe { uia.AddFocusChangedEventHandler(None, &handler) } {
        tracing::warn!(error = %e, "could not listen for focus changes");
        return;
    }
    tracing::info!("beside mic: listening for focus changes");
    let _ = stop.recv();
    let _ = unsafe { uia.RemoveFocusChangedEventHandler(&handler) };
}

fn cursor() -> POINT {
    let mut p = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) };
    POINT { x: p.x, y: p.y }
}

fn watch_hover(uia: &IUIAutomation, events: Sender<PlatformEvent>, stop: mpsc::Receiver<()>) {
    let own_pid = std::process::id();
    tracing::info!("beside mic: watching the pointer");
    let mut tracker: HoverTracker<(u32, Rect)> = HoverTracker::default();
    // The field the pointer is on, with when it got there (to measure how fast the mic comes).
    let mut current: Option<((u32, Rect), FieldProbe, Instant)> = None;
    loop {
        match stop.recv_timeout(HOVER_POLL) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }
        let now = Instant::now();
        let pointed = match unsafe { uia.ElementFromPoint(cursor()) } {
            Err(_) => Pointed::Other,
            Ok(element) => {
                let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0) as u32;
                if pid == own_pid {
                    Pointed::Ours
                } else {
                    let p = probe(&element, false);
                    match (p.is_text_field, p.bounds) {
                        (true, Some(b)) => {
                            let key = (pid, b);
                            if current.as_ref().map(|(k, _, _)| *k) != Some(key) {
                                current = Some((key, p, now));
                            }
                            Pointed::Field(key)
                        }
                        _ => Pointed::Other,
                    }
                }
            }
        };
        match tracker.observe(now, pointed) {
            Some(HoverChange::Show(key)) => {
                if let Some((_, p, since)) = current.as_ref().filter(|(k, _, _)| *k == key) {
                    let _ = events.send(PlatformEvent::FieldChanged { probe: p.clone(), at: *since });
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
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    pub fn start(trigger: BesideFieldTrigger, events: Sender<PlatformEvent>) -> Watcher {
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("beside-field".into())
            .spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(uia) => match trigger {
                        BesideFieldTrigger::Focus => watch_focus(&uia, events, stop_rx),
                        BesideFieldTrigger::Hover => watch_hover(&uia, events, stop_rx),
                    },
                    Err(e) => tracing::warn!(error = %e, "UI Automation is not available"),
                }
                CoUninitialize();
            })
            .ok();
        Watcher { stop: Some(stop_tx), thread }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
