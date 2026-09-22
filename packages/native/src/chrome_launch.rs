//! How the daemon starts its own Chrome for the speech page (standalone plan C4): a profile of
//! its own (never the user's), the page as an `--app` window, and the window either on screen
//! (first run: the consent and the microphone prompt need the user) or off screen.
//!
//! Off screen is what the C1 spike measured: Chrome at `--window-position=-32000,-32000` kept
//! recognising for 10 minutes. The page cannot hide itself later (Chrome pulls `window.moveTo`
//! back onto the screen), but it can close itself; the daemon then starts it again off screen.
//!
//! No `--remote-debugging-port`, no headless mode (parent plan S6).

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowPlacement {
    /// On screen, for the first-run consent and microphone prompt.
    Visible,
    /// Off screen, for recognising.
    Hidden,
}

/// The daemon's own Chrome profile: the microphone grant for the page lives here.
pub fn profile_dir() -> PathBuf {
    crate::paths::data_dir().join("chrome-profile")
}

/// The page's URL for this launch. `consent=1` tells the page the user already agreed;
/// `setup=1` tells it to close itself once consent and the microphone are done.
pub fn page_url(base: &str, consented: bool, placement: WindowPlacement) -> String {
    let mut params = Vec::new();
    if consented {
        params.push("consent=1");
    }
    if placement == WindowPlacement::Visible {
        params.push("setup=1");
    }
    if params.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", params.join("&"))
    }
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
        WindowPlacement::Visible => {
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
        for placement in [WindowPlacement::Hidden, WindowPlacement::Visible] {
            let args = speech_args(Path::new("p"), BASE, placement);
            assert!(!args.iter().any(|a| a.contains("remote-debugging") || a.contains("headless")), "{args:?}");
        }
    }

    #[test]
    fn the_url_says_consent_and_setup() {
        assert_eq!(page_url(BASE, false, WindowPlacement::Visible), format!("{BASE}?setup=1"));
        assert_eq!(page_url(BASE, true, WindowPlacement::Visible), format!("{BASE}?consent=1&setup=1"));
        assert_eq!(page_url(BASE, true, WindowPlacement::Hidden), format!("{BASE}?consent=1"));
        assert_eq!(page_url(BASE, false, WindowPlacement::Hidden), BASE);
    }
}
