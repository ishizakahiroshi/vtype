//! "The mic does not come to this app's text fields" (plan C1). For `CHECK_FOR` after the user
//! asks, what the app reports and what its focused element is are noted, then sorted into the ways
//! an app can keep the mic away. Only what kind of element it was is kept: never its name or
//! value, which can hold what was written.
//!
//! The record and the verdict are the same on every OS; only Windows looks (the floating mic comes
//! to fields only there).
#![cfg_attr(not(windows), allow(dead_code))]

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::i18n::{t, t_with};
use crate::platform::Rect;

/// How long an app is watched after the user asks.
pub const CHECK_FOR: Duration = Duration::from_secs(10);

/// At most this many sightings are kept, so the diagnostic info stays short.
pub const MAX_SEEN: usize = 40;

/// How an element was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Path {
    /// The focused element when the check began. Opened from the mic's menu, the focus goes back
    /// to the app before the check listens, so a field found here tells nothing about reports.
    Start,
    /// The app reported it taking the focus.
    Report,
    /// Looked at again after the app reported its window (as the mic does for LINE).
    Recheck,
    /// The focused element, looked at every so often whether the app reported anything or not.
    Look,
}

/// What an element is. Nothing here can hold text the user wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    /// The control type, e.g. `Edit`, `Window`, `ListItem`.
    pub kind: String,
    /// The class name the app gave it, e.g. `AutoSuggestTextArea`.
    pub class: String,
    pub keyboard_focusable: bool,
    pub has_value: bool,
    pub read_only: bool,
    pub has_text: bool,
    /// Taken as a text field: what brings the mic.
    pub takes_text: bool,
    pub bounds: Option<Rect>,
}

/// One element, found one way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    /// Milliseconds from the start of the check.
    pub after_ms: u64,
    pub path: Path,
    #[serde(flatten)]
    pub element: Element,
}

/// What was seen of one app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldCheck {
    /// Executable name, e.g. `line.exe`.
    pub app_id: String,
    pub seen: Vec<Seen>,
}

/// The ways an app can give its text fields away, or not (plan C1, "来ないアプリの 4 つの型").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Type 1: the app reports its text field taking the focus.
    Reported,
    /// Type 2: the app reports its window, and its text field takes the focus a moment later
    /// without a report (LINE).
    AfterWindow,
    /// Type 2, another way: no report at all, but asking for the focused element finds the field.
    OnlyWhenAsked,
    /// Not known yet: the text field had the focus from the start and kept it, so clicking it
    /// reported nothing. The user is asked to click elsewhere first and check again.
    AlreadyThere,
    /// Type 3: something that looks typeable had the focus, but is not taken as a text field.
    NotTaken,
    /// Type 4: nothing that looks typeable was found.
    Nothing,
}

/// Something the user could type into: takes the keyboard, is not read-only, and holds a value or
/// text.
pub fn looks_typeable(e: &Element) -> bool {
    e.keyboard_focusable && !e.read_only && (e.has_value || e.has_text)
}

/// Which way the app gives its text fields away. A text field found by a report counts first,
/// then one found by looking again after a window's report, then one found only by looking, then
/// one that had the focus from the start.
pub fn classify(seen: &[Seen]) -> Verdict {
    let found = |path: Path| seen.iter().any(|s| s.path == path && s.element.takes_text);
    if found(Path::Report) {
        Verdict::Reported
    } else if found(Path::Recheck) {
        Verdict::AfterWindow
    } else if found(Path::Look) {
        Verdict::OnlyWhenAsked
    } else if found(Path::Start) {
        Verdict::AlreadyThere
    } else if seen.iter().any(|s| looks_typeable(&s.element)) {
        Verdict::NotTaken
    } else {
        Verdict::Nothing
    }
}

/// What the bubble says about a finished check.
pub fn message(verdict: Verdict, app: &str) -> String {
    let seconds = CHECK_FOR.as_secs().to_string();
    let found = match verdict {
        Verdict::Reported => t_with("native_bubbleCheckFieldsReported", &[("app", app)]),
        Verdict::AfterWindow => t_with("native_bubbleCheckFieldsAfterWindow", &[("app", app)]),
        Verdict::OnlyWhenAsked => t_with("native_bubbleCheckFieldsOnlyWhenAsked", &[("app", app)]),
        Verdict::AlreadyThere => t_with("native_bubbleCheckFieldsAlreadyThere", &[("app", app)]),
        Verdict::NotTaken => t_with("native_bubbleCheckFieldsNotTaken", &[("app", app)]),
        Verdict::Nothing => t_with("native_bubbleCheckFieldsNothing", &[("app", app), ("seconds", &seconds)]),
    };
    format!("{found}\n{}", t("native_bubbleCheckFieldsDetails"))
}

