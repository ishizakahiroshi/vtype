//! What the floating mic does with the pointer, and where it goes: the same rules on every OS.

use crate::platform::Rect;

/// Moving less than this between press and release is a click, not a drag.
pub const DRAG_THRESHOLD: i32 = 4;
/// The floating mic's window at 100 % scale: the mic in the middle, room for the ripple around it.
pub const ICON_SIZE: i32 = 56;
/// The mic itself at 100 % scale.
pub const MIC_SIZE: i32 = 40;
/// Distance from the bottom-right corner of the work area to the window, so the mic itself sits
/// 16 px from it.
pub const MARGIN: i32 = 16 - (ICON_SIZE - MIC_SIZE) / 2;

/// The floating mic's size the user chose, in percent of `ICON_SIZE`: typed on the settings page,
/// or Ctrl+wheel over the mic in `SCALE_STEP`s.
pub const SCALE_MIN: u16 = 50;
pub const SCALE_MAX: u16 = 200;
pub const SCALE_DEFAULT: u16 = 100;
pub const SCALE_STEP: u16 = 10;

pub fn clamp_scale(percent: i64) -> u16 {
    percent.clamp(SCALE_MIN.into(), SCALE_MAX.into()) as u16
}

/// `steps` wheel notches from `percent` (up is bigger), landing on multiples of `SCALE_STEP` so a
/// typed 95 goes to 100 or 90, not 105 or 85.
pub fn zoom_scale(percent: u16, steps: i32) -> u16 {
    let (p, step) = (i64::from(percent), i64::from(SCALE_STEP));
    let next = match steps.signum() {
        1 => p.div_euclid(step) * step + step * i64::from(steps),
        -1 => (p + step - 1).div_euclid(step) * step + step * i64::from(steps),
        _ => p,
    };
    clamp_scale(next)
}

/// The window's side at `percent`, in the same unit as `ICON_SIZE` times `dpi`.
pub fn scaled_size(percent: u16, dpi: f64) -> i32 {
    (f64::from(ICON_SIZE) * dpi * f64::from(percent) / 100.0).round().max(1.0) as i32
}

/// Wheel turns come in small pieces from touchpads; a notch is `unit` of them.
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelSteps {
    acc: f64,
}

impl WheelSteps {
    /// Adds `delta` (positive = away from the user) and returns the whole notches it completed.
    pub fn add(&mut self, delta: f64, unit: f64) -> i32 {
        self.acc += delta / unit;
        let steps = self.acc.trunc();
        self.acc -= steps;
        steps as i32
    }
}

/// Where the mic goes when its size changes from `old` to `new`: at the corner when it was never
/// moved, else with its centre kept (the pointer stays on it) and kept on that screen.
pub fn resized_position(
    pos: (i32, i32),
    old: i32,
    new: i32,
    at_default: bool,
    work_areas: &[Rect],
    primary: Rect,
) -> (i32, i32) {
    if at_default {
        let area = work_areas
            .iter()
            .copied()
            .find(|a| pos.0 >= a.x && pos.0 < a.x + a.width && pos.1 >= a.y && pos.1 < a.y + a.height)
            .unwrap_or(primary);
        return default_position(area, new);
    }
    let centred = (pos.0 + (old - new) / 2, pos.1 + (old - new) / 2);
    crate::beside_field::keep_on_screen(centred, new, work_areas, primary)
}

/// The small buttons at the floating mic's corners: the templates top left, the input mode top
/// right (a click moves to the next mode), send bottom right, clear bottom left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicButton {
    Templates,
    Mode,
    Send,
    Clear,
}

/// A button's radius, as a fraction of the window.
pub const BUTTON_RADIUS: f32 = 0.16;

impl MicButton {
    pub const ALL: [MicButton; 4] = [MicButton::Templates, MicButton::Mode, MicButton::Send, MicButton::Clear];

    /// Its centre, as fractions of the window (x, y from the top left).
    pub fn center(self) -> (f32, f32) {
        match self {
            MicButton::Templates => (0.2, 0.2),
            MicButton::Mode => (0.8, 0.2),
            MicButton::Send => (0.8, 0.8),
            MicButton::Clear => (0.2, 0.8),
        }
    }
}

/// Which button is under (`x`, `y`), window-local in the same unit as `size`. Only the buttons
/// on screen count (`shown`); anywhere else is the mic.
pub fn button_at(x: f32, y: f32, size: f32, shown: impl Fn(MicButton) -> bool) -> Option<MicButton> {
    MicButton::ALL.into_iter().filter(|&b| shown(b)).find(|&b| {
        let (cx, cy) = b.center();
        let (dx, dy) = (x - cx * size, y - cy * size);
        // A little more than drawn, so a quick click on the edge still lands.
        dx * dx + dy * dy <= (BUTTON_RADIUS * 1.15 * size).powi(2)
    })
}

