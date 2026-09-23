//! `config.json`: the desktop app's own settings, including the input mode and the replacement
//! table since the desktop app does its own recognition (standalone plan C5).
//!
//! The JSON uses camelCase: the speech page and the settings page read the same object.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::overlay_logic::{clamp_scale, SCALE_DEFAULT};
use crate::protocol::InputMode;

/// Same limit as vtype-core's `MAX_REPLACEMENT_RULES`.
pub const MAX_REPLACEMENT_RULES: usize = 200;

/// One entry of the replacement table: `from` is replaced by `to` in what is recognised.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementRule {
    pub from: String,
    pub to: String,
}

/// vtype-core's `normalizeReplacementRules`: entries that are not two strings and entries with an
/// empty `from` are dropped, a `from` seen before (A-Z compared without case) is dropped, and the
/// table stops at `MAX_REPLACEMENT_RULES`. Spaces are kept.
pub fn normalize_replacements(raw: &Value) -> Vec<ReplacementRule> {
    let mut out: Vec<ReplacementRule> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in raw.as_array().map(Vec::as_slice).unwrap_or_default() {
        if out.len() >= MAX_REPLACEMENT_RULES {
            break;
        }
        let (Some(from), Some(to)) = (item.get("from").and_then(Value::as_str), item.get("to").and_then(Value::as_str))
        else {
            continue;
        };
        if from.is_empty() || !seen.insert(from.to_ascii_lowercase()) {
            continue;
        }
        out.push(ReplacementRule { from: from.to_string(), to: to.to_string() });
    }
    out
}

/// A broken table (or broken entries) must not make the whole file unreadable.
fn lenient_replacements<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ReplacementRule>, D::Error> {
    Ok(normalize_replacements(&Value::deserialize(d)?))
}

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
    /// The user agreed, on the speech page, that their voice goes to Google through Chrome's
    /// speech recognition (standalone plan S7). Written only once true, so the extension's view
    /// of this object (and the shared message fixture) is unchanged until then.
    #[serde(skip_serializing_if = "is_false")]
    pub consented: bool,
    /// The input mode the recording starts in (standalone plan C5; the extension kept it before).
    pub input_mode: InputMode,
    /// Words the recognition gets wrong, and what to write instead.
    #[serde(deserialize_with = "lenient_replacements")]
    pub replacements: Vec<ReplacementRule>,
    /// What the floating mic's send button presses.
    pub send_key: SendKey,
    /// Stop the recording this many seconds after the last new words (0 = off). Nothing is sent:
    /// only the send button sends.
    #[serde(deserialize_with = "lenient_silence_stop")]
    pub silence_stop_sec: u8,
    /// Texts the user saved to put in with one click (the floating mic's top-left button).
    #[serde(deserialize_with = "lenient_templates")]
    pub templates: Vec<String>,
    /// Press the send key right after a template went in.
    pub template_send_immediate: bool,
}

/// At most this many templates, as the many-ai-cli dashboard keeps.
pub const MAX_TEMPLATES: usize = 100;
/// Characters per template.
pub const MAX_TEMPLATE_CHARS: usize = 8000;

/// Trimmed, non-empty, cut to `MAX_TEMPLATE_CHARS`, no duplicates, at most `MAX_TEMPLATES`.
pub fn normalize_templates(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let text: String = item.trim().chars().take(MAX_TEMPLATE_CHARS).collect();
        let text = text.trim_end().to_string();
        if !text.is_empty() && !out.contains(&text) && out.len() < MAX_TEMPLATES {
            out.push(text);
        }
    }
    out
}

/// A broken list (or broken entries) must not make the whole file unreadable.
fn lenient_templates<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let value = Value::deserialize(d)?;
    let items = value.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>());
    Ok(normalize_templates(items.unwrap_or_default()))
}

/// The floating mic's send button: Enter, or Ctrl+Enter (⌘+Enter on macOS) for apps where Enter
/// starts a new line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendKey {
    #[default]
    Enter,
    CtrlEnter,
}