/// A finished check for the diagnostic info: when (UTC), which app, the verdict, what was seen.
pub fn report(at: &str, check: &FieldCheck) -> Value {
    json!({
        "at": at,
        "appId": check.app_id,
        "verdict": classify(&check.seen),
        "seen": check.seen,
    })
}

/// What a check has seen so far.
#[derive(Debug)]
pub struct Record {
    started: Instant,
    seen: Vec<Seen>,
    /// The last element found by looking, and by looking again since the last report: the same
    /// one again adds nothing (it is looked at every few hundred milliseconds).
    last_look: Option<Element>,
    last_recheck: Option<Element>,
}

impl Record {
    pub fn new(started: Instant) -> Record {
        Record { started, seen: Vec::new(), last_look: None, last_recheck: None }
    }

    /// Notes `element`, found by `path` at `at`. Every report counts; a look (or a look again)
    /// only when it finds another element than the last one. Up to `MAX_SEEN`.
    pub fn note(&mut self, at: Instant, path: Path, element: Element) {
        let last = match path {
            Path::Report => {
                self.last_recheck = None;
                None
            }
            Path::Recheck => Some(&mut self.last_recheck),
            Path::Start | Path::Look => Some(&mut self.last_look),
        };
        if let Some(last) = last {
            if last.as_ref() == Some(&element) {
                return;
            }
            *last = Some(element.clone());
        }
        if self.seen.len() < MAX_SEEN {
            let after_ms = u64::try_from(at.saturating_duration_since(self.started).as_millis()).unwrap_or(u64::MAX);
            self.seen.push(Seen { after_ms, path, element });
        }
    }

