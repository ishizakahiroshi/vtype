//! `vtype install` / `vtype uninstall`: register this program as Chrome's Native Messaging host
//! `com.ishizakahiroshi.vtype` for the vtype extension, and start it with the OS.
//!
//! Where the registration goes is computed by pure functions (`host_locations`), so every OS's
//! layout is tested on any OS.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::config;
use crate::daemon::STORE_EXTENSION_ID;
use crate::paths::Os;
use crate::platform::PlatformError;

pub const HOST_NAME: &str = "com.ishizakahiroshi.vtype";
const HOST_FILE: &str = "com.ishizakahiroshi.vtype.json";

/// Chrome's own registry key for per-user hosts on Windows.
pub const WINDOWS_REGISTRY_KEY: &str = r"Software\Google\Chrome\NativeMessagingHosts\com.ishizakahiroshi.vtype";

/// A Chrome extension ID: 32 letters from a to p.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// The store build first, then the extra (development) IDs, without duplicates.
pub fn allowed_origins(extra_ids: &[String]) -> Vec<String> {
    let mut ids = vec![STORE_EXTENSION_ID.to_string()];
    for id in extra_ids {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids.into_iter().map(|id| format!("chrome-extension://{id}/")).collect()
}

/// The host manifest Chrome reads.
pub fn host_manifest(exe_path: &str, extra_ids: &[String]) -> Value {
    json!({
        "name": HOST_NAME,
        "description": "vtype desktop: types what the vtype extension recognizes into any app",
        "path": exe_path,
        "type": "stdio",
        "allowed_origins": allowed_origins(extra_ids),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct HostLocations {
    /// Where the manifest file is written.
    pub manifest_files: Vec<PathBuf>,
    /// Windows only: the HKCU key whose default value points at the manifest file.
    pub registry_key: Option<&'static str>,
}

/// Per-user locations. `home` is the user's home, `local_app_data` is %LOCALAPPDATA% (Windows).
pub fn host_locations(os: Os, home: &Path, local_app_data: Option<&Path>) -> HostLocations {
    match os {
        Os::Windows => {
            let base = local_app_data.map(Path::to_path_buf).unwrap_or_else(|| home.join("AppData").join("Local"));
            HostLocations {
                manifest_files: vec![base.join("vtype").join(HOST_FILE)],
                registry_key: Some(WINDOWS_REGISTRY_KEY),
            }
        }
        Os::Mac => HostLocations {
            manifest_files: vec![home
                .join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
                .join(HOST_FILE)],
            registry_key: None,
        },
        Os::Linux => HostLocations {
            manifest_files: vec![
                home.join(".config/google-chrome/NativeMessagingHosts").join(HOST_FILE),
                home.join(".config/chromium/NativeMessagingHosts").join(HOST_FILE),
            ],
            registry_key: None,
        },
    }
}

fn home_dir() -> Result<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).context("could not find the home directory")
}

fn current_locations() -> Result<HostLocations> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    Ok(host_locations(Os::current(), &home_dir()?, local.as_deref()))
}

/// The path Chrome should start. Inside an MSIX package the versioned install folder changes on
/// every update, so the app execution alias is used instead (see `msix_alias_path`).
pub fn host_exe_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not find this program's path")?;
    Ok(msix_alias_path(&exe, std::env::var_os("LOCALAPPDATA").map(PathBuf::from).as_deref()).unwrap_or(exe))
}

/// `…\WindowsApps\<package>\vtype.exe` (an MSIX install) → `%LOCALAPPDATA%\Microsoft\WindowsApps\vtype.exe`.
pub fn msix_alias_path(exe: &Path, local_app_data: Option<&Path>) -> Option<PathBuf> {
    if !is_msix_install(exe) {
        return None;
    }
    local_app_data.map(|l| l.join("Microsoft").join("WindowsApps").join("vtype.exe"))
}

/// True for an executable inside the Windows Store's package folder.
pub fn is_msix_install(exe: &Path) -> bool {
    // Split by hand rather than with Path::components, so the test runs the same on every OS.
    let text = exe.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    text.split('\\').any(|part| part == "windowsapps") && !text.contains(r"\microsoft\windowsapps\")
}

/// Whether Chrome's registration file is missing or points somewhere else than `expected_path`.
pub fn needs_registration(manifest_text: Option<&str>, expected_path: &str) -> bool {
    let recorded = manifest_text
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| v.get("path").and_then(Value::as_str).map(str::to_string));
    recorded.as_deref() != Some(expected_path)
}

