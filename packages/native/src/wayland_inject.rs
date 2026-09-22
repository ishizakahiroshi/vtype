//! Typing on Wayland, where an app may not send keys to another (child plan C7-C2). Three routes,
//! tried in order until one works:
//!
//! 1. the RemoteDesktop portal: the text goes on the clipboard and the portal presses Ctrl+V
//!    (the first time, the desktop asks the user to allow it);
//! 2. `ydotool type`, when it is installed and its daemon runs (ASCII only: it types through the
//!    keyboard layout, so other characters come out wrong);
//! 3. the clipboard alone, and a notification to paste with Ctrl+V.
//!
//! The order and the trail of what failed are decided here, apart from the real routes, so they
//! are tested on every OS.
// On other systems this is built only for its tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::platform::{InjectOutcome, PlatformError};

pub trait WaylandRoutes {
    fn portal_paste(&mut self, text: &str) -> Result<(), String>;
    fn ydotool_type(&mut self, text: &str) -> Result<(), String>;
    fn copy(&mut self, text: &str) -> Result<(), String>;
}

pub struct WaylandResult {
    pub outcome: Result<InjectOutcome, PlatformError>,
    /// What each route did, for the diagnostic report (never the text itself).
    pub trail: String,
}

pub fn inject_wayland(text: &str, routes: &mut dyn WaylandRoutes) -> WaylandResult {
    let mut trail: Vec<String> = Vec::new();
    match routes.portal_paste(text) {
        Ok(()) => {
            trail.push("portal: ok".into());
            return WaylandResult { outcome: Ok(InjectOutcome::Pasted), trail: trail.join("; ") };
        }
        Err(e) => trail.push(format!("portal: {e}")),
    }
    if text.is_ascii() {
        match routes.ydotool_type(text) {
            Ok(()) => {
                trail.push("ydotool: ok".into());
                return WaylandResult { outcome: Ok(InjectOutcome::Typed), trail: trail.join("; ") };
            }
            Err(e) => trail.push(format!("ydotool: {e}")),
        }
    } else {
        trail.push("ydotool: skipped (not ASCII)".into());
    }
    let outcome = match routes.copy(text) {
        Ok(()) => {
            trail.push("clipboard: ok".into());
            Ok(InjectOutcome::CopiedOnly)
        }
        Err(e) => {
            trail.push(format!("clipboard: {e}"));
            Err(PlatformError::Failed("no way to type or copy on this Wayland session".into()))
        }
    };
    WaylandResult { outcome, trail: trail.join("; ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        portal: Option<String>,
        ydotool: Option<String>,
        copy: Option<String>,
        calls: Vec<&'static str>,
    }

    fn result(err: &Option<String>) -> Result<(), String> {
        match err {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    impl WaylandRoutes for Fake {
        fn portal_paste(&mut self, _text: &str) -> Result<(), String> {
            self.calls.push("portal");
            result(&self.portal)
        }
        fn ydotool_type(&mut self, _text: &str) -> Result<(), String> {
            self.calls.push("ydotool");
            result(&self.ydotool)
        }
        fn copy(&mut self, _text: &str) -> Result<(), String> {
            self.calls.push("copy");
            result(&self.copy)
        }
    }

    #[test]
    fn the_portal_comes_first() {
        let mut f = Fake::default();
        let r = inject_wayland("hello", &mut f);
        assert_eq!(r.outcome, Ok(InjectOutcome::Pasted));
        assert_eq!(f.calls, vec!["portal"]);
    }

    #[test]
    fn ydotool_when_the_portal_is_refused() {
        let mut f = Fake { portal: Some("refused".into()), ..Fake::default() };
        let r = inject_wayland("hello", &mut f);
        assert_eq!(r.outcome, Ok(InjectOutcome::Typed));
        assert_eq!(f.calls, vec!["portal", "ydotool"]);
        assert_eq!(r.trail, "portal: refused; ydotool: ok");
    }

    #[test]
    fn japanese_skips_ydotool_and_ends_on_the_clipboard() {
        let mut f = Fake { portal: Some("no portal".into()), ..Fake::default() };
        let r = inject_wayland("こんにちは", &mut f);
        assert_eq!(r.outcome, Ok(InjectOutcome::CopiedOnly));
        assert_eq!(f.calls, vec!["portal", "copy"]);
        assert_eq!(r.trail, "portal: no portal; ydotool: skipped (not ASCII); clipboard: ok");
        assert!(!r.trail.contains("こんにちは"));
    }

    #[test]
    fn fails_only_when_even_the_clipboard_does() {
        let mut f = Fake {
            portal: Some("a".into()),
            ydotool: Some("not installed".into()),
            copy: Some("no clipboard".into()),
            ..Fake::default()
        };
        let r = inject_wayland("hi", &mut f);
        assert!(r.outcome.is_err());
        assert_eq!(f.calls, vec!["portal", "ydotool", "copy"]);
    }
}