    pub fn finish(self, app_id: String) -> FieldCheck {
        FieldCheck { app_id, seen: self.seen }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn element(kind: &str, class: &str) -> Element {
        Element {
            kind: kind.into(),
            class: class.into(),
            keyboard_focusable: true,
            has_value: false,
            read_only: false,
            has_text: false,
            takes_text: false,
            bounds: Some(Rect { x: 100, y: 600, width: 500, height: 60 }),
        }
    }

    fn line_window() -> Element {
        element("Window", "AllInOneWindow")
    }

    fn line_message() -> Element {
        Element { has_value: true, read_only: true, ..element("ListItem", "") }
    }

    fn line_field() -> Element {
        Element { has_value: true, has_text: true, takes_text: true, ..element("Edit", "AutoSuggestTextArea") }
    }

    /// Modelled on LINE as measured (2026-09-23): clicking its text field from another app reports
    /// the window; the focused element is a message at 0 and 100 ms, and the text field by 300 ms.
    pub fn line() -> FieldCheck {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut r = Record::new(t0);
        r.note(at(0), Path::Start, line_message());
        r.note(at(1800), Path::Report, line_window());
        r.note(at(1900), Path::Recheck, line_message());
        r.note(at(2000), Path::Look, line_message());
        r.note(at(2000), Path::Recheck, line_message());
        r.note(at(2100), Path::Recheck, line_field());
        r.note(at(2200), Path::Look, line_field());
        r.finish("line.exe".into())
    }

    fn one(path: Path, element: Element) -> Vec<Seen> {
        vec![Seen { after_ms: 0, path, element }]
    }

    #[test]
    fn line_moves_into_its_field_after_reporting_the_window() {
        let check = line();
        let paths: Vec<(u64, Path, &str)> =
            check.seen.iter().map(|s| (s.after_ms, s.path, s.element.kind.as_str())).collect();
        assert_eq!(
            paths,
            vec![
                (0, Path::Start, "ListItem"),
                (1800, Path::Report, "Window"),
                (1900, Path::Recheck, "ListItem"),
                (2100, Path::Recheck, "Edit"),
                (2200, Path::Look, "Edit"),
            ],
            "the same element again adds nothing"
        );
        assert_eq!(classify(&check.seen), Verdict::AfterWindow);
    }

    #[test]
    fn chrome_reports_its_field() {
        // Measured on Windows: Chrome's text fields are reported as Edit.
        let chrome =
            Element { has_value: true, has_text: true, takes_text: true, ..element("Edit", "OmniboxViewViews") };
        let mut seen = one(Path::Report, chrome.clone());
        seen.push(Seen { after_ms: 200, path: Path::Look, element: chrome });
        assert_eq!(classify(&seen), Verdict::Reported);
    }

    #[test]
    fn a_field_found_only_by_asking_is_told_apart() {
        let mut seen = one(Path::Look, line_message());
        seen.push(Seen { after_ms: 400, path: Path::Look, element: line_field() });
        assert_eq!(classify(&seen), Verdict::OnlyWhenAsked);
    }

    #[test]
    fn a_field_that_had_the_focus_from_the_start_asks_to_check_again() {
        // Opened from the mic's menu, the focus is back in the field before the check listens,
        // and clicking a field that has the focus reports nothing.
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut r = Record::new(t0);
        r.note(at(0), Path::Start, line_field());
        r.note(at(200), Path::Look, line_field());
        let check = r.finish("notepad.exe".into());
        assert_eq!(check.seen.len(), 1, "the same field looked at again adds nothing");
        assert_eq!(classify(&check.seen), Verdict::AlreadyThere);
        // Out of the field and back with no report on the way: found only by looking.
        let mut r = Record::new(t0);
        r.note(at(0), Path::Start, line_field());
        r.note(at(200), Path::Look, line_message());
        r.note(at(400), Path::Look, line_field());
        assert_eq!(classify(&r.finish("notepad.exe".into()).seen), Verdict::OnlyWhenAsked);
    }

    #[test]
    fn something_typeable_that_is_not_taken_is_type_3() {
        // A text field some apps give as another kind (Windows Terminal and Google's search box
        // were such, before `takes_text` took them).
        let custom = Element { has_value: true, ..element("Custom", "ChatInput") };
        assert!(looks_typeable(&custom));
        assert_eq!(classify(&one(Path::Report, custom.clone())), Verdict::NotTaken);
        assert_eq!(
            classify(&one(Path::Look, Element { has_value: false, has_text: true, ..custom })),
            Verdict::NotTaken
        );
        // With a text field reported as well, the field counts.
        let mut both = one(Path::Report, Element { has_value: true, ..element("Custom", "ChatInput") });
        both.push(Seen { after_ms: 300, path: Path::Report, element: line_field() });
        assert_eq!(classify(&both), Verdict::Reported);
    }

    #[test]
    fn nothing_typeable_is_type_4() {
        assert_eq!(classify(&[]), Verdict::Nothing);
        let mut seen = one(Path::Report, line_window());
        // Read-only, not focusable, or holding neither a value nor text: not typeable.
        seen.push(Seen { after_ms: 100, path: Path::Look, element: line_message() });
        let unfocusable = Element { keyboard_focusable: false, has_value: true, ..element("Pane", "") };
        seen.push(Seen { after_ms: 200, path: Path::Look, element: unfocusable });
        seen.push(Seen { after_ms: 300, path: Path::Look, element: element("Button", "") });
        assert_eq!(classify(&seen), Verdict::Nothing);
    }

    #[test]
    fn a_new_report_lets_the_same_field_count_again_and_the_record_stays_short() {
        let t0 = Instant::now();
        let mut r = Record::new(t0);
        r.note(t0, Path::Report, line_window());
        r.note(t0, Path::Recheck, line_field());
        r.note(t0, Path::Report, line_window());
        r.note(t0, Path::Recheck, line_field());
        assert_eq!(r.seen.len(), 4, "every report, and the field after each");
        for i in 0..MAX_SEEN as i32 {
            let moved = Element { bounds: Some(Rect { x: i, y: 0, width: 1, height: 1 }), ..line_message() };
            r.note(t0, Path::Look, moved);
        }
        assert_eq!(r.finish("line.exe".into()).seen.len(), MAX_SEEN);
    }

    #[test]
    fn every_verdict_is_said_with_the_app() {
        for verdict in [
            Verdict::Reported,
            Verdict::AfterWindow,
            Verdict::OnlyWhenAsked,
            Verdict::AlreadyThere,
            Verdict::NotTaken,
            Verdict::Nothing,
        ] {
            let text = message(verdict, "line.exe");
            assert!(text.contains("line.exe"), "{verdict:?}: {text}");
            assert!(!text.contains('{'), "{verdict:?}: {text}");
        }
    }
}