/// The Store package has no installer step, so its daemon registers itself with Chrome the first
/// time it runs (and again if the registration went missing or points elsewhere).
pub fn register_if_packaged() {
    let Ok(exe) = std::env::current_exe() else { return };
    if !is_msix_install(&exe) {
        return;
    }
    let (Ok(expected), Ok(locations)) = (host_exe_path(), current_locations()) else { return };
    let current = locations.manifest_files.first().and_then(|f| fs::read_to_string(f).ok());
    if needs_registration(current.as_deref(), &expected.to_string_lossy()) {
        tracing::info!("registering with Chrome (Store package)");
        if let Err(e) = install(&[]) {
            tracing::warn!(error = %format!("{e:#}"), "could not register with Chrome");
        }
    }
}

pub fn install(extra_ids: &[String]) -> Result<()> {
    for id in extra_ids {
        if !is_extension_id(id) {
            bail!("{id} is not an extension ID (32 letters a–p; see chrome://extensions)");
        }
    }
    let config_path = crate::paths::config_file();
    let (mut cfg, _) = config::load(&config_path);
    for id in extra_ids {
        if !cfg.extra_extension_ids.contains(id) && id != STORE_EXTENSION_ID {
            cfg.extra_extension_ids.push(id.clone());
        }
    }
    config::save(&config_path, &cfg).context("could not save config.json")?;

    let exe = host_exe_path()?;
    let manifest = host_manifest(&exe.to_string_lossy(), &cfg.extra_extension_ids);
    let text = serde_json::to_string_pretty(&manifest)?;
    let locations = current_locations()?;
    for file in &locations.manifest_files {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
        }
        fs::write(file, &text).with_context(|| format!("could not write {}", file.display()))?;
        println!("wrote {}", file.display());
    }
    register(&locations)?;

    let platform = crate::platform::current();
    match platform.set_autostart(true) {
        Ok(()) => println!("vtype starts with the system"),
        Err(PlatformError::Unsupported) => {}
        Err(e) => eprintln!("could not set up starting with the system: {e}"),
    }
    after_install_hooks();
    println!("allowed: {}", allowed_origins(&cfg.extra_extension_ids).join(", "));
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let locations = current_locations()?;
    for file in &locations.manifest_files {
        match fs::remove_file(file) {
            Ok(()) => println!("removed {}", file.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("could not remove {}: {e}", file.display()),
        }
    }
    // On Windows the file sits in a folder of vtype's own (`%LOCALAPPDATA%\vtype`); take it out
    // too once it is empty. Elsewhere the folders are Chrome's and stay.
    #[cfg(windows)]
    if let Some(dir) = locations.manifest_files.first().and_then(|f| f.parent()) {
        let _ = fs::remove_dir(dir);
    }
    unregister(&locations)?;
    let platform = crate::platform::current();
    match platform.set_autostart(false) {
        Ok(()) | Err(PlatformError::Unsupported) => {}
        Err(e) => eprintln!("could not stop starting with the system: {e}"),
    }
    before_uninstall_hooks();
    let _ = crate::ipc::request_at(&crate::paths::ipc_endpoint(), &crate::protocol::Request::Quit);
    Ok(())
}

