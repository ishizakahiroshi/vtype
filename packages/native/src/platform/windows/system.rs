//! Windows services that need no window: starting with Windows, finding and starting Chrome,
//! notifications, the clipboard, and what the OS is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::platform::{Autostart, PlatformError};
use crate::win_registry::{self, Hive};

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "vtype";
const CHROME_APP_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe";

/// The Run value that starts the daemon at sign-in.
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" daemon", exe.display())
}

/// The Run value, rewritten to start `exe` when it starts another copy of vtype (moved, or another
/// build); `None` when it starts `exe` already. Windows paths are compared without case.
pub fn repointed_run_value(value: &str, exe: &Path) -> Option<String> {
    let wanted = autostart_command(exe);
    (!value.eq_ignore_ascii_case(&wanted)).then_some(wanted)
}

pub fn autostart() -> Result<Autostart, PlatformError> {
    let exe = std::env::current_exe().map_err(PlatformError::failed)?;
    if crate::install::is_msix_install(&exe) {
        return super::startup_task::autostart();
    }
    let value = win_registry::get_string(Hive::CurrentUser, RUN_KEY, Some(RUN_VALUE)).map_err(PlatformError::failed)?;
    Ok(Autostart { enabled: value.is_some(), can_change: true })
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let exe = std::env::current_exe().map_err(PlatformError::failed)?;
    // An MSIX install starts through its StartupTask (the package manifest); the Run key would
    // point into a folder that changes with every update.
    if crate::install::is_msix_install(&exe) {
        return super::startup_task::set_autostart(enabled);
    }
    if enabled {
        win_registry::set_string(Hive::CurrentUser, RUN_KEY, Some(RUN_VALUE), &autostart_command(&exe))
            .map_err(PlatformError::failed)
    } else {
        win_registry::delete_value(Hive::CurrentUser, RUN_KEY, RUN_VALUE).map_err(PlatformError::failed)
    }
}

/// The Run value starting another copy of vtype now starts this one. (The Store version's
/// StartupTask always starts the package's own.)
pub fn autostart_here() {
    let Ok(exe) = std::env::current_exe() else { return };
    if crate::install::is_msix_install(&exe) {
        return;
    }
    let Ok(Some(value)) = win_registry::get_string(Hive::CurrentUser, RUN_KEY, Some(RUN_VALUE)) else { return };
    if let Some(value) = repointed_run_value(&value, &exe) {
        match win_registry::set_string(Hive::CurrentUser, RUN_KEY, Some(RUN_VALUE), &value) {
            Ok(()) => tracing::info!("starting at sign-in now starts this copy of vtype"),
            Err(e) => tracing::warn!(error = %e, "could not point starting at sign-in here"),
        }
    }
}

/// Where chrome.exe is: the App Paths registration (per user, then per machine), then the usual
/// install folders.
pub fn find_chrome() -> Option<PathBuf> {
    for hive in [Hive::CurrentUser, Hive::LocalMachine] {
        if let Ok(Some(path)) = win_registry::get_string(hive, CHROME_APP_PATH, None) {
            let path = PathBuf::from(path.trim_matches('"'));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let tail = Path::new(r"Google\Chrome\Application\chrome.exe");
    ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join(tail))
        .find(|p| p.is_file())
}

pub fn launch_chrome(args: &[String]) -> Result<(), PlatformError> {
    if std::env::var_os("VTYPE_TEST_NO_CHROME").is_some() {
        tracing::info!("VTYPE_TEST_NO_CHROME is set; not starting Chrome");
        return Ok(());
    }
    let chrome = find_chrome().ok_or_else(|| PlatformError::Failed("Chrome was not found".into()))?;
    // A window started by a process that is not in front opens behind the others, and the
    // first-run window has to be seen. vtype may hand its foreground right on (it has it right
    // after the shortcut or the tray); without that right the call does nothing.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        );
    }
    Command::new(chrome).args(args).spawn().map_err(PlatformError::failed)?;
    if args.iter().any(|a| a == crate::chrome_launch::OFF_SCREEN_POSITION) {
        thread::spawn(keep_off_screen_chrome_off_taskbar);
    }
    Ok(())
}

/// How long to look for the off-screen window after starting Chrome (a cold start is slow).
const OFF_SCREEN_WAIT: Duration = Duration::from_secs(20);
const OFF_SCREEN_POLL: Duration = Duration::from_millis(50);

