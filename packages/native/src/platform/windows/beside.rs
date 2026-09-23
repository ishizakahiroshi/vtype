//! What the beside mic needs from UI Automation (child plan C8-C1): a thread of its own (MTA)
//! that either listens for focus changes, or looks at what is under the pointer every 150 ms
//! (no input hooks), and reports each text field as a `FieldChanged` event. On request it also
//! watches one app for a while and reports what it saw (plan C1, `Watcher::check`).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::core::{implement, Ref};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationFocusChangedEventHandler,
    IUIAutomationFocusChangedEventHandler_Impl, IUIAutomationTextPattern2, IUIAutomationTextRange,
    IUIAutomationTreeWalker, IUIAutomationValuePattern, TextUnit_Character, UIA_ButtonControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_TextControlTypeId,
    UIA_TextPattern2Id, UIA_TextPatternId, UIA_ValuePatternId, UIA_WindowControlTypeId, UIA_CONTROLTYPE_ID,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, GA_ROOT};

use crate::beside_field::{FieldProbe, HoverChange, HoverTracker, Pointed, HOVER_POLL};
use crate::config::BesideFieldTrigger;
use crate::field_check::{Element, Path, Record, CHECK_FOR};
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

/// Windows Terminal's text area: a Text element, not an Edit.
const TERMINAL_CLASS: &str = "TermControl";

/// Whether an element takes typed text, from what UI Automation says about it: an edit or a
/// document; a combo box that has a text pattern (Chrome gives Google's search box as one; a
/// drop-down list has none); Windows Terminal's text area. Not when read-only or unfocusable.
fn takes_text(kind: UIA_CONTROLTYPE_ID, class: &str, has_text: bool, focusable: bool, read_only: bool) -> bool {
    let typed_into = kind == UIA_EditControlTypeId
        || kind == UIA_DocumentControlTypeId
        || (kind == UIA_ComboBoxControlTypeId && has_text)
        || (kind == UIA_TextControlTypeId && class == TERMINAL_CLASS);
    typed_into && focusable && !read_only
}

/// What an element is, for the beside mic. `with_caret` is false for hover (the pointer, not the
/// caret, says where the user is looking).
pub fn probe(element: &IUIAutomationElement, with_caret: bool) -> FieldProbe {
    unsafe {
        let kind = element.CurrentControlType().unwrap_or_default();
        let class = if kind == UIA_TextControlTypeId {
            element.CurrentClassName().map(|s| s.to_string()).unwrap_or_default()
        } else {
            String::new()
        };
        let has_text = kind == UIA_ComboBoxControlTypeId && element.GetCurrentPattern(UIA_TextPatternId).is_ok();
        let focusable = element.CurrentIsKeyboardFocusable().map(|b| b.as_bool()).unwrap_or(false);
        let read_only = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|v| v.CurrentIsReadOnly().ok())
            .is_some_and(|b| b.as_bool());
        let is_text_field = takes_text(kind, &class, has_text, focusable, read_only);
        FieldProbe {
            is_text_field,
            is_password: element.CurrentIsPassword().ok().map(|b| b.as_bool()),
            app_id: element.CurrentProcessId().ok().and_then(|pid| super::system_process_name(pid as u32)),
            caret: if is_text_field && with_caret { caret_rect(element) } else { None },
            bounds: element.CurrentBoundingRectangle().ok().map(to_rect),
            pointer: None,
            on_taskbar: false,
        }
    }
}

/// The taskbar and its tray belong to Explorer.
const SHELL: &str = "explorer.exe";
/// The windows of the taskbar, a second screen's taskbar, and the tray's overflow (Windows 10,
/// Windows 11): where vtype's tray menu is opened from.
const TASKBAR_CLASSES: [&str; 4] =
    ["Shell_TrayWnd", "Shell_SecondaryTrayWnd", "NotifyIconOverflowWindow", "TopLevelWindowForOverflowXamlIsland"];
/// How far up from an element its window is looked for.
const MAX_DEPTH: usize = 16;