#[cfg(windows)]
fn register(locations: &HostLocations) -> Result<()> {
    use crate::win_registry::{set_string, Hive};
    if let (Some(key), Some(file)) = (locations.registry_key, locations.manifest_files.first()) {
        set_string(Hive::CurrentUser, key, None, &file.to_string_lossy())
            .with_context(|| format!(r"could not write HKCU\{key}"))?;
        println!(r"registered HKCU\{key}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn register(_locations: &HostLocations) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn unregister(locations: &HostLocations) -> Result<()> {
    use crate::win_registry::{delete_tree, Hive};
    if let Some(key) = locations.registry_key {
        delete_tree(Hive::CurrentUser, key).with_context(|| format!(r"could not remove HKCU\{key}"))?;
        println!(r"removed HKCU\{key}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn unregister(_locations: &HostLocations) -> Result<()> {
    Ok(())
}

/// OS-specific extras after registering: GNOME's custom shortcut, the only global shortcut on
/// GNOME's Wayland.
fn after_install_hooks() {
    #[cfg(target_os = "linux")]
    if let Ok(exe) = std::env::current_exe() {
        crate::platform::linux::gnome::install(&exe);
    }
}

fn before_uninstall_hooks() {
    #[cfg(target_os = "linux")]
    crate::platform::linux::gnome::uninstall();
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = "abcdefghijklmnopabcdefghijklmnop";

    #[test]
    fn recognizes_extension_ids() {
        assert!(is_extension_id(STORE_EXTENSION_ID));
        assert!(is_extension_id(DEV));
        assert!(!is_extension_id("abcdefghijklmnopabcdefghijklmnoz"));
        assert!(!is_extension_id("short"));
    }

    #[test]
    fn the_manifest_allows_the_store_build_and_the_extra_ids() {
        let m = host_manifest(r"C:\Apps\vtype.exe", &[DEV.to_string(), DEV.to_string()]);
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["path"], r"C:\Apps\vtype.exe");
        assert_eq!(
            m["allowed_origins"],
            json!([
                "chrome-extension://nngfilimeplngdjdmgkddlhbdjpmikgn/",
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/"
            ])
        );
    }

    #[test]
    fn locations_on_windows() {
        let l = host_locations(Os::Windows, Path::new(r"X:\profile"), Some(Path::new(r"X:\profile\AppData\Local")));
        assert_eq!(l.manifest_files, vec![Path::new(r"X:\profile\AppData\Local").join("vtype").join(HOST_FILE)]);
        assert_eq!(l.registry_key, Some(WINDOWS_REGISTRY_KEY));
        assert!(WINDOWS_REGISTRY_KEY.starts_with(r"Software\Google\Chrome\NativeMessagingHosts\"));
    }

    #[test]
    fn locations_on_macos_and_linux() {
        let home = Path::new("/h");
        let mac = host_locations(Os::Mac, home, None);
        assert_eq!(
            mac.manifest_files,
            vec![home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts").join(HOST_FILE)]
        );
        assert_eq!(mac.registry_key, None);
        let linux = host_locations(Os::Linux, home, None);
        assert_eq!(
            linux.manifest_files,
            vec![
                home.join(".config/google-chrome/NativeMessagingHosts").join(HOST_FILE),
                home.join(".config/chromium/NativeMessagingHosts").join(HOST_FILE),
            ]
        );
    }

    #[test]
    fn the_deb_ships_the_same_registration_and_autostart_entry() {
        let shipped: Value =
            serde_json::from_str(include_str!("../packaging/deb/com.ishizakahiroshi.vtype.json")).unwrap();
        assert_eq!(shipped, host_manifest("/usr/bin/vtype", &[]));
        assert_eq!(
            include_str!("../packaging/deb/vtype.desktop").replace("\r\n", "\n"),
            crate::linux_setup::autostart_entry(Path::new("/usr/bin/vtype"))
        );
    }

    #[test]
    fn registers_again_only_when_the_file_is_missing_or_stale() {
        let alias = r"X:\profile\AppData\Local\Microsoft\WindowsApps\vtype.exe";
        let good = host_manifest(alias, &[]).to_string();
        assert!(!needs_registration(Some(&good), alias));
        assert!(needs_registration(None, alias));
        assert!(needs_registration(Some("not json"), alias));
        let old = host_manifest(r"X:\old\vtype.exe", &[]).to_string();
        assert!(needs_registration(Some(&old), alias));
    }

    #[test]
    fn an_msix_install_points_chrome_at_the_alias() {
        let local = Path::new(r"X:\profile\AppData\Local");
        let packaged = Path::new(r"C:\Program Files\WindowsApps\ishizakahiroshi.vtype_0.1.0.0_x64__abc\vtype.exe");
        assert!(is_msix_install(packaged));
        assert_eq!(
            msix_alias_path(packaged, Some(local)),
            Some(local.join("Microsoft").join("WindowsApps").join("vtype.exe"))
        );
        let plain = Path::new(r"X:\profile\bin\vtype.exe");
        assert!(!is_msix_install(plain));
        assert_eq!(msix_alias_path(plain, Some(local)), None);
        // The alias itself is not the package folder.
        assert!(!is_msix_install(&local.join("Microsoft").join("WindowsApps").join("vtype.exe")));
    }
}