/// "Stop after you finish speaking", in seconds: 0 is off, at most `SILENCE_STOP_MAX`. The same
/// range as vtype-core's Whisper auto-stop.
pub const SILENCE_STOP_DEFAULT: u8 = 3;
pub const SILENCE_STOP_MAX: u8 = 10;

/// Out of range is pulled into it (below 0 is off); not a number is the default.
fn lenient_silence_stop<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value.as_f64().map_or(SILENCE_STOP_DEFAULT, |s| (s.round() as i64).clamp(0, SILENCE_STOP_MAX.into()) as u8))
}

fn is_false(v: &bool) -> bool {
    !v
}

impl Default for NativeConfig {
    fn default() -> Self {
        NativeConfig {
            hotkey: None,
            icon: IconConfig::default(),
            inject: InjectMethod::Auto,
            beside_field: BesideFieldConfig::default(),
            extra_extension_ids: Vec::new(),
            consented: false,
            input_mode: InputMode::Normal,
            replacements: Vec::new(),
            send_key: SendKey::Enter,
            silence_stop_sec: SILENCE_STOP_DEFAULT,
            templates: Vec::new(),
            template_send_immediate: false,
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
    /// The floating mic's size in percent (`overlay_logic::SCALE_MIN..=SCALE_MAX`).
    #[serde(deserialize_with = "lenient_scale")]
    pub scale: u16,
}

impl Default for IconConfig {
    fn default() -> Self {
        IconConfig { visible: true, x: None, y: None, hide_on_fullscreen: true, scale: SCALE_DEFAULT }
    }
}

/// Out of range is pulled into it; not a number is the default.
fn lenient_scale<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value.as_f64().map_or(SCALE_DEFAULT, |p| clamp_scale(p.round() as i64)))
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
        assert!(!cfg.consented);
        assert_eq!(cfg.effective_hotkey(), default_hotkey());
        assert_eq!(cfg.send_key, SendKey::Enter);
        assert_eq!(cfg.silence_stop_sec, 3);
        assert!(cfg.templates.is_empty());
        assert!(!cfg.template_send_immediate);
    }

    #[test]
    fn the_seconds_before_stopping_after_speech_stay_in_range_and_old_files_read_as_3() {
        let old: NativeConfig = serde_json::from_str(r#"{"inject":"auto"}"#).unwrap();
        assert_eq!(old.silence_stop_sec, SILENCE_STOP_DEFAULT);
        let read = |json: &str| serde_json::from_str::<NativeConfig>(json).unwrap().silence_stop_sec;
        assert_eq!(read(r#"{"silenceStopSec":0}"#), 0);
        assert_eq!(read(r#"{"silenceStopSec":7}"#), 7);
        assert_eq!(read(r#"{"silenceStopSec":1.6}"#), 2);
        assert_eq!(read(r#"{"silenceStopSec":-4}"#), 0);
        assert_eq!(read(r#"{"silenceStopSec":90}"#), SILENCE_STOP_MAX);
        assert_eq!(read(r#"{"silenceStopSec":"soon"}"#), SILENCE_STOP_DEFAULT);
        assert_eq!(read(r#"{"silenceStopSec":null}"#), SILENCE_STOP_DEFAULT);
        assert_eq!(serde_json::to_value(NativeConfig::default()).unwrap()["silenceStopSec"], 3);
    }

    #[test]
    fn templates_are_tidied_and_a_broken_list_is_dropped_not_fatal() {
        let many = (0..150).map(|i| format!("t{i}"));
        assert_eq!(normalize_templates(many).len(), MAX_TEMPLATES);
        let long = "あ".repeat(MAX_TEMPLATE_CHARS + 10);
        assert_eq!(normalize_templates([long])[0].chars().count(), MAX_TEMPLATE_CHARS);
        assert_eq!(
            normalize_templates(["  hi \n".into(), "".into(), "hi".into(), "line1\nline2".into()]),
            vec!["hi".to_string(), "line1\nline2".to_string()]
        );
        let cfg: NativeConfig =
            serde_json::from_str(r#"{"templates": ["a", 3, null, " a ", "b"], "templateSendImmediate": true}"#)
                .unwrap();
        assert_eq!(cfg.templates, vec!["a".to_string(), "b".to_string()]);
        assert!(cfg.template_send_immediate);
        let broken: NativeConfig = serde_json::from_str(r#"{"templates": "oops"}"#).unwrap();
        assert!(broken.templates.is_empty());
    }

    #[test]
    fn the_mic_size_is_kept_in_range_and_old_files_read_as_100() {
        let old: NativeConfig = serde_json::from_str(r#"{"icon":{"visible":true}}"#).unwrap();
        assert_eq!(old.icon.scale, 100);
        let read = |json: &str| serde_json::from_str::<NativeConfig>(json).unwrap().icon.scale;
        assert_eq!(read(r#"{"icon":{"scale":150}}"#), 150);
        assert_eq!(read(r#"{"icon":{"scale":10}}"#), 50);
        assert_eq!(read(r#"{"icon":{"scale":900}}"#), 500);
        assert_eq!(read(r#"{"icon":{"scale":"big"}}"#), 100);
        assert_eq!(serde_json::to_value(NativeConfig::default()).unwrap()["icon"]["scale"], 100);
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");
        let mut cfg = NativeConfig { hotkey: Some("Ctrl+Shift+F9".into()), ..NativeConfig::default() };
        cfg.icon.x = Some(100);
        cfg.icon.y = Some(-20);
        cfg.icon.scale = 130;
        cfg.inject = InjectMethod::Paste;
        cfg.beside_field.trigger = BesideFieldTrigger::Hover;
        cfg.extra_extension_ids = vec!["a".repeat(32)];
        cfg.silence_stop_sec = 0;
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
    fn consent_is_written_only_once_given_and_old_files_read_as_not_given() {
        let old: NativeConfig = serde_json::from_str(r#"{"inject":"auto","icon":{"visible":true}}"#).unwrap();
        assert!(!old.consented);
        assert!(serde_json::to_value(NativeConfig::default()).unwrap().get("consented").is_none());
        let given = NativeConfig { consented: true, ..NativeConfig::default() };
        assert_eq!(serde_json::to_value(&given).unwrap()["consented"], true);
    }

    #[test]
    fn the_mode_and_the_table_are_kept_and_old_files_read() {
        let old: NativeConfig = serde_json::from_str(r#"{"inject":"auto"}"#).unwrap();
        assert_eq!(old.input_mode, InputMode::Normal);
        assert!(old.replacements.is_empty());
        let cfg: NativeConfig = serde_json::from_str(
            r#"{"inputMode":"kana","replacements":[{"from":"ブイタイプ","to":"vtype"},{"from":1,"to":"x"}]}"#,
        )
        .unwrap();
        assert_eq!(cfg.input_mode, InputMode::Kana);
        assert_eq!(cfg.replacements, vec![ReplacementRule { from: "ブイタイプ".into(), to: "vtype".into() }]);
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["inputMode"], "kana");
        assert_eq!(json["replacements"][0]["to"], "vtype");
    }

    #[test]
    fn the_table_is_normalized_like_vtype_core() {
        let mut raw: Vec<Value> = vec![
            serde_json::json!({"from":"","to":"empty"}),
            serde_json::json!({"from":"GitHub","to":"first"}),
            serde_json::json!({"from":"github","to":"dropped"}),
            serde_json::json!({"from":" a ","to":" kept "}),
            serde_json::json!("not an object"),
        ];
        for i in 0..300 {
            raw.push(serde_json::json!({"from": format!("w{i}"), "to": "x"}));
        }
        let rules = normalize_replacements(&Value::Array(raw));
        assert_eq!(rules.len(), MAX_REPLACEMENT_RULES);
        assert_eq!(rules[0], ReplacementRule { from: "GitHub".into(), to: "first".into() });
        assert_eq!(rules[1], ReplacementRule { from: " a ".into(), to: " kept ".into() });
        assert_eq!(rules[2].from, "w0");
        assert_eq!(rules.last().unwrap().from, "w197");
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
