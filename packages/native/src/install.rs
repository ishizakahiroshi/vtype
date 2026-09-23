//! `vtype install` / `vtype uninstall`: start vtype with the OS (and, on GNOME, register its
//! global shortcut). There is nothing to register with Chrome: vtype starts a Chrome of its own
//! for the speech page (standalone plan C4).

use anyhow::Result;

use crate::platform::PlatformError;

/// True for an executable inside the Windows Store's package folder.
#[cfg(any(windows, test))]
pub fn is_msix_install(exe: &std::path::Path) -> bool {
    // Split by hand rather than with Path::components, so the test runs the same on every OS.
    let text = exe.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    text.split('\\').any(|part| part == "windowsapps") && !text.contains(r"\microsoft\windowsapps\")
}

pub fn install() -> Result<()> {
    let platform = crate::platform::current();
    match platform.set_autostart(true) {
        Ok(()) => println!("vtype starts with the system"),
        Err(PlatformError::Unsupported) => {}
        Err(e) => eprintln!("could not set up starting with the system: {e}"),
    }
    after_install_hooks();
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let platform = crate::platform::current();
    match platform.set_autostart(false) {
        Ok(()) | Err(PlatformError::Unsupported) => {}
        Err(e) => eprintln!("could not stop starting with the system: {e}"),
    }
    before_uninstall_hooks();
    let _ = crate::ipc::request_at(&crate::paths::ipc_endpoint(), &crate::protocol::Request::Quit);
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
    use std::path::Path;

    #[test]
    fn the_deb_ships_the_autostart_entry() {
        assert_eq!(
            include_str!("../packaging/deb/vtype.desktop").replace("\r\n", "\n"),
            crate::linux_setup::autostart_entry(Path::new("/usr/bin/vtype"))
        );
    }

    #[test]
    fn recognizes_an_msix_install() {
        let packaged = Path::new(r"X:\Program Files\WindowsApps\ishizakahiroshi.vtype_0.1.0.0_x64__abc\vtype.exe");
        assert!(is_msix_install(packaged));
        assert!(!is_msix_install(Path::new(r"X:\profile\bin\vtype.exe")));
        // The app execution alias is not the package folder.
        assert!(!is_msix_install(Path::new(r"X:\profile\AppData\Local\Microsoft\WindowsApps\vtype.exe")));
    }
}
