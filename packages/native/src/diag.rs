//! "Copy diagnostic info" / `vtype diag`: version, OS, settings, connection and recent error
//! codes. Never what was said: no audio, no transcript (parent plan D24).

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::NativeConfig;

pub const MAX_ERRORS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorEntry {
    pub at: String,
    pub code: String,
}

#[derive(Default, Debug)]
pub struct ErrorLog {
    entries: VecDeque<ErrorEntry>,
}

impl ErrorLog {
    pub fn push(&mut self, code: impl Into<String>) {
        self.push_at(utc_now(), code);
    }

    pub fn push_at(&mut self, at: String, code: impl Into<String>) {
        if self.entries.len() == MAX_ERRORS {
            self.entries.pop_front();
        }
        self.entries.push_back(ErrorEntry { at, code: code.into() });
    }

    pub fn entries(&self) -> impl Iterator<Item = &ErrorEntry> {
        self.entries.iter()
    }
}

pub struct Snapshot<'a> {
    pub os: &'a str,
    pub config: &'a NativeConfig,
    pub connected: bool,
    pub extension_version: Option<&'a str>,
    pub browser: Option<&'a str>,
    pub errors: &'a ErrorLog,
    /// Named facts the platform code wants on record (e.g. how far the Wayland fallback got).
    pub notes: &'a [(String, String)],
    /// The last look at an app's text fields (`field_check::report`): kinds of elements, no text.
    pub field_check: Option<&'a Value>,
}

pub fn report(s: &Snapshot<'_>) -> Value {
    let mut config_val = serde_json::to_value(s.config).unwrap_or_else(|_| json!({}));
    if let Some(obj) = config_val.as_object_mut() {
        if obj.contains_key("templates") {
            obj.insert("templatesCount".to_string(), json!(s.config.templates.len()));
            obj.remove("templates");
        }
        if obj.contains_key("replacements") {
            obj.insert("replacementsCount".to_string(), json!(s.config.replacements.len()));
            obj.remove("replacements");
        }
    }
    json!({
        "app": "vtype desktop",
        "version": env!("CARGO_PKG_VERSION"),
        "os": s.os,
        "connected": s.connected,
        "extensionVersion": s.extension_version,
        "browser": s.browser,
        "config": config_val,
        "notes": s.notes.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>(),
        "recentErrors": s.errors.entries().collect::<Vec<_>>(),
        "lastFieldCheck": s.field_check,
    })
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    format_utc(secs)
}

pub fn format_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_790_000_000), "2026-09-21T14:13:20Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn keeps_the_last_twenty_errors() {
        let mut log = ErrorLog::default();
        for i in 0..25 {
            log.push_at(format!("t{i}"), format!("e{i}"));
        }
        let codes: Vec<_> = log.entries().map(|e| e.code.clone()).collect();
        assert_eq!(codes.len(), MAX_ERRORS);
        assert_eq!(codes[0], "e5");
        assert_eq!(codes[19], "e24");
    }

    #[test]
    fn the_report_has_no_room_for_text() {
        let cfg = NativeConfig::default();
        let errors = ErrorLog::default();
        let r = report(&Snapshot {
            os: "Windows 10.0.26200",
            config: &cfg,
            connected: true,
            extension_version: Some("0.1.0"),
            browser: Some("Chrome 140"),
            errors: &errors,
            notes: &[],
            field_check: None,
        });
        let keys: Vec<_> = r.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "app",
                "browser",
                "config",
                "connected",
                "extensionVersion",
                "lastFieldCheck",
                "notes",
                "os",
                "recentErrors",
                "version"
            ]
        );
    }

    #[test]
    fn masks_templates_and_replacements_in_report() {
        let mut cfg = NativeConfig::default();
        cfg.templates = vec!["CONFIDENTIAL TEMPLATE TEXT".to_string()];
        cfg.replacements = vec![crate::config::ReplacementRule {
            from: "SECRET_WORD".to_string(),
            to: "REPLACED_WORD".to_string(),
        }];
        let errors = ErrorLog::default();
        let r = report(&Snapshot {
            os: "Windows 10.0.26200",
            config: &cfg,
            connected: true,
            extension_version: Some("0.1.0"),
            browser: Some("Chrome 140"),
            errors: &errors,
            notes: &[],
            field_check: None,
        });
        let config_str = r["config"].to_string();
        assert!(!config_str.contains("CONFIDENTIAL"));
        assert!(!config_str.contains("SECRET_WORD"));
        assert_eq!(r["config"]["templatesCount"], 1);
        assert_eq!(r["config"]["replacementsCount"], 1);
    }
}