fn window_class(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Whether an element is part of the taskbar: the top-level window of the nearest element that
/// has a window of its own (the taskbar's buttons do not).
fn on_taskbar(walker: &IUIAutomationTreeWalker, element: &IUIAutomationElement) -> bool {
    let mut current = element.clone();
    for _ in 0..MAX_DEPTH {
        let hwnd = unsafe { current.CurrentNativeWindowHandle() }.unwrap_or_default();
        if !hwnd.is_invalid() {
            let top = unsafe { GetAncestor(hwnd.0, GA_ROOT) };
            return TASKBAR_CLASSES.contains(&window_class(top).as_str());
        }
        match unsafe { walker.GetParentElement(&current) } {
            Ok(parent) => current = parent,
            Err(_) => return false,
        }
    }
    false
}

/// What the watching thread is told: a focus change (from UI Automation's thread), or to stop.
enum Msg {
    Focus { probe: FieldProbe, pid: u32, window: bool, at: Instant },
    Stop,
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    reports: Sender<Msg>,
    own_pid: u32,
    /// To tell the taskbar from the apps.
    walker: Option<IUIAutomationTreeWalker>,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(&self, sender: Ref<IUIAutomationElement>) -> windows::core::Result<()> {
        let at = Instant::now();
        if let Some(element) = sender.as_ref() {
            let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0) as u32;
            if pid != self.own_pid {
                // Where the pointer is now: after a click, where the user clicked.
                let c = cursor();
                let mut probe = FieldProbe { pointer: Some((c.x, c.y)), ..probe(element, true) };
                // Only Explorer's elements are looked at more closely; File Explorer is an app too.
                probe.on_taskbar = probe.app_id.as_deref() == Some(SHELL)
                    && self.walker.as_ref().is_some_and(|w| on_taskbar(w, element));
                let window = unsafe { element.CurrentControlType() }.is_ok_and(|k| k == UIA_WindowControlTypeId);
                let _ = self.reports.send(Msg::Focus { probe, pid, window, at });
            }
        }
        Ok(())
    }
}

/// Qt apps (LINE) report the focus coming to their window as the window itself, and move it into
/// their text field a moment later without another report (measured on LINE: the focused element
/// was still a message 100 ms after the report, and the text field by 300 ms). After such a
/// report the focused element is looked at again this often…
const RECHECK_EVERY: Duration = Duration::from_millis(100);
/// …for this long: less than the daemon's wait before the mic goes home (700 ms), so a mic at a
/// field is not sent to the corner and back.
const RECHECK_FOR: Duration = Duration::from_millis(600);

/// Whether a report is the window itself, whose text field may take the focus after it.
fn waits_for_field(window: bool, probe: &FieldProbe) -> bool {
    window && !probe.is_text_field
}

/// One more look at the focused element after a window's report.
enum Refocused {
    /// A text field of the same app.
    Field(FieldProbe),
    /// Not a text field (yet).
    NotYet,
    /// Another app has the focus; its own report tells.
    Elsewhere,
}

fn look_again(uia: &IUIAutomation, pid: u32) -> Refocused {
    let Ok(element) = (unsafe { uia.GetFocusedElement() }) else { return Refocused::NotYet };
    match unsafe { element.CurrentProcessId() } {
        Ok(p) if p as u32 == pid => {
            let probe = probe(&element, true);
            if probe.is_text_field {
                Refocused::Field(probe)
            } else {
                Refocused::NotYet
            }
        }
        Ok(_) => Refocused::Elsewhere,
        Err(_) => Refocused::NotYet,
    }
}

/// The text field to report after a window's report, or whether to keep looking.
fn after_look(look: Refocused, now: Instant, until: Instant) -> Result<FieldProbe, bool> {
    match look {
        Refocused::Field(probe) => Ok(probe),
        Refocused::NotYet => Err(now < until),
        Refocused::Elsewhere => Err(false),
    }
}

/// A window's report waiting for its text field.
struct Waiting {
    pid: u32,
    pointer: Option<(i32, i32)>,
    at: Instant,
    until: Instant,
}

fn watch_focus(uia: &IUIAutomation, events: Sender<PlatformEvent>, reports: Sender<Msg>, inbox: Receiver<Msg>) {
    let walker = unsafe { uia.RawViewWalker() }.ok();
    let handler: IUIAutomationFocusChangedEventHandler =
        FocusHandler { reports, own_pid: std::process::id(), walker }.into();
    if let Err(e) = unsafe { uia.AddFocusChangedEventHandler(None, &handler) } {
        tracing::warn!(error = %e, "could not listen for focus changes");
        return;
    }
    tracing::info!("beside mic: listening for focus changes");
    let mut waiting: Option<Waiting> = None;
    loop {
        let msg = match waiting {
            Some(_) => inbox.recv_timeout(RECHECK_EVERY),
            None => inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Focus { probe, pid, window, at }) => {
                // Every report goes on as it is; a newer one ends the wait for the last window's field.
                waiting = waits_for_field(window, &probe).then(|| Waiting {
                    pid,
                    pointer: probe.pointer,
                    at,
                    until: at + RECHECK_FOR,
                });
                let _ = events.send(PlatformEvent::FieldChanged { probe, at });
            }
            Err(RecvTimeoutError::Timeout) => {
                let Some(w) = waiting.take() else { continue };
                match after_look(look_again(uia, w.pid), Instant::now(), w.until) {
                    Ok(probe) => {
                        tracing::info!(
                            after_ms = w.at.elapsed().as_millis() as u64,
                            "beside mic: the window's text field took the focus"
                        );
                        // The pointer as it was at the report: where the user clicked into the window.
                        let probe = FieldProbe { pointer: w.pointer, ..probe };
                        let _ = events.send(PlatformEvent::FieldChanged { probe, at: w.at });
                    }
                    Err(true) => waiting = Some(w),
                    Err(false) => {}
                }
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = unsafe { uia.RemoveFocusChangedEventHandler(&handler) };
}

fn cursor() -> POINT {
    let mut p = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) };
    POINT { x: p.x, y: p.y }
}

