//! The Accessibility API (child plan C6-C2): whether vtype may send keystrokes, and what the
//! focused element of the front app is (a password field, where it is, whose it is).
//!
//! Safe to call from any thread. Without the permission every question answers "unknown".

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_app_kit::NSRunningApplication;
use objc2_application_services::{
    kAXTrustedCheckOptionPrompt, AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElement, AXValue,
    AXValueType,
};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGPoint, CGSize};

use crate::platform::{FieldInfo, Rect};

/// The subrole AppKit (and Chrome) give a password field.
const SECURE_TEXT_FIELD: &str = "AXSecureTextField";
/// How long one question may keep us waiting on a busy app.
const MESSAGING_TIMEOUT_SECONDS: f32 = 0.5;

/// Whether vtype is allowed to control the computer. With `prompt`, macOS shows its own dialog
/// that leads to System Settings (only when not allowed yet; it does not change the answer).
pub fn trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrusted();
        }
        let key: &CFString = kAXTrustedCheckOptionPrompt;
        let options = CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
        AXIsProcessTrustedWithOptions(Some(options.as_opaque()))
    }
}

pub(super) fn attribute(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let name = CFString::from_static_str(name);
    let mut value: *const CFType = std::ptr::null();
    let err = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut value)) };
    if err != AXError::Success || value.is_null() {
        return None;
    }
    // Copy… returns +1; hand it to CFRetained to release.
    NonNull::new(value as *mut CFType).map(|p| unsafe { CFRetained::from_raw(p) })
}

pub(super) fn point_or_size<T: Default>(element: &AXUIElement, name: &'static str, kind: AXValueType) -> Option<T> {
    let value = attribute(element, name)?.downcast::<AXValue>().ok()?;
    let mut out = T::default();
    let ok = unsafe { value.value(kind, NonNull::from(&mut out).cast::<c_void>()) };
    ok.then_some(out)
}

pub(super) fn bundle_id(element: &AXUIElement) -> Option<String> {
    let mut pid: i32 = 0;
    let err = unsafe { element.pid(NonNull::from(&mut pid)) };
    if err != AXError::Success || pid <= 0 {
        return None;
    }
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    app.bundleIdentifier().map(|s| s.to_string())
}

/// The focused element of whichever app is in front.
pub fn focused_field() -> FieldInfo {
    unsafe {
        let system = AXUIElement::new_system_wide();
        system.set_messaging_timeout(MESSAGING_TIMEOUT_SECONDS);
        let Some(focused) = attribute(&system, "AXFocusedUIElement").and_then(|v| v.downcast::<AXUIElement>().ok())
        else {
            return FieldInfo::default();
        };
        let subrole =
            attribute(&focused, "AXSubrole").and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string());
        let role = attribute(&focused, "AXRole").and_then(|v| v.downcast::<CFString>().ok()).map(|s| s.to_string());
        let is_password = match (&subrole, &role) {
            (Some(s), _) if s == SECURE_TEXT_FIELD => Some(true),
            (None, None) => None,
            _ => Some(false),
        };
        let position: Option<CGPoint> = point_or_size(&focused, "AXPosition", AXValueType::CGPoint);
        let size: Option<CGSize> = point_or_size(&focused, "AXSize", AXValueType::CGSize);
        // AX speaks in points from the top-left of the main screen, as the overlay does.
        let caret_rect = match (position, size) {
            (Some(p), Some(s)) => Some(Rect {
                x: p.x.round() as i32,
                y: p.y.round() as i32,
                width: s.width.round() as i32,
                height: s.height.round() as i32,
            }),
            _ => None,
        };
        let is_text_field = role.as_deref().map(|r| super::beside::TEXT_ROLES.contains(&r));
        FieldInfo { is_password, caret_rect, app_id: bundle_id(&focused), is_text_field }
    }
}
