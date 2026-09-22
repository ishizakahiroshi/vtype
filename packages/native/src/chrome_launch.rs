//! How the daemon starts its own Chrome for the speech page (standalone plan C4): a profile of
//! its own (never the user's), the page as an `--app` window, and the window either on screen
//! (first run: the consent needs the user) or off screen. The microphone is allowed by writing
//! Chrome's own setting into that profile once the user agreed (`grant_microphone`), so Chrome's
//! per-site prompt never shows.
//!
//! Off screen is what the C1 spike measured: Chrome at `--window-position=-32000,-32000` kept
//! recognising for 10 minutes. The page cannot hide itself later (Chrome pulls `window.moveTo`
//! back onto the screen), but it can close itself; the daemon then starts it again off screen.
//!
//! No `--remote-debugging-port`, no headless mode (parent plan S6).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPlacement {
    /// On screen, for the first-run consent. Closes itself once the user agreed.
    Visible,
    /// On screen, and it stays: the microphone was allowed only this time (Chrome's "allow this
    /// time"), which a closed window loses, so the page cannot be hidden by starting it again.
    Kept,
    /// Off screen, for recognising.
    Hidden,
}

/// The daemon's own Chrome profile: the microphone grant for the page lives here.
pub fn profile_dir() -> PathBuf {
    crate::paths::data_dir().join("chrome-profile")
}

/// The page's URL for this launch. `consent=1` tells the page the user already agreed;
/// `setup=1` tells it to close itself once consent and the microphone are done; `stay=1` tells
/// it never to close itself.
pub fn page_url(base: &str, consented: bool, placement: WindowPlacement) -> String {
    let mut params = Vec::new();
    if consented {
        params.push("consent=1");
    }
    match placement {
        WindowPlacement::Visible => params.push("setup=1"),
        WindowPlacement::Kept => params.push("stay=1"),
        WindowPlacement::Hidden => {}
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", params.join("&"))
    }
}

/// The page's origin (`http://127.0.0.1:<port>`) from its URL.
pub fn origin_of(page_url: &str) -> Option<&str> {
    let rest = page_url.strip_prefix("http://")?;
    let end = rest.find('/').unwrap_or(rest.len());
    Some(&page_url[.."http://".len() + end])
}

/// The profile's Preferences with the microphone allowed for `origin` (Chrome's own content
/// setting, as "allow while visiting the site" leaves it). The user agreed on vtype's own consent
/// page; Chrome's per-site prompt means nothing to someone using a desktop app, and its "allow
/// this time" would be lost every time the window is started again off screen.
/// `now_us` is Chrome's clock: microseconds since 1601-01-01.
pub fn grant_microphone(mut prefs: Value, origin: &str, now_us: u64) -> Value {
    if !prefs.is_object() {
        prefs = json!({});
    }
    let mut node = &mut prefs;
    for key in ["profile", "content_settings", "exceptions", "media_stream_mic"] {
        if !node.get(key).is_some_and(Value::is_object) {
            node[key] = json!({});
        }
        node = &mut node[key];
    }
    node[format!("{origin},*")] = json!({ "setting": 1, "last_modified": now_us.to_string() });
    prefs
}

/// Writes the grant into `<profile>/Default/Preferences`. Only while Chrome is not using the
/// profile does it stick; Chrome rewrites the file when it exits.
pub fn grant_microphone_in_profile(profile_dir: &Path, origin: &str) -> std::io::Result<()> {
    let dir = profile_dir.join("Default");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("Preferences");
    let prefs = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| json!({}));
    const WINDOWS_TO_UNIX_US: u64 = 11_644_473_600_000_000;
    let now_us =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0) + WINDOWS_TO_UNIX_US;
    let text = serde_json::to_string(&grant_microphone(prefs, origin, now_us)).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// Chrome's arguments, one string each (a path or a URL with spaces stays one argument).