/// Which buttons are on screen: all of them while the pointer is on the mic or it is recording or
/// has just typed; otherwise only the mode, and only when it is not the normal one.
pub fn button_shown(button: MicButton, busy_or_hovered: bool, mode_is_normal: bool) -> bool {
    busy_or_hovered || (button == MicButton::Mode && !mode_is_normal)
}

/// What the pointer is on, over the floating mic: the hovered button is drawn lighter, and the
/// bubble says what the part does once the pointer rests on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicPart {
    Mic,
    Button(MicButton),
}

/// The part under (`x`, `y`), window-local in the same unit as `size`, while the pointer is on
/// the window (so all four buttons are on screen).
pub fn part_at(x: f32, y: f32, size: f32) -> MicPart {
    button_at(x, y, size, |_| true).map_or(MicPart::Mic, MicPart::Button)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Click,
    Drag,
}

/// One press of the mouse button on the icon.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    start: (i32, i32),
    dragging: bool,
}

impl Press {
    pub fn new(at: (i32, i32)) -> Press {
        Press { start: at, dragging: false }
    }

    /// The pointer moved; true once this press has become a drag (and stays one).
    pub fn moved(&mut self, to: (i32, i32)) -> bool {
        if !self.dragging {
            let (dx, dy) = (to.0 - self.start.0, to.1 - self.start.1);
            self.dragging = dx.abs() >= DRAG_THRESHOLD || dy.abs() >= DRAG_THRESHOLD;
        }
        self.dragging
    }

    pub fn offset(&self, to: (i32, i32)) -> (i32, i32) {
        (to.0 - self.start.0, to.1 - self.start.1)
    }

    pub fn release(mut self, at: (i32, i32)) -> Gesture {
        if self.moved(at) {
            Gesture::Drag
        } else {
            Gesture::Click
        }
    }
}

/// Bottom-right of `work`, `MARGIN` inside it.
pub fn default_position(work: Rect, size: i32) -> (i32, i32) {
    (work.x + work.width - MARGIN - size, work.y + work.height - MARGIN - size)
}

/// Whether the icon at `pos` lies wholly inside one of the work areas.
pub fn fits(pos: (i32, i32), size: i32, work_areas: &[Rect]) -> bool {
    work_areas
        .iter()
        .any(|w| pos.0 >= w.x && pos.1 >= w.y && pos.0 + size <= w.x + w.width && pos.1 + size <= w.y + w.height)
}

/// The saved position if it is still on a screen (a monitor may have gone), else the default.
pub fn resolve_position(saved: Option<(i32, i32)>, size: i32, work_areas: &[Rect], primary: Rect) -> (i32, i32) {
    match saved {
        Some(pos) if fits(pos, size, work_areas) => pos,
        _ => default_position(primary, size),
    }
}