fn watch_hover(uia: &IUIAutomation, events: Sender<PlatformEvent>, stop: Receiver<Msg>) {
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

/// UI Automation's names for its control types, from `UIA_ButtonControlTypeId` (50000) on.
const KIND_NAMES: [&str; 41] = [
    "Button",
    "Calendar",
    "CheckBox",
    "ComboBox",
    "Edit",
    "Hyperlink",
    "Image",
    "ListItem",
    "List",
    "Menu",
    "MenuBar",
    "MenuItem",
    "ProgressBar",
    "RadioButton",
    "ScrollBar",
    "Slider",
    "Spinner",
    "StatusBar",
    "Tab",
    "TabItem",
    "Text",
    "ToolBar",
    "ToolTip",
    "Tree",
    "TreeItem",
    "Custom",
    "Group",
    "Thumb",
    "DataGrid",
    "DataItem",
    "Document",
    "SplitButton",
    "Window",
    "Pane",
    "Header",
    "HeaderItem",
    "Table",
    "TitleBar",
    "Separator",
    "SemanticZoom",
    "AppBar",
];

fn kind_name(kind: UIA_CONTROLTYPE_ID) -> String {
    usize::try_from(kind.0 - UIA_ButtonControlTypeId.0)
        .ok()
        .and_then(|i| KIND_NAMES.get(i))
        .map_or_else(|| kind.0.to_string(), |name| name.to_string())
}

/// What a check keeps of an element: what kind it is, never its name or value (they can hold
/// what was written).
fn facts(element: &IUIAutomationElement) -> Element {
    unsafe {
        let kind = element.CurrentControlType().unwrap_or_default();
        let class = element.CurrentClassName().map(|s| s.to_string()).unwrap_or_default();
        let has_text = element.GetCurrentPattern(UIA_TextPatternId).is_ok();
        let value = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok();
        let read_only = value.as_ref().and_then(|v| v.CurrentIsReadOnly().ok()).is_some_and(|b| b.as_bool());
        let keyboard_focusable = element.CurrentIsKeyboardFocusable().map(|b| b.as_bool()).unwrap_or(false);
        Element {
            takes_text: takes_text(kind, &class, has_text, keyboard_focusable, read_only),
            kind: kind_name(kind),
            class,
            keyboard_focusable,
            has_value: value.is_some(),
            read_only,
            has_text,
            bounds: element.CurrentBoundingRectangle().ok().map(to_rect),
        }
    }
}

/// During a check the focused element is also looked at this often, reported or not: clicking a
/// text field that has the focus already reports nothing.
const LOOK_EVERY: Duration = Duration::from_millis(200);

fn is_app(pid: u32, app: &str) -> bool {
    pid != 0 && super::system_process_name(pid).as_deref() == Some(app)
}

/// The focused element and its process.
fn focused(uia: &IUIAutomation) -> Option<(u32, IUIAutomationElement)> {
    let element = unsafe { uia.GetFocusedElement() }.ok()?;
    let pid = unsafe { element.CurrentProcessId() }.ok()? as u32;
    Some((pid, element))
}

/// What the checking thread is told: a focus report of the app it watches, or to stop.
enum CheckMsg {
    Report { element: Element, pid: u32, window: bool, at: Instant },
    Stop,
}

#[implement(IUIAutomationFocusChangedEventHandler)]
struct CheckHandler {
    reports: Sender<CheckMsg>,
    app: String,
}

impl IUIAutomationFocusChangedEventHandler_Impl for CheckHandler_Impl {
    fn HandleFocusChangedEvent(&self, sender: Ref<IUIAutomationElement>) -> windows::core::Result<()> {
        let at = Instant::now();
        if let Some(element) = sender.as_ref() {
            let pid = unsafe { element.CurrentProcessId() }.unwrap_or(0) as u32;
            if is_app(pid, &self.app) {
                let window = unsafe { element.CurrentControlType() }.is_ok_and(|k| k == UIA_WindowControlTypeId);
                let _ = self.reports.send(CheckMsg::Report { element: facts(element), pid, window, at });
            }
        }
        Ok(())
    }
}

/// Watches `app` for `CHECK_FOR`: its focus reports, the looks again after a window's report (as
/// `watch_focus` does), and its focused element every `LOOK_EVERY`. Sends what it saw, unless
/// stopped first.
fn check_app(
    uia: &IUIAutomation,
    app: String,
    events: Sender<PlatformEvent>,
    reports: Sender<CheckMsg>,
    inbox: Receiver<CheckMsg>,
) {
    let handler: IUIAutomationFocusChangedEventHandler = CheckHandler { reports, app: app.clone() }.into();
    // Its own handler, whatever the trigger: with hover nothing else listens for the focus.
    let listening = unsafe { uia.AddFocusChangedEventHandler(None, &handler) }.is_ok();
    if !listening {
        tracing::warn!("field check: could not listen for focus changes");
    }
    let started = Instant::now();
    let until = started + CHECK_FOR;
    let mut record = Record::new(started);
    let mut next_look = started;
    // The first look is what had the focus before the user did anything.
    let mut look_path = Path::Start;
    // After a window's report: its process, when to look again, and until when.
    let mut recheck: Option<(u32, Instant, Instant)> = None;
    let finished = loop {
        let now = Instant::now();
        if now >= until {
            break true;
        }
        if now >= next_look {
            if let Some((_, element)) = focused(uia).filter(|(pid, _)| is_app(*pid, &app)) {
                record.note(now, look_path, facts(&element));
            }
            look_path = Path::Look;
            next_look = now + LOOK_EVERY;
        }
        if let Some((pid, _, give_up)) = recheck.filter(|&(_, due, _)| now >= due) {
            let look = match focused(uia) {
                Some((p, element)) if p == pid => {
                    let element = facts(&element);
                    let field = element.takes_text;
                    record.note(now, Path::Recheck, element);
                    if field {
                        Refocused::Field(FieldProbe::default())
                    } else {
                        Refocused::NotYet
                    }
                }
                Some(_) => Refocused::Elsewhere,
                None => Refocused::NotYet,
            };
            recheck =
                matches!(after_look(look, now, give_up), Err(true)).then_some((pid, now + RECHECK_EVERY, give_up));
        }
        let wake = recheck.map_or(next_look, |(_, due, _)| due.min(next_look)).min(until);
        match inbox.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(CheckMsg::Report { element, pid, window, at }) => {
                // As in `watch_focus`: a window's report is followed by looks again, and a newer
                // report ends them.
                recheck = (window && !element.takes_text).then_some((pid, at + RECHECK_EVERY, at + RECHECK_FOR));
                record.note(at, Path::Report, element);
            }
            Ok(CheckMsg::Stop) | Err(RecvTimeoutError::Disconnected) => break false,
            Err(RecvTimeoutError::Timeout) => {}
        }
    };
    if listening {
        let _ = unsafe { uia.RemoveFocusChangedEventHandler(&handler) };
    }
    if finished {
        let check = record.finish(app);
        tracing::info!(app = %check.app_id, seen = check.seen.len(), "field check: done");
        let _ = events.send(PlatformEvent::FieldsChecked(check));
    }
}

