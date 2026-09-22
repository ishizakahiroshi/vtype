//! Windows services that need no window: starting with Windows, finding and starting Chrome,
//! notifications, the clipboard, and what the OS is.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::platform::PlatformError;
use crate::win_registry::{self, Hive};

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "vtype";
const CHROME_APP_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe";

/// The Run value that starts the daemon at sign-in.
pub fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" daemon", exe.display())
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let exe = std::env::current_exe().map_err(PlatformError::failed)?;
    // An MSIX install starts through its StartupTask (the package manifest); the Run key would
    // point into a folder that changes with every update.
    if crate::install::is_msix_install(&exe) {
        return Err(PlatformError::Unsupported);
    }
    if enabled {
        win_registry::set_string(Hive::CurrentUser, RUN_KEY, Some(RUN_VALUE), &autostart_command(&exe))
            .map_err(PlatformError::failed)
    } else {
        win_registry::delete_value(Hive::CurrentUser, RUN_KEY, RUN_VALUE).map_err(PlatformError::failed)
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
    Command::new(chrome).args(args).spawn().map(|_| ()).map_err(PlatformError::failed)
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
    fn tells_windows_11_from_10_by_the_build() {
        assert_eq!(describe_windows(Some("26200"), Some("25H2")), "Windows 11 (build 26200, 25H2)");
        assert_eq!(describe_windows(Some("19045"), None), "Windows 10 (build 19045)");
        assert_eq!(describe_windows(None, None), "Windows");
    }
}