/// vtype lives in the tray: the off-screen speech page has no business on the taskbar (the
/// requester, 2026-09-23). Once its window shows, it becomes a tool window. A style change only
/// reaches the taskbar across a hide and show; the show does not take the focus.
fn keep_off_screen_chrome_off_taskbar() {
    let until = Instant::now() + OFF_SCREEN_WAIT;
    while Instant::now() < until {
        let found = off_screen_chrome_windows();
        if !found.is_empty() {
            for hwnd in found {
                unsafe {
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let tool = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
                    ShowWindow(hwnd, SW_HIDE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, tool);
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
            return;
        }
        thread::sleep(OFF_SCREEN_POLL);
    }
    tracing::warn!("the off-screen Chrome window did not show; it may stay on the taskbar");
}

/// Chrome's windows that are shown, not minimized, on no monitor and still on the taskbar. Only
/// the speech page is put there (`chrome_launch::OFF_SCREEN_POSITION`); a minimized window is
/// off screen too, which is why `IsIconic` is asked.
fn off_screen_chrome_windows() -> Vec<HWND> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> i32 {
        let found = &mut *(data as *mut Vec<HWND>);
        if IsWindowVisible(hwnd) == 0
            || IsIconic(hwnd) != 0
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0
            || !MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL).is_null()
        {
            return 1;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if String::from_utf16_lossy(&class[..len.max(0) as usize]) != "Chrome_WidgetWin_1" {
            return 1;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if super::system_process_name(pid).as_deref() == Some("chrome.exe") {
            found.push(hwnd);
        }
        1
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut found as *mut Vec<HWND> as LPARAM) };
    found
}

pub fn notify(title: &str, body: &str) {
    if let Err(e) = notify_rust::Notification::new().summary(title).body(body).show() {
        tracing::warn!(error = %e, "notification failed");
    }
}

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    open::that(url).map_err(PlatformError::failed)
}

pub fn copy_to_clipboard(text: &str) -> Result<(), PlatformError> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).map_err(PlatformError::failed)
}

/// e.g. `Windows 11 (build 26200, 25H2)`.
pub fn os_description() -> String {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = win_registry::get_string(Hive::LocalMachine, key, Some("CurrentBuild")).ok().flatten();
    let display = win_registry::get_string(Hive::LocalMachine, key, Some("DisplayVersion")).ok().flatten();
    describe_windows(build.as_deref(), display.as_deref())
}

/// Windows 11 still says "Windows 10" in ProductName; the build number is what tells them apart.
pub fn describe_windows(build: Option<&str>, display_version: Option<&str>) -> String {
    let Some(build) = build else {
        return "Windows".to_string();
    };
    let name = match build.parse::<u32>() {
        Ok(n) if n >= 22000 => "Windows 11",
        Ok(_) => "Windows 10",
        Err(_) => "Windows",
    };
    match display_version {
        Some(v) if !v.is_empty() => format!("{name} (build {build}, {v})"),
        _ => format!("{name} (build {build})"),
    }
}

/// `ja-JP` when the Windows display language is Japanese, else `en`.
pub fn ui_language() -> String {
    let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    // The primary language is the low 10 bits; 0x11 is Japanese.
    if lang & 0x3ff == 0x11 {
        "ja-JP".to_string()
    } else {
        "en".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_runs_the_daemon() {
        assert_eq!(autostart_command(Path::new(r"X:\Apps\vtype\vtype.exe")), r#""X:\Apps\vtype\vtype.exe" daemon"#);
    }

    #[test]
    fn a_run_value_for_another_copy_is_pointed_here() {
        let here = Path::new(r"X:\Apps\vtype\vtype.exe");
        let old = autostart_command(Path::new(r"X:\Old\vtype\vtype.exe"));
        assert_eq!(repointed_run_value(&old, here), Some(autostart_command(here)));
        assert_eq!(repointed_run_value(&autostart_command(here), here), None);
        // Windows paths ignore case.
        assert_eq!(repointed_run_value(r#""x:\apps\VTYPE\vtype.exe" daemon"#, here), None);
    }

    #[test]
    fn tells_windows_11_from_10_by_the_build() {
        assert_eq!(describe_windows(Some("26200"), Some("25H2")), "Windows 11 (build 26200, 25H2)");
        assert_eq!(describe_windows(Some("19045"), None), "Windows 10 (build 19045)");
        assert_eq!(describe_windows(None, None), "Windows");
    }
}
