//! Whether the front app covers a whole screen (a full-screen video, a game, a presentation), for
//! "hide over full-screen apps" (child plan C6-C3). When it cannot tell, it says no, and the mic
//! stays.

use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::{CFDictionary, CFNumber, CFString, CFType, CGRect};
use objc2_core_graphics::{
    kCGNullWindowID, kCGWindowBounds, kCGWindowLayer, kCGWindowOwnerPID, CGRectMakeWithDictionaryRepresentation,
    CGWindowListCopyWindowInfo, CGWindowListOption,
};

use crate::platform::Rect;

fn number(dict: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<i64> {
    dict.get(key)?.downcast::<CFNumber>().ok()?.as_i64()
}

/// `screens` are whole screen frames, in points from the top-left of the main screen (the same
/// space CGWindow bounds use).
pub fn front_app_covers_a_screen(screens: &[Rect]) -> bool {
    let Some(front) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return false;
    };
    let pid = front.processIdentifier() as i64;
    if pid == std::process::id() as i64 {
        return false;
    }
    let options = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return false;
    };
    let list = unsafe { list.cast_unchecked::<CFDictionary<CFString, CFType>>() };
    let (owner_key, layer_key, bounds_key) = unsafe { (kCGWindowOwnerPID, kCGWindowLayer, kCGWindowBounds) };
    list.iter().any(|window| {
        if number(&window, owner_key) != Some(pid) || number(&window, layer_key) != Some(0) {
            return false;
        }
        let Some(bounds) = window.get(bounds_key).and_then(|b| b.downcast::<CFDictionary>().ok()) else {
            return false;
        };
        let mut rect = CGRect::default();
        if !unsafe { CGRectMakeWithDictionaryRepresentation(Some(&bounds), &mut rect) } {
            return false;
        }
        let r = Rect {
            x: rect.origin.x.round() as i32,
            y: rect.origin.y.round() as i32,
            width: rect.size.width.round() as i32,
            height: rect.size.height.round() as i32,
        };
        screens.contains(&r)
    })
}
