//! The mic beside the text field (child plan C8, experimental, off by default: parent plan D18).
//! When a text field takes the focus (or, by choice, the pointer rests on one), a small mic
//! appears next to it; pressing it starts recording. Where the platform can move the floating mic
//! (Windows), a field taking the focus brings the floating mic itself there instead, and it stays
//! when the focus leaves.
//!
//! What to show and where is decided here, the same on every OS; the platform code only reports
//! what the focused (or pointed-at) element is and puts the window where it is told.
// Linux has no beside mic (parent plan D18): the daemon still asks, and nothing reports fields.
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::time::{Duration, Instant};

use crate::config::BesideFieldConfig;
use crate::platform::Rect;

/// The beside mic is smaller than the floating one (40 px).
pub const BESIDE_SIZE: i32 = 26;
/// Space between the caret (or the field's edge) and the mic.
const GAP: i32 = 4;
/// Space between the pointer and the mic, so the I-beam stays clear of it.
const POINTER_GAP: i32 = 10;

/// Hover: the pointer must rest on the same field this long before the mic shows…
pub const HOVER_SHOW_AFTER: Duration = Duration::from_millis(300);
/// …and be away this long before it goes.
pub const HOVER_HIDE_AFTER: Duration = Duration::from_millis(500);
/// How often the pointer is looked at (no input hooks).
pub const HOVER_POLL: Duration = Duration::from_millis(150);

/// What the OS says about one element (the focused one, or the one under the pointer).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldProbe {
    /// An editable text field (not read-only, takes the keyboard).
    pub is_text_field: bool,
    pub is_password: Option<bool>,
    /// Executable name (Windows) or bundle id (macOS) of the app it belongs to.
    pub app_id: Option<String>,
    pub caret: Option<Rect>,
    pub bounds: Option<Rect>,
    /// Where the pointer was when the field took the focus (screen pixels). Inside the field it
    /// is where the user clicked.
    pub pointer: Option<(i32, i32)>,
}

fn contains(r: Rect, (x, y): (i32, i32)) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// The vtype extension shows its own mic beside Chrome's fields; with it installed, two would be
/// one too many (the `in_chrome` setting).
pub fn is_chrome(app_id: &str) -> bool {
    app_id.eq_ignore_ascii_case("chrome.exe") || app_id.starts_with("com.google.Chrome")
}

/// Where the user is in a field, which the mic goes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Clicked into: where the pointer was. Chrome gives no caret, and the field's end can be far
    /// from where the hand is.
    Pointer(i32, i32),
    /// The caret. Apps give it as 0 or 1 px wide, one character, or even the whole line; its left
    /// edge is where the caret is in every case.
    Caret(Rect),
    /// Only the field (Chrome after Tab).
    Field(Rect),
}

/// Where the user is in the field, or `None` when no mic belongs there.
pub fn anchor(config: &BesideFieldConfig, probe: &FieldProbe) -> Option<Anchor> {
    if !config.enabled || !probe.is_text_field || probe.is_password == Some(true) {
        return None;
    }
    if !config.in_chrome && probe.app_id.as_deref().is_some_and(is_chrome) {
        return None;
    }
    if let Some((x, y)) = probe.pointer.filter(|&p| probe.bounds.is_some_and(|b| contains(b, p))) {
        return Some(Anchor::Pointer(x, y));
    }
    if let Some(c) = probe.caret.filter(|c| c.height > 0) {
        return Some(Anchor::Caret(c));
    }
    probe.bounds.filter(|b| b.width > 0 && b.height > 0).map(Anchor::Field)
}

/// The small mic's top-left (before keeping it on a screen): just above and right of the pointer
/// or the caret; with only the field, inside its right end, centred.
pub fn beside_position(anchor: Anchor) -> (i32, i32) {
    match anchor {
        Anchor::Pointer(x, y) => (x + POINTER_GAP, y - BESIDE_SIZE - POINTER_GAP),
        Anchor::Caret(c) => (c.x + GAP, c.y - BESIDE_SIZE),
        Anchor::Field(b) => (b.x + b.width - BESIDE_SIZE - GAP, b.y + b.height / 2 - BESIDE_SIZE / 2),
    }
}

