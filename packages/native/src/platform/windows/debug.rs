//! Hidden subcommands for checking the Windows parts on a real desktop (child plan C5):
//! `vtype debug-type`, `vtype debug-field`, `vtype debug-click-icon`, `vtype debug-foreground`.
//! They print what happened and exit; nothing is kept.

use std::ptr::null;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowTextW, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::overlay::{wide, ICON_CLASS};
use crate::config::InjectMethod;

/// Title of the window in front.
pub fn foreground_title() -> String {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
}

/// Types `text` into whatever has the focus, after `delay_ms`.
pub fn debug_type(text: &str, method: InjectMethod, delay_ms: u64, expect_foreground: &str) -> String {
    thread::sleep(Duration::from_millis(delay_ms));
    let front = foreground_title();
    if front != expect_foreground {
        return format!("not typing: the window in front is {front:?}, not {expect_foreground:?}");
    }
    let field = super::uia::focused_field();
    let outcome = super::inject::inject(text, method);
    format!(
        "foreground: {}\nis_password: {:?}\napp: {:?}\noutcome: {:?}",
        foreground_title(),
        field.is_password,
        field.app_id,
        outcome
    )
}

/// What UI Automation says about the focused element.
pub fn debug_field(delay_ms: u64, hwnd: Option<isize>) -> String {
    thread::sleep(Duration::from_millis(delay_ms));
    let field = match hwnd {
        Some(h) => super::uia::window_field(h),
        None => super::uia::focused_field(),
    };
    format!(
        "foreground: {}\nis_password: {:?}\napp: {:?}\nrect: {:?}",
        foreground_title(),
        field.is_password,
        field.app_id,
        field.caret_rect
    )
}

fn mouse(dx: i32, dy: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// Clicks the middle of the floating mic, then reports which window is in front.
pub fn debug_click_icon() -> String {
    unsafe {
        let class = wide(ICON_CLASS);
        let hwnd = FindWindowW(class.as_ptr(), null());
        if hwnd.is_null() {
            return "no mic icon window (is the daemon running with the icon shown?)".into();
        }
        let before = foreground_title();
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let nx = ((cx - vx) * 65535) / (vw - 1).max(1);
        let ny = ((cy - vy) * 65535) / (vh - 1).max(1);
        let abs = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let inputs = [
            mouse(nx, ny, abs | MOUSEEVENTF_MOVE),
            mouse(nx, ny, abs | MOUSEEVENTF_LEFTDOWN),
            mouse(nx, ny, abs | MOUSEEVENTF_LEFTUP),
        ];
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        thread::sleep(Duration::from_millis(400));
        format!(
            "icon: {},{} {}x{}\nforeground before: {before}\nforeground after: {}",
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            foreground_title()
        )
    }
}