/// A check of one app on a thread of its own; dropping it stops it without a result.
struct Checker {
    stop: Option<Sender<CheckMsg>>,
    thread: Option<JoinHandle<()>>,
}

impl Checker {
    fn start(app: String, events: Sender<PlatformEvent>) -> Checker {
        let (tx, rx) = mpsc::channel();
        let reports = tx.clone();
        let thread = thread::Builder::new()
            .name("field-check".into())
            .spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(uia) => check_app(&uia, app, events, reports, rx),
                    Err(e) => tracing::warn!(error = %e, "UI Automation is not available"),
                }
                CoUninitialize();
            })
            .ok();
        Checker { stop: Some(tx), thread }
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        // Told, not just dropped: the focus handler holds a sender of the same channel.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(CheckMsg::Stop);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The watching thread; dropping it stops it.
pub struct Watcher {
    stop: Option<Sender<Msg>>,
    thread: Option<JoinHandle<()>>,
    /// Where a check of an app sends what it saw.
    events: Sender<PlatformEvent>,
    checker: Option<Checker>,
}

impl Watcher {
    pub fn start(trigger: BesideFieldTrigger, events: Sender<PlatformEvent>) -> Watcher {
        let for_checks = events.clone();
        let (tx, rx) = mpsc::channel();
        let reports = tx.clone();
        let thread = thread::Builder::new()
            .name("beside-field".into())
            .spawn(move || unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(uia) => match trigger {
                        BesideFieldTrigger::Focus => watch_focus(&uia, events, reports, rx),
                        BesideFieldTrigger::Hover => watch_hover(&uia, events, rx),
                    },
                    Err(e) => tracing::warn!(error = %e, "UI Automation is not available"),
                }
                CoUninitialize();
            })
            .ok();
        Watcher { stop: Some(tx), thread, events: for_checks, checker: None }
    }

    /// Watches `app` for `CHECK_FOR` on a thread of its own, listening for its focus reports
    /// whatever the trigger (plan C1). A check still running is dropped without a result.
    pub fn check(&mut self, app: String) {
        self.checker = None;
        self.checker = Some(Checker::start(app, self.events.clone()));
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.checker = None;
        // Told, not just dropped: the focus handler holds a sender of the same channel.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(Msg::Stop);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_takes_typed_text() {
        // Measured on the development machine (Chrome, Windows Terminal).
        assert!(takes_text(UIA_EditControlTypeId, "OmniboxViewViews", true, true, false));
        assert!(takes_text(UIA_ComboBoxControlTypeId, "truncate", true, true, false), "Google's search box");
        assert!(takes_text(UIA_TextControlTypeId, TERMINAL_CLASS, true, true, false), "Windows Terminal");
        assert!(!takes_text(UIA_DocumentControlTypeId, "", true, true, true), "a web page itself is read-only");
        assert!(!takes_text(UIA_ButtonControlTypeId, "card", false, true, false));
        assert!(!takes_text(UIA_WindowControlTypeId, "CASCADIA_HOSTING_WINDOW_CLASS", false, true, false));
        // A drop-down list has no text pattern; other text is a label.
        assert!(!takes_text(UIA_ComboBoxControlTypeId, "", false, true, false));
        assert!(!takes_text(UIA_TextControlTypeId, "", true, true, false));
        assert!(!takes_text(UIA_EditControlTypeId, "", true, false, false), "not focusable");
        assert!(!takes_text(UIA_ComboBoxControlTypeId, "", true, true, true), "read-only");
    }

    #[test]
    fn a_windows_report_waits_for_its_text_field() {
        // Measured on LINE (Qt): clicking into its text field from another app reports only the
        // window; the focused element is a message at 0 and 100 ms, and the text field by 300 ms.
        let window = FieldProbe { is_text_field: false, app_id: Some("line.exe".into()), ..FieldProbe::default() };
        assert!(waits_for_field(true, &window));
        let t0 = Instant::now();
        let until = t0 + RECHECK_FOR;
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert_eq!(after_look(Refocused::NotYet, at(100), until), Err(true));
        assert_eq!(after_look(Refocused::NotYet, at(200), until), Err(true));
        let field = FieldProbe { is_text_field: true, ..window.clone() };
        assert_eq!(after_look(Refocused::Field(field.clone()), at(300), until), Ok(field.clone()));
        // Never a text field: given up, before the mic's wait to go home is over.
        assert_eq!(after_look(Refocused::NotYet, until, until), Err(false));
        // Another app took the focus: its own report tells.
        assert_eq!(after_look(Refocused::Elsewhere, at(100), until), Err(false));
        // A text field's own report, or anything but a window, goes on as it is.
        assert!(!waits_for_field(true, &field));
        assert!(!waits_for_field(false, &window));
    }

    #[test]
    fn a_check_names_control_types_the_way_ui_automation_does() {
        use windows::Win32::UI::Accessibility::{UIA_AppBarControlTypeId, UIA_ListItemControlTypeId};
        assert_eq!(kind_name(UIA_ButtonControlTypeId), "Button");
        assert_eq!(kind_name(UIA_EditControlTypeId), "Edit");
        assert_eq!(kind_name(UIA_ListItemControlTypeId), "ListItem");
        assert_eq!(kind_name(UIA_TextControlTypeId), "Text");
        assert_eq!(kind_name(UIA_DocumentControlTypeId), "Document");
        assert_eq!(kind_name(UIA_WindowControlTypeId), "Window");
        assert_eq!(kind_name(UIA_AppBarControlTypeId), "AppBar");
        // Unknown ones by number.
        assert_eq!(kind_name(UIA_CONTROLTYPE_ID(0)), "0");
        assert_eq!(kind_name(UIA_CONTROLTYPE_ID(50041)), "50041");
    }
}