/// Where the small mic goes (top-left, before keeping it on a screen), or `None` for no mic.
pub fn decide(config: &BesideFieldConfig, probe: &FieldProbe) -> Option<(i32, i32)> {
    anchor(config, probe).map(beside_position)
}

/// The floating mic's top-left when it comes to a field (`size` is its window), before keeping it
/// on a screen: above the line, so it covers neither the text nor the IME's candidates below it.
/// Right of the pointer or the caret; with only the field, over the field's start.
pub fn follow_position(anchor: Anchor, size: i32) -> (i32, i32) {
    match anchor {
        Anchor::Pointer(x, y) => (x + POINTER_GAP, y - size - POINTER_GAP),
        Anchor::Caret(c) => (c.x + GAP, c.y - size),
        Anchor::Field(b) => (b.x, b.y - size),
    }
}

/// Moves `pos` so the mic lies inside the work area it is (mostly) on, or the primary one.
pub fn keep_on_screen(pos: (i32, i32), size: i32, work_areas: &[Rect], primary: Rect) -> (i32, i32) {
    let (cx, cy) = (pos.0 + size / 2, pos.1 + size / 2);
    let area = work_areas
        .iter()
        .copied()
        .find(|a| cx >= a.x && cx < a.x + a.width && cy >= a.y && cy < a.y + a.height)
        .unwrap_or(primary);
    let x = pos.0.clamp(area.x, (area.x + area.width - size).max(area.x));
    let y = pos.1.clamp(area.y, (area.y + area.height - size).max(area.y));
    (x, y)
}

/// One look at what is under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pointed<K> {
    /// A text field, told apart from others by `K`.
    Field(K),
    /// Something else.
    Other,
    /// vtype's own mic (moving onto it to press it must not make it go away).
    Ours,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoverChange<K> {
    Show(K),
    Hide,
}

/// Hover as a small state machine over the polled observations.
#[derive(Debug)]
pub struct HoverTracker<K> {
    candidate: Option<(K, Instant)>,
    shown: Option<K>,
    away_since: Option<Instant>,
}

impl<K> Default for HoverTracker<K> {
    fn default() -> Self {
        HoverTracker { candidate: None, shown: None, away_since: None }
    }
}

