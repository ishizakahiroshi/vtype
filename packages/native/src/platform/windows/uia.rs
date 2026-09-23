//! What the focused element of the foreground app is, through UI Automation (child plan C5-C2):
//! whether it is a password field, where it is, and which program it belongs to.
//!
//! COM is initialized (multithreaded) on whichever thread asks, once; the daemon asks from its one
//! worker thread.

use std::cell::RefCell;

use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationElement};

use crate::platform::{FieldInfo, Rect};

thread_local! {
    static AUTOMATION: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

fn automation() -> Option<IUIAutomation> {
    AUTOMATION.with(|slot| {
        if slot.borrow().is_none() {
            unsafe {
                // S_FALSE (already initialized) is fine; a different apartment is not ours to fix.
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(a) => *slot.borrow_mut() = Some(a),
                    Err(e) => tracing::warn!(error = %e, "UI Automation is not available"),
                }
            }
        }
        slot.borrow().clone()
    })
}

pub fn focused_element() -> Option<IUIAutomationElement> {
    let uia = automation()?;
    unsafe { uia.GetFocusedElement().ok() }
}

pub fn describe(element: &IUIAutomationElement) -> FieldInfo {
    unsafe {
        let is_password = element.CurrentIsPassword().ok().map(|b| b.as_bool());
        let caret_rect = element.CurrentBoundingRectangle().ok().map(|r| Rect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        });
        let app_id = element.CurrentProcessId().ok().and_then(|pid| super::system_process_name(pid as u32));
        // The same test as the beside mic's: an editable edit or document control.
        let is_text_field = Some(super::beside::probe(element, false).is_text_field);
        FieldInfo { is_password, caret_rect, app_id, is_text_field }
    }
}

/// The element of a given window (for checks that must not depend on who has the focus).
pub fn element_for_window(hwnd: isize) -> Option<IUIAutomationElement> {
    let uia = automation()?;
    unsafe { uia.ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as *mut _)).ok() }
}

pub fn window_field(hwnd: isize) -> FieldInfo {
    element_for_window(hwnd).map(|e| describe(&e)).unwrap_or_default()
}

pub fn focused_field() -> FieldInfo {
    focused_element().map(|e| describe(&e)).unwrap_or_default()
}
