//! The ripple around the floating mic while the user speaks: rings that leave the mic's edge and
//! fade as they spread. Like the extension's waveform, it is driven by what the recognizer says
//! (sound or speech started or ended, text arriving), never by the microphone's volume: the
//! microphone is Chrome's, not vtype's. The same rules on every OS; each OS only runs a timer
//! (`FRAME`) while `active()` and draws `rings()`.

use std::time::Duration;

/// One animation step. The timer runs only while there is something to draw.
pub const FRAME: Duration = Duration::from_millis(33);
/// How long one ring takes from the mic's edge to the window's edge.
const RING_LIFE: f32 = 1.2;
/// While the voice is loud enough, a new ring leaves this often.
const SPAWN_EVERY: f32 = 0.45;
/// Below this level no new ring leaves.
const SPAWN_LEVEL: f32 = 0.3;
/// The target halves in this long without a new cue (the voice is assumed to fade).
const TARGET_HALF_LIFE: f32 = 0.8;
/// How fast the level follows the target (per second).
const EASE_RATE: f32 = 8.0;
/// Below this level with no ring left, the animation stops.
const REST: f32 = 0.01;

/// What the recognizer said, as far as the ripple cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceCue {
    Sound,
    Speech,
    SpeechEnd,
    /// Interim text arrived.
    Text,
}

impl VoiceCue {
    /// The Web Speech API event names the speech page forwards; the others do not move the ripple.
    pub fn from_activity(activity: &str) -> Option<VoiceCue> {
        match activity {
            "soundstart" => Some(VoiceCue::Sound),
            "speechstart" => Some(VoiceCue::Speech),
            "speechend" | "soundend" => Some(VoiceCue::SpeechEnd),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Wave {
    age: f32,
    strength: f32,
}

#[derive(Debug, Default)]
pub struct Ripple {
    recording: bool,
    level: f32,
    target: f32,
    since_spawn: f32,
    rings: Vec<Wave>,
}

impl Ripple {
    /// Waves leave only while recording; when it stops, the ones out there fade away.
    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
        if !recording {
            self.target = 0.0;
        }
    }

    pub fn cue(&mut self, cue: VoiceCue) {
        if !self.recording {
            return;
        }
        match cue {
            VoiceCue::Sound => self.target = self.target.max(0.55),
            VoiceCue::Speech => self.target = self.target.max(0.9),
            VoiceCue::Text => self.target = self.target.max(0.85),
            VoiceCue::SpeechEnd => self.target = self.target.min(0.25),
        }
    }

    /// Moves the animation `dt` seconds on.
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        self.target *= 0.5f32.powf(dt / TARGET_HALF_LIFE);
        self.level += (self.target - self.level) * (1.0 - (-dt * EASE_RATE).exp());
        for ring in &mut self.rings {
            ring.age += dt;
        }
        self.rings.retain(|r| r.age < RING_LIFE);
        self.since_spawn += dt;
        if self.recording && self.level >= SPAWN_LEVEL && (self.rings.is_empty() || self.since_spawn >= SPAWN_EVERY) {
            self.rings.push(Wave { age: 0.0, strength: self.level.min(1.0) });
            self.since_spawn = 0.0;
        }
    }

    /// Whether the timer should keep running.
    pub fn active(&self) -> bool {
        !self.rings.is_empty() || self.level >= REST || (self.recording && self.target >= REST)
    }

    /// Each ring as (how far it has spread 0..1, opacity 0..1).
    pub fn rings(&self) -> Vec<(f32, f32)> {
        self.rings
            .iter()
            .map(|r| {
                let t = r.age / RING_LIFE;
                (t, r.strength * (1.0 - t) * 0.85)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(r: &mut Ripple, seconds: f32) {
        let steps = (seconds / FRAME.as_secs_f32()).round() as usize;
        for _ in 0..steps {
            r.tick(FRAME.as_secs_f32());
        }
    }

    #[test]
    fn nothing_moves_before_the_user_speaks() {
        let mut r = Ripple::default();
        r.set_recording(true);
        run(&mut r, 1.0);
        assert!(r.rings().is_empty());
        assert!(!r.active());
    }

    #[test]
    fn speech_sends_rings_out_and_they_stop_after_the_speech() {
        let mut r = Ripple::default();
        r.set_recording(true);
        r.cue(VoiceCue::Speech);
        assert!(r.active());
        run(&mut r, 1.0);
        assert!(r.rings().len() >= 2, "{:?}", r.rings());
        assert!(r.rings().iter().all(|&(t, a)| (0.0..1.0).contains(&t) && a > 0.0 && a <= 0.85));
        r.cue(VoiceCue::SpeechEnd);
        run(&mut r, 5.0);
        assert!(r.rings().is_empty());
        assert!(!r.active());
    }

    #[test]
    fn text_keeps_it_going() {
        let mut r = Ripple::default();
        r.set_recording(true);
        for _ in 0..10 {
            r.cue(VoiceCue::Text);
            run(&mut r, 0.3);
        }
        assert!(!r.rings().is_empty());
    }

    #[test]
    fn stopping_the_recording_lets_the_rings_fade() {
        let mut r = Ripple::default();
        r.set_recording(true);
        r.cue(VoiceCue::Speech);
        run(&mut r, 0.5);
        r.set_recording(false);
        r.cue(VoiceCue::Speech); // ignored
        run(&mut r, 3.0);
        assert!(!r.active());
    }

    #[test]
    fn only_the_known_activities_count() {
        assert_eq!(VoiceCue::from_activity("speechstart"), Some(VoiceCue::Speech));
        assert_eq!(VoiceCue::from_activity("soundstart"), Some(VoiceCue::Sound));
        assert_eq!(VoiceCue::from_activity("speechend"), Some(VoiceCue::SpeechEnd));
        assert_eq!(VoiceCue::from_activity("audiostart"), None);
        assert_eq!(VoiceCue::from_activity("nomatch"), None);
    }
}