impl<K: Clone + PartialEq> HoverTracker<K> {
    pub fn observe(&mut self, now: Instant, pointed: Pointed<K>) -> Option<HoverChange<K>> {
        match pointed {
            Pointed::Ours => {
                self.away_since = None;
                None
            }
            Pointed::Field(k) => {
                self.away_since = None;
                if self.shown.as_ref() == Some(&k) {
                    return None;
                }
                match &self.candidate {
                    Some((c, since)) if *c == k => {
                        if now.duration_since(*since) >= HOVER_SHOW_AFTER {
                            self.candidate = None;
                            self.shown = Some(k.clone());
                            return Some(HoverChange::Show(k));
                        }
                        None
                    }
                    _ => {
                        self.candidate = Some((k, now));
                        None
                    }
                }
            }
            Pointed::Other => {
                self.candidate = None;
                self.shown.as_ref()?;
                let since = *self.away_since.get_or_insert(now);
                if now.duration_since(since) >= HOVER_HIDE_AFTER {
                    self.shown = None;
                    self.away_since = None;
                    return Some(HoverChange::Hide);
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BesideFieldTrigger;

    fn on() -> BesideFieldConfig {
        BesideFieldConfig { enabled: true, trigger: BesideFieldTrigger::Focus, in_chrome: true }
    }

    fn field() -> FieldProbe {
        FieldProbe {
            is_text_field: true,
            is_password: Some(false),
            app_id: Some("notepad.exe".into()),
            caret: Some(Rect { x: 100, y: 200, width: 1, height: 18 }),
            bounds: Some(Rect { x: 50, y: 190, width: 400, height: 40 }),
            pointer: None,
        }
    }

    #[test]
    fn shows_beside_the_caret() {
        assert_eq!(decide(&on(), &field()), Some((100 + GAP, 200 - BESIDE_SIZE)));
        // Measured on Windows: an empty box gave its whole line as the caret; still its left edge.
        let wide = FieldProbe { caret: Some(Rect { x: 72, y: 95, width: 376, height: 19 }), ..field() };
        assert_eq!(decide(&on(), &wide), Some((72 + GAP, 95 - BESIDE_SIZE)));
    }

    #[test]
    fn a_click_puts_the_mic_by_the_pointer() {
        // Measured on Windows: Chrome gives no caret, and its chat box was 877 px wide.
        let chat = FieldProbe {
            app_id: Some("chrome.exe".into()),
            caret: None,
            bounds: Some(Rect { x: 379, y: 873, width: 877, height: 24 }),
            pointer: Some((420, 885)),
            ..field()
        };
        assert_eq!(decide(&on(), &chat), Some((420 + POINTER_GAP, 885 - BESIDE_SIZE - POINTER_GAP)));
        // Ahead of the caret too: the hand is where the user is.
        let clicked = FieldProbe { pointer: Some((300, 210)), ..field() };
        assert_eq!(decide(&on(), &clicked), Some((300 + POINTER_GAP, 210 - BESIDE_SIZE - POINTER_GAP)));
        // The pointer outside the field (Tab moved the focus), or no field to check it against:
        // by the caret as before.
        let tabbed = FieldProbe { pointer: Some((10, 10)), ..field() };
        assert_eq!(decide(&on(), &tabbed), Some((100 + GAP, 200 - BESIDE_SIZE)));
        let unbounded = FieldProbe { pointer: Some((300, 210)), bounds: None, ..field() };
        assert_eq!(decide(&on(), &unbounded), Some((100 + GAP, 200 - BESIDE_SIZE)));
    }

    #[test]
    fn the_floating_mic_comes_above_the_line_by_the_pointer_the_caret_or_the_field() {
        let size = 112;
        let clicked = anchor(&on(), &FieldProbe { pointer: Some((300, 210)), ..field() }).unwrap();
        assert_eq!(clicked, Anchor::Pointer(300, 210));
        assert_eq!(follow_position(clicked, size), (300 + POINTER_GAP, 210 - size - POINTER_GAP));
        // Tab: the pointer is elsewhere; by the caret.
        let tabbed = anchor(&on(), &FieldProbe { pointer: Some((10, 10)), ..field() }).unwrap();
        assert_eq!(follow_position(tabbed, size), (100 + GAP, 200 - size));
        // Tab into Chrome (no caret): over the field's start.
        let chrome = FieldProbe {
            app_id: Some("chrome.exe".into()),
            caret: None,
            bounds: Some(Rect { x: 379, y: 873, width: 877, height: 24 }),
            pointer: Some((10, 10)),
            ..field()
        };
        let tabbed_chrome = anchor(&on(), &chrome).unwrap();
        assert_eq!(tabbed_chrome, Anchor::Field(Rect { x: 379, y: 873, width: 877, height: 24 }));
        assert_eq!(follow_position(tabbed_chrome, size), (379, 873 - size));
        // Nothing to go by, or no mic wanted there: no anchor.
        assert_eq!(anchor(&on(), &FieldProbe { caret: None, bounds: None, ..field() }), None);
        assert_eq!(anchor(&on(), &FieldProbe { is_password: Some(true), ..field() }), None);
    }

    #[test]
    fn falls_back_to_the_fields_right_end() {
        let probe = FieldProbe { caret: None, ..field() };
        assert_eq!(decide(&on(), &probe), Some((50 + 400 - BESIDE_SIZE - GAP, 190 + 20 - BESIDE_SIZE / 2)));
        let nothing = FieldProbe { caret: None, bounds: None, ..field() };
        assert_eq!(decide(&on(), &nothing), None);
    }

    #[test]
    fn stays_away_where_it_does_not_belong() {
        let off = BesideFieldConfig { enabled: false, ..on() };
        assert_eq!(decide(&off, &field()), None);
        assert_eq!(decide(&on(), &FieldProbe { is_password: Some(true), ..field() }), None);
        assert_eq!(decide(&on(), &FieldProbe { is_text_field: false, ..field() }), None);
        // Unknown password state still shows (the typing itself checks again).
        assert!(decide(&on(), &FieldProbe { is_password: None, ..field() }).is_some());
    }

    #[test]
    fn chrome_gets_the_mic_unless_turned_off() {
        // Measured on Windows: Chrome's fields are text fields with no caret; the field's right end.
        let chrome = FieldProbe {
            app_id: Some("chrome.exe".into()),
            caret: None,
            bounds: Some(Rect { x: -1209, y: 215, width: 387, height: 45 }),
            ..field()
        };
        assert_eq!(decide(&on(), &chrome), Some((-1209 + 387 - BESIDE_SIZE - GAP, 215 + 22 - BESIDE_SIZE / 2)));
        let no_chrome = BesideFieldConfig { in_chrome: false, ..on() };
        assert_eq!(decide(&no_chrome, &chrome), None);
        assert_eq!(decide(&no_chrome, &FieldProbe { app_id: Some("com.google.Chrome".into()), ..field() }), None);
        assert!(decide(&no_chrome, &field()).is_some());
    }

    #[test]
    fn keeps_the_mic_on_its_screen() {
        let screen = Rect { x: 0, y: 0, width: 1920, height: 1040 };
        assert_eq!(keep_on_screen((1910, -10), BESIDE_SIZE, &[screen], screen), (1920 - BESIDE_SIZE, 0));
        assert_eq!(keep_on_screen((300, 300), BESIDE_SIZE, &[screen], screen), (300, 300));
        let second = Rect { x: 1920, y: 0, width: 1280, height: 1024 };
        assert_eq!(keep_on_screen((3180, 500), BESIDE_SIZE, &[screen, second], screen), (3200 - BESIDE_SIZE, 500));
    }

    #[test]
    fn hover_waits_then_shows_and_lingers_before_hiding() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut h: HoverTracker<u32> = HoverTracker::default();
        assert_eq!(h.observe(at(0), Pointed::Field(1)), None);
        assert_eq!(h.observe(at(150), Pointed::Field(1)), None);
        assert_eq!(h.observe(at(300), Pointed::Field(1)), Some(HoverChange::Show(1)));
        assert_eq!(h.observe(at(450), Pointed::Field(1)), None);
        // Onto the mic itself: it stays.
        assert_eq!(h.observe(at(600), Pointed::Ours), None);
        assert_eq!(h.observe(at(750), Pointed::Other), None);
        assert_eq!(h.observe(at(1100), Pointed::Other), None);
        assert_eq!(h.observe(at(1250), Pointed::Other), Some(HoverChange::Hide));
        assert_eq!(h.observe(at(1400), Pointed::Other), None);
    }

    #[test]
    fn hover_restarts_the_wait_on_another_field() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut h: HoverTracker<u32> = HoverTracker::default();
        h.observe(at(0), Pointed::Field(1));
        assert_eq!(h.observe(at(200), Pointed::Field(2)), None);
        assert_eq!(h.observe(at(400), Pointed::Field(2)), None);
        assert_eq!(h.observe(at(500), Pointed::Field(2)), Some(HoverChange::Show(2)));
    }
}
