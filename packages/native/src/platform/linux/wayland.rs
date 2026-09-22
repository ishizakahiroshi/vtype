//! The real routes behind `crate::wayland_inject` (child plan C7-C2): the RemoteDesktop portal,
//! `ydotool`, and the clipboard.
//!
//! The portal asks the user once; with "remember", it hands back a restore token that is kept
//! next to the config file so later sessions start without asking.

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::PersistMode;

use super::system;
use crate::linux_setup::{XK_CONTROL_L, XK_V};
use crate::wayland_inject::WaylandRoutes;

const PASTE_RESTORE_DELAY: Duration = Duration::from_millis(300);

fn token_file() -> PathBuf {
    crate::paths::config_dir().join("portal-restore-token")
}

fn read_token() -> Option<String> {
    std::fs::read_to_string(token_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn save_token(token: &str) {
    let path = token_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, token) {
        tracing::warn!(error = %e, "could not keep the portal's restore token");
    }
}

/// Presses Ctrl+V through the portal.
async fn portal_ctrl_v() -> ashpd::Result<()> {
    let portal = RemoteDesktop::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let token = read_token();
    portal
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(ashpd::enumflags2::BitFlags::from(DeviceType::Keyboard))
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await?;
    let selected = portal.start(&session, None, Default::default()).await?.response()?;
    if let Some(new_token) = selected.restore_token() {
        if token.as_deref() != Some(new_token) {
            save_token(new_token);
        }
    }
    let keys = [
        (XK_CONTROL_L, KeyState::Pressed),
        (XK_V, KeyState::Pressed),
        (XK_V, KeyState::Released),
        (XK_CONTROL_L, KeyState::Released),
    ];
    for (keysym, state) in keys {
        portal.notify_keyboard_keysym(&session, keysym as i32, state, Default::default()).await?;
    }
    session.close().await?;
    Ok(())
}

/// Where ydotoold listens, when it runs.
fn ydotool_socket() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(s) = std::env::var_os("YDOTOOL_SOCKET") {
        candidates.push(s.into());
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(dir).join(".ydotool_socket"));
    }
    candidates.push(PathBuf::from("/tmp/.ydotool_socket"));
    candidates.into_iter().find(|p| p.exists())
}

pub struct Routes;

impl WaylandRoutes for Routes {
    fn portal_paste(&mut self, text: &str) -> Result<(), String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("clipboard: {e}"))?;
        let previous = clipboard.get_text().ok();
        clipboard.set_text(text.to_string()).map_err(|e| format!("clipboard: {e}"))?;
        let pressed = zbus::block_on(portal_ctrl_v()).map_err(|e| e.to_string());
        thread::sleep(PASTE_RESTORE_DELAY);
        // Pasted: bring back what the clipboard held. Not pasted: the next route may use the
        // clipboard, so the text stays on it.
        if let (Ok(()), Some(old)) = (&pressed, previous) {
            let _ = clipboard.set_text(old);
        }
        pressed
    }

    fn ydotool_type(&mut self, text: &str) -> Result<(), String> {
        let path = std::env::var("PATH").unwrap_or_default();
        let ydotool = system::find_in_path(&["ydotool"], &path, |p| p.is_file()).ok_or("not installed")?;
        ydotool_socket().ok_or("ydotoold is not running")?;
        if text.starts_with('-') {
            return Err("text would read as an option".into());
        }
        let status = Command::new(ydotool).arg("type").arg(text).status().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("exited with {status}"))
        }
    }

    fn copy(&mut self, text: &str) -> Result<(), String> {
        system::copy_to_clipboard(text).map_err(|e| e.to_string())
    }
}
