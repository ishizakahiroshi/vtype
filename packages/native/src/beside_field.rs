//! The mic beside the text field (child plan C8, experimental, off by default: parent plan D18).
//! When a text field takes the focus (or, by choice, the pointer rests on one), a small mic
//! appears next to it; pressing it starts recording.
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
}

/// Chrome shows its own mic beside fields (the extension); two would be one too many.
pub fn is_chrome(app_id: &str) -> bool {
    app_id.eq_ignore_ascii_case("chrome.exe") || app_id.starts_with("com.google.Chrome")
}

/// Where the mic goes (top-left, before keeping it on a screen), or `None` for no mic.
pub fn decide(config: &BesideFieldConfig, probe: &FieldProbe) -> Option<(i32, i32)> {
    if !config.enabled || !probe.is_text_field || probe.is_password == Some(true) {
        return None;
    }
    if probe.app_id.as_deref().is_some_and(is_chrome) {
        return None;
    }
    // Just above and right of the caret; without one, inside the field's right end, centred.
    // Apps give the caret as 0 or 1 px wide, one character, or even the whole line; its left
    // edge is where the caret is in every case.
    if let Some(c) = probe.caret.filter(|c| c.height > 0) {
        return Some((c.x + GAP, c.y - BESIDE_SIZE));
    }
    let b = probe.bounds.filter(|b| b.width > 0 && b.height > 0)?;
    Some((b.x + b.width - BESIDE_SIZE - GAP, b.y + b.height / 2 - BESIDE_SIZE / 2))
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
        BesideFieldConfig { enabled: true, trigger: BesideFieldTrigger::Focus }
    }

    fn field() -> FieldProbe {
        FieldProbe {
            is_text_field: true,
            is_password: Some(false),
            app_id: Some("notepad.exe".into()),
            caret: Some(Rect { x: 100, y: 200, width: 1, height: 18 }),
            bounds: Some(Rect { x: 50, y: 190, width: 400, height: 40 }),
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
        assert_eq!(decide(&on(), &FieldProbe { app_id: Some("chrome.exe".into()), ..field() }), None);
        assert_eq!(decide(&on(), &FieldProbe { app_id: Some("com.google.Chrome".into()), ..field() }), None);
        // Unknown password state still shows (the typing itself checks again).
        assert!(decide(&on(), &FieldProbe { is_password: None, ..field() }).is_some());
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
