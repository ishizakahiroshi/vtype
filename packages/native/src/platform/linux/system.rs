//! Linux services that need no window: the autostart entry, finding and starting Chrome,
//! notifications (D-Bus), the clipboard, and what the OS is.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::linux_setup::{autostart_on, autostart_path, autostart_user_entry, repointed_entry, SYSTEM_AUTOSTART};
use crate::platform::{Autostart, PlatformError};

/// In the order they are tried.
const CHROME_COMMANDS: [&str; 4] = ["google-chrome-stable", "google-chrome", "chromium", "chromium-browser"];

fn config_dir() -> Result<PathBuf, PlatformError> {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().to_path_buf())
        .ok_or_else(|| PlatformError::Failed("no home folder".into()))
}

/// The user's autostart entry, when there is one.
fn user_entry(path: &Path) -> Result<Option<String>, PlatformError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PlatformError::failed(e)),
    }
}

pub fn autostart() -> Result<Autostart, PlatformError> {
    let entry = user_entry(&autostart_path(&config_dir()?))?;
    Ok(Autostart { enabled: autostart_on(entry.as_deref(), Path::new(SYSTEM_AUTOSTART).is_file()), can_change: true })
}

/// The user's entry decides; with the .deb's entry for everyone, switching off writes one that
/// turns it off for this user.
pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let path = autostart_path(&config_dir()?);
    let exe = std::env::current_exe().map_err(PlatformError::failed)?;
    match autostart_user_entry(enabled, &exe, Path::new(SYSTEM_AUTOSTART).is_file()) {
        Some(entry) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(PlatformError::failed)?;
            }
            std::fs::write(&path, entry).map_err(PlatformError::failed)
        }
        None => match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PlatformError::failed(e)),
        },
    }
}

/// The user's entry starting another copy of vtype now starts this one.
pub fn autostart_here() {
    let (Ok(dir), Ok(exe)) = (config_dir(), std::env::current_exe()) else { return };
    let path = autostart_path(&dir);
    let Ok(Some(entry)) = user_entry(&path) else { return };
    if let Some(entry) = repointed_entry(&entry, &exe) {
        match std::fs::write(&path, entry) {
            Ok(()) => tracing::info!("starting at login now starts this copy of vtype"),
            Err(e) => tracing::warn!(error = %e, "could not point starting at login here"),
        }
    }
}

/// The first of `names` found in the folders of `path_var` (`PATH`'s format).
pub fn find_in_path(names: &[&str], path_var: &str, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|name| std::env::split_paths(path_var).map(|dir| dir.join(name)).find(|candidate| is_file(candidate)))
}

pub fn find_chrome() -> Option<PathBuf> {
    let path = std::env::var("PATH").unwrap_or_default();
    find_in_path(&CHROME_COMMANDS, &path, |p| p.is_file())
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

/// `PRETTY_NAME` from os-release, e.g. `Ubuntu 24.04.1 LTS`.
pub fn os_description() -> String {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    pretty_name(&text).unwrap_or_else(|| "Linux".to_string())
}

pub fn pretty_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|line| {
        let value = line.strip_prefix("PRETTY_NAME=")?.trim();
        let value = value.trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_pretty_name() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("ID=arch\n"), None);
    }

    #[test]
    fn looks_for_chrome_in_order() {
        let path = "/opt/a:/opt/b";
        let found = find_in_path(&CHROME_COMMANDS, path, |p| p == Path::new("/opt/b/chromium"));
        assert_eq!(found, Some(PathBuf::from("/opt/b/chromium")));
        let both = find_in_path(&CHROME_COMMANDS, path, |p| {
            p == Path::new("/opt/b/chromium") || p == Path::new("/opt/a/google-chrome")
        });
        assert_eq!(both, Some(PathBuf::from("/opt/a/google-chrome")));
    }
}
