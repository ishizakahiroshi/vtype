//! macOS services that need no window: the LaunchAgent, starting Chrome, notifications, the
//! clipboard, and what the OS is.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_foundation::NSLocale;

use crate::launch_agent;
use crate::platform::{Autostart, PlatformError};

const CHROME_APP: &str = "Google Chrome";

fn plist_path() -> Result<std::path::PathBuf, PlatformError> {
    directories::BaseDirs::new()
        .map(|d| launch_agent::plist_path(d.home_dir()))
        .ok_or_else(|| PlatformError::Failed("no home folder".into()))
}

pub fn autostart() -> Result<Autostart, PlatformError> {
    Ok(Autostart { enabled: plist_path()?.is_file(), can_change: true })
}

/// The LaunchAgent starting another copy of vtype now starts this one.
pub fn autostart_here() {
    let (Ok(path), Ok(exe)) = (plist_path(), std::env::current_exe()) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    if let Some(text) = launch_agent::repointed_plist(&text, &exe) {
        match std::fs::write(&path, text) {
            Ok(()) => tracing::info!("starting at login now starts this copy of vtype"),
            Err(e) => tracing::warn!(error = %e, "could not point starting at login here"),
        }
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let path = plist_path()?;
    if enabled {
        let exe = std::env::current_exe().map_err(PlatformError::failed)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(PlatformError::failed)?;
        }
        std::fs::write(&path, launch_agent::plist(&exe)).map_err(PlatformError::failed)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PlatformError::failed(e)),
        }
    }
}

/// `open` hands URLs to a running Chrome; the flags only count when `open` starts it, and `-g`
/// keeps a start without windows from taking the focus. A launch with its own `--user-data-dir`
/// (the speech page's Chrome) needs `-n`: without it `open` hands the arguments to the user's
/// running Chrome, which ignores them.
pub fn open_args(args: &[String]) -> Vec<String> {
    let (flags, urls): (Vec<&String>, Vec<&String>) = args.iter().partition(|a| a.starts_with("--"));
    let mut out: Vec<String> = Vec::new();
    if flags.iter().any(|f| f.starts_with("--user-data-dir=")) {
        out.push("-n".into());
    }
    if urls.is_empty() {
        out.push("-g".into());
    }
    out.push("-a".into());
    out.push(CHROME_APP.into());
    out.extend(urls.into_iter().cloned());
    if !flags.is_empty() {
        out.push("--args".into());
        out.extend(flags.into_iter().cloned());
    }
    out
}

pub fn launch_chrome(args: &[String]) -> Result<(), PlatformError> {
    if std::env::var_os("VTYPE_TEST_NO_CHROME").is_some() {
        tracing::info!("VTYPE_TEST_NO_CHROME is set; not starting Chrome");
        return Ok(());
    }
    let status = Command::new("open").args(open_args(args)).status().map_err(PlatformError::failed)?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Failed("Chrome was not found".into()))
    }
}

pub fn notify(title: &str, body: &str) {
    if let Err(e) = notify_rust::Notification::new().summary(title).body(body).show() {
        tracing::warn!(error = %e, "notification failed");
    }
}

static TOLD_ABOUT_ACCESSIBILITY: AtomicBool = AtomicBool::new(false);

/// True the first time in a run: that typing needs the Accessibility permission, and where to
/// give it, is said once.
pub fn first_accessibility_notice() -> bool {
    !TOLD_ABOUT_ACCESSIBILITY.swap(true, Ordering::AcqRel)
}

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    open::that(url).map_err(PlatformError::failed)
}

pub fn copy_to_clipboard(text: &str) -> Result<(), PlatformError> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())).map_err(PlatformError::failed)
}

/// e.g. `macOS 15.1`.
pub fn os_description() -> String {
    let version = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    describe_macos(version.as_deref())
}

pub fn describe_macos(version: Option<&str>) -> String {
    match version {
        Some(v) if !v.is_empty() => format!("macOS {v}"),
        _ => "macOS".to_string(),
    }
}

/// The first of the user's preferred languages, e.g. `ja-JP`.
pub fn ui_language() -> String {
    NSLocale::preferredLanguages()
        .firstObject()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::platform::language_from_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn starts_chrome_in_the_background_with_flags() {
        assert_eq!(
            open_args(&strings(&["--no-startup-window"])),
            strings(&["-g", "-a", "Google Chrome", "--args", "--no-startup-window"])
        );
    }

    #[test]
    fn hands_urls_to_chrome() {
        let url = "chrome-extension://nngfilimeplngdjdmgkddlhbdjpmikgn/options.html";
        assert_eq!(open_args(&strings(&[url])), strings(&["-a", "Google Chrome", url]));
    }

    #[test]
    fn a_profile_of_its_own_starts_a_new_instance() {
        let args = strings(&["--user-data-dir=/tmp/p", "--app=http://127.0.0.1:47213/t/x/speech"]);
        assert_eq!(
            open_args(&args),
            strings(&[
                "-n",
                "-g",
                "-a",
                "Google Chrome",
                "--args",
                "--user-data-dir=/tmp/p",
                "--app=http://127.0.0.1:47213/t/x/speech"
            ])
        );
    }

    #[test]
    fn names_the_version() {
        assert_eq!(describe_macos(Some("15.1")), "macOS 15.1");
        assert_eq!(describe_macos(None), "macOS");
    }
}