pub fn speech_args(profile_dir: &Path, url: &str, placement: WindowPlacement) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile_dir.display()),
        format!("--app={url}"),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    match placement {
        WindowPlacement::Hidden => {
            args.push("--window-position=-32000,-32000".to_string());
            args.push("--window-size=400,300".to_string());
        }
        WindowPlacement::Visible | WindowPlacement::Kept => {
            args.push("--window-position=240,160".to_string());
            args.push("--window-size=560,440".to_string());
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:47213/t/0123456789abcdef0123456789abcdef/speech";

    #[test]
    fn a_profile_of_its_own_and_the_page_as_an_app() {
        let args = speech_args(Path::new("/data/vtype/chrome-profile"), BASE, WindowPlacement::Hidden);
        assert_eq!(
            args,
            vec![
                "--user-data-dir=/data/vtype/chrome-profile".to_string(),
                format!("--app={BASE}"),
                "--no-first-run".into(),
                "--no-default-browser-check".into(),
                "--window-position=-32000,-32000".into(),
                "--window-size=400,300".into(),
            ]
        );
    }

    #[test]
    fn visible_is_on_screen() {
        let args = speech_args(Path::new("p"), BASE, WindowPlacement::Visible);
        assert!(args.contains(&"--window-position=240,160".to_string()));
        assert!(!args.iter().any(|a| a.contains("-32000")));
    }

    #[test]
    fn spaces_stay_inside_one_argument() {
        let args = speech_args(Path::new("home of Taro Y/vtype/chrome profile"), BASE, WindowPlacement::Hidden);
        assert_eq!(args[0], "--user-data-dir=home of Taro Y/vtype/chrome profile");
        assert_eq!(args.len(), 6);
    }

    #[test]
    fn never_opens_a_debugging_port_or_runs_headless() {
        for placement in [WindowPlacement::Hidden, WindowPlacement::Visible, WindowPlacement::Kept] {
            let args = speech_args(Path::new("p"), BASE, placement);
            assert!(!args.iter().any(|a| a.contains("remote-debugging") || a.contains("headless")), "{args:?}");
        }
    }

    #[test]
    fn the_origin_is_the_page_url_up_to_the_path() {
        assert_eq!(origin_of(BASE), Some("http://127.0.0.1:47213"));
        assert_eq!(origin_of("https://example.com/x"), None);
    }

    #[test]
    fn the_microphone_grant_goes_into_chromes_own_setting_and_keeps_the_rest() {
        let before = json!({
            "profile": { "name": "Person 1", "content_settings": { "exceptions": {
                "media_stream_mic": { "https://other.example:443,*": { "setting": 2 } },
                "geolocation": {}
            } } },
            "browser": { "has_seen_welcome_page": true }
        });
        let after = grant_microphone(before, "http://127.0.0.1:47213", 13_434_555_907_811_152);
        let mic = &after["profile"]["content_settings"]["exceptions"]["media_stream_mic"];
        assert_eq!(mic["http://127.0.0.1:47213,*"], json!({ "setting": 1, "last_modified": "13434555907811152" }));
        assert_eq!(mic["https://other.example:443,*"]["setting"], 2);
        assert_eq!(after["profile"]["name"], "Person 1");
        assert_eq!(after["browser"]["has_seen_welcome_page"], true);
        // A profile Chrome has not written yet.
        let fresh = grant_microphone(Value::Null, "http://127.0.0.1:47213", 1);
        assert_eq!(
            fresh["profile"]["content_settings"]["exceptions"]["media_stream_mic"]["http://127.0.0.1:47213,*"]
                ["setting"],
            1
        );
    }

    #[test]
    fn the_url_says_consent_and_setup() {
        assert_eq!(page_url(BASE, false, WindowPlacement::Visible), format!("{BASE}?setup=1"));
        assert_eq!(page_url(BASE, true, WindowPlacement::Visible), format!("{BASE}?consent=1&setup=1"));
        assert_eq!(page_url(BASE, true, WindowPlacement::Hidden), format!("{BASE}?consent=1"));
        assert_eq!(page_url(BASE, false, WindowPlacement::Hidden), BASE);
        assert_eq!(page_url(BASE, true, WindowPlacement::Kept), format!("{BASE}?consent=1&stay=1"));
    }
}
