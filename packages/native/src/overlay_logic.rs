//! What the floating mic does with the pointer, and where it goes: the same rules on every OS.

use crate::platform::Rect;

/// Moving less than this between press and release is a click, not a drag.
pub const DRAG_THRESHOLD: i32 = 4;
/// Distance from the bottom-right corner of the work area.
pub const MARGIN: i32 = 16;
/// The icon's size at 100 % scale.
pub const ICON_SIZE: i32 = 40;

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
        assert_eq!(default_position(SCREEN, 40), (1920 - 16 - 40, 1040 - 16 - 40));
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
    fn the_bubble_drops_the_start() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("abcdefghij", 5), "…ghij");
        assert_eq!(tail("あいうえおかきく", 4), "…かきく");
    }
}
