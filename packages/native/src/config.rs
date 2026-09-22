//! `config.json`: the desktop app's own settings. The input mode and the replacement table are
//! not here; they belong to the extension, which does the recognition (parent plan D7).
//!
//! The JSON uses camelCase because the extension's options page reads and writes the same object
//! over Native Messaging.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NativeConfig {
    /// Global shortcut, e.g. `Ctrl+Alt+Space`. `None` means the OS default (parent plan D20).
    pub hotkey: Option<String>,
    pub icon: IconConfig,
    pub inject: InjectMethod,
    pub beside_field: BesideFieldConfig,
    /// Extension IDs allowed besides the store build (development builds).
    pub extra_extension_ids: Vec<String>,
}

impl Default for NativeConfig {
    fn default() -> Self {
        NativeConfig {
            hotkey: None,
            icon: IconConfig::default(),
            inject: InjectMethod::Auto,
            beside_field: BesideFieldConfig::default(),
            extra_extension_ids: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IconConfig {
    pub visible: bool,
    /// Saved position (physical pixels, top-left). `None` = bottom-right of the work area.
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub hide_on_fullscreen: bool,
}

impl Default for IconConfig {
    fn default() -> Self {
        IconConfig { visible: true, x: None, y: None, hide_on_fullscreen: true }
    }
}

/// How recognized text reaches the foreground app (parent plan D15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InjectMethod {
    /// Type it, and fall back to pasting if typing did not go through.
    #[default]
    Auto,
    Type,
    Paste,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BesideFieldConfig {
    /// Experimental; off by default (parent plan D18).
    pub enabled: bool,
    pub trigger: BesideFieldTrigger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BesideFieldTrigger {
    #[default]
    Focus,
    Hover,
}

/// Default global shortcut for this OS (parent plan D20).
pub const fn default_hotkey() -> &'static str {
    if cfg!(target_os = "macos") {
        "Ctrl+Alt+V"
    } else {
        "Ctrl+Alt+Space"
    }
}

impl NativeConfig {
    pub fn effective_hotkey(&self) -> &str {
        match self.hotkey.as_deref() {
            Some(h) if !h.trim().is_empty() => h,
            _ => default_hotkey(),
        }
    }
}

/// What `load` found, so the caller can log a broken file without the loader knowing about logs.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    Missing,
    Unreadable,
    /// The file did not parse and was moved to `config.json.bak`.
    BackedUp(PathBuf),
}

/// Reads the file; anything wrong with it yields the defaults. A file that exists but does not
/// parse is moved aside to `<name>.bak` so the next save does not destroy what the user had.
pub fn load(path: &Path) -> (NativeConfig, LoadOutcome) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (NativeConfig::default(), LoadOutcome::Missing),
        Err(_) => return (NativeConfig::default(), LoadOutcome::Unreadable),
    };
    match serde_json::from_str::<NativeConfig>(&text) {
        Ok(cfg) => (cfg, LoadOutcome::Loaded),
        Err(_) => {
            let bak = backup_path(path);
            let _ = fs::remove_file(&bak);
            match fs::rename(path, &bak) {
                Ok(()) => (NativeConfig::default(), LoadOutcome::BackedUp(bak)),
                Err(_) => (NativeConfig::default(), LoadOutcome::Unreadable),
            }
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".bak");
    path.with_file_name(name)
}

/// Writes through a temporary file so a crash never leaves half a config behind.
pub fn save(path: &Path, cfg: &NativeConfig) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(cfg).map_err(io::Error::other)?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtype-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_follow_the_plan() {
        let cfg = NativeConfig::default();
        assert_eq!(cfg.hotkey, None);
        assert!(cfg.icon.visible);
        assert!(cfg.icon.hide_on_fullscreen);
        assert_eq!(cfg.icon.x, None);
        assert_eq!(cfg.inject, InjectMethod::Auto);
        assert!(!cfg.beside_field.enabled);
        assert_eq!(cfg.beside_field.trigger, BesideFieldTrigger::Focus);
        assert!(cfg.extra_extension_ids.is_empty());
        assert_eq!(cfg.effective_hotkey(), default_hotkey());
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");
        let mut cfg = NativeConfig { hotkey: Some("Ctrl+Shift+F9".into()), ..NativeConfig::default() };
        cfg.icon.x = Some(100);
        cfg.icon.y = Some(-20);
        cfg.inject = InjectMethod::Paste;
        cfg.beside_field.trigger = BesideFieldTrigger::Hover;
        cfg.extra_extension_ids = vec!["a".repeat(32)];
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path), (cfg, LoadOutcome::Loaded));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uses_camel_case_json_and_fills_missing_fields() {
        let json = serde_json::to_value(NativeConfig::default()).unwrap();
        assert_eq!(json["icon"]["hideOnFullscreen"], true);
        assert_eq!(json["besideField"]["trigger"], "focus");
        assert_eq!(json["inject"], "auto");
        let partial: NativeConfig = serde_json::from_str(r#"{"inject":"type"}"#).unwrap();
        assert_eq!(partial.inject, InjectMethod::Type);
        assert!(partial.icon.visible);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(load(&dir.join("config.json")), (NativeConfig::default(), LoadOutcome::Missing));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_moved_aside() {
        let dir = temp_dir("broken");
        let path = dir.join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let (cfg, outcome) = load(&path);
        assert_eq!(cfg, NativeConfig::default());
        let bak = dir.join("config.json.bak");
        assert_eq!(outcome, LoadOutcome::BackedUp(bak.clone()));
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&bak).unwrap(), "{ not json");
        let _ = fs::remove_dir_all(&dir);
    }
}