/// The bubble shows the end of what is being said: when it is too long, the start goes.
pub fn tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let keep: String = text.chars().skip(count - (max_chars - 1)).collect();
    format!("…{keep}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, width: 1920, height: 1040 };
    const SECOND: Rect = Rect { x: 1920, y: 0, width: 1280, height: 1024 };

    #[test]
    fn small_moves_are_clicks() {
        let p = Press::new((100, 100));
        assert_eq!(p.release((103, 97)), Gesture::Click);
        let mut p = Press::new((100, 100));
        assert!(!p.moved((102, 102)));
        assert!(p.moved((104, 100)));
        // Once a drag, coming back does not make it a click.
        assert!(p.moved((100, 100)));
        assert_eq!(p.release((100, 100)), Gesture::Drag);
        assert_eq!(Press::new((0, 0)).offset((5, -3)), (5, -3));
    }

    #[test]
    fn sits_bottom_right_by_default() {
        assert_eq!(default_position(SCREEN, 40), (1920 - MARGIN - 40, 1040 - MARGIN - 40));
        // The mic in the middle of the window is 16 px from the corner.
        let (x, _) = default_position(SCREEN, ICON_SIZE);
        assert_eq!(1920 - (x + (ICON_SIZE + MIC_SIZE) / 2), 16);
    }

    #[test]
    fn keeps_a_saved_position_only_while_it_is_on_a_screen() {
        let areas = [SCREEN, SECOND];
        assert_eq!(resolve_position(Some((2000, 500)), 40, &areas, SCREEN), (2000, 500));
        // The second monitor was unplugged.
        assert_eq!(resolve_position(Some((2000, 500)), 40, &[SCREEN], SCREEN), default_position(SCREEN, 40));
        // Half off the edge.
        assert_eq!(resolve_position(Some((1900, 10)), 40, &[SCREEN], SCREEN), default_position(SCREEN, 40));
        assert_eq!(resolve_position(None, 40, &areas, SCREEN), default_position(SCREEN, 40));
    }

    #[test]
    fn the_corners_are_buttons_only_while_they_are_shown() {
        let s = ICON_SIZE as f32;
        let all = |_| true;
        assert_eq!(button_at(0.8 * s, 0.2 * s, s, all), Some(MicButton::Mode));
        assert_eq!(button_at(0.8 * s, 0.8 * s, s, all), Some(MicButton::Send));
        assert_eq!(button_at(0.2 * s, 0.8 * s, s, all), Some(MicButton::Clear));
        assert_eq!(button_at(0.2 * s, 0.2 * s, s, all), Some(MicButton::Templates));
        // The middle is the mic.
        assert_eq!(button_at(0.5 * s, 0.5 * s, s, all), None);
        // Hidden buttons are the mic.
        let shown = |b| button_shown(b, false, true);
        assert_eq!(button_at(0.8 * s, 0.8 * s, s, shown), None);
        assert_eq!(button_at(0.8 * s, 0.2 * s, s, shown), None);
        assert_eq!(button_at(0.2 * s, 0.2 * s, s, shown), None);
        // The mode shows by itself when it is not the normal one.
        assert!(button_shown(MicButton::Mode, false, false));
        assert!(!button_shown(MicButton::Send, false, false));
        assert!(button_shown(MicButton::Clear, true, true));
    }

    #[test]
    fn the_pointer_is_on_a_button_or_else_on_the_mic() {
        let s = ICON_SIZE as f32;
        assert_eq!(part_at(0.5 * s, 0.5 * s, s), MicPart::Mic);
        assert_eq!(part_at(0.2 * s, 0.8 * s, s), MicPart::Button(MicButton::Clear));
        assert_eq!(part_at(0.8 * s, 0.2 * s, s), MicPart::Button(MicButton::Mode));
        // Between a button and the mic's disc.
        assert_eq!(part_at(0.5 * s, 0.05 * s, s), MicPart::Mic);
    }

    #[test]
    fn the_wheel_zooms_in_tens_between_the_limits() {
        assert_eq!(zoom_scale(100, 1), 110);
        assert_eq!(zoom_scale(100, -1), 90);
        assert_eq!(zoom_scale(100, 3), 130);
        // A typed size lands on the next ten each way.
        assert_eq!(zoom_scale(95, 1), 100);
        assert_eq!(zoom_scale(95, -1), 90);
        assert_eq!(zoom_scale(195, 5), SCALE_MAX);
        assert_eq!(zoom_scale(SCALE_MIN, -1), SCALE_MIN);
        assert_eq!(zoom_scale(120, 0), 120);
        assert_eq!(clamp_scale(10), SCALE_MIN);
        assert_eq!(clamp_scale(1000), SCALE_MAX);
        assert_eq!(scaled_size(100, 1.0), ICON_SIZE);
        assert_eq!(scaled_size(50, 1.0), ICON_SIZE / 2);
        assert_eq!(scaled_size(200, 1.5), ICON_SIZE * 3);
    }

    #[test]
    fn touchpad_pieces_add_up_to_notches() {
        let mut w = WheelSteps::default();
        assert_eq!(w.add(40.0, 120.0), 0);
        assert_eq!(w.add(40.0, 120.0), 0);
        assert_eq!(w.add(40.0, 120.0), 1);
        assert_eq!(w.add(-240.0, 120.0), -2);
        assert_eq!(w.add(1.0, 1.0), 1);
    }

    #[test]
    fn a_resized_mic_keeps_its_centre_or_its_corner() {
        let areas = [SCREEN, SECOND];
        // Moved by the user: the centre stays.
        assert_eq!(resized_position((500, 500), 56, 112, false, &areas, SCREEN), (472, 472));
        assert_eq!(resized_position((472, 472), 112, 56, false, &areas, SCREEN), (500, 500));
        // Growing at an edge stays on the screen.
        assert_eq!(resized_position((1920 - 56, 0), 56, 112, false, &areas, SCREEN), (1920 - 112, 0));
        // Never moved: it stays in the corner of the screen it is on.
        assert_eq!(
            resized_position(default_position(SECOND, 56), 56, 112, true, &areas, SCREEN),
            default_position(SECOND, 112)
        );
    }

    #[test]
    fn the_bubble_drops_the_start() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("abcdefghij", 5), "…ghij");
        assert_eq!(tail("あいうえおかきく", 4), "…かきく");
    }
}
