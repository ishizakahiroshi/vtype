//! The bubble that keeps words which did not go in because no text field had the focus (plan
//! C11): where its parts go and what a click hits, the same on every system. Each system measures
//! and draws the text itself (GDI, AppKit, GTK) and asks `layout` where everything goes; the
//! background and the buttons are `icon_draw::draw_kept`.
//!
//! Units are the system's: pixels at 100 % on Windows and GTK, points on macOS. The drawing is
//! scaled to the screen.

/// As wide as the speech bubble.
pub const WIDTH: f32 = 320.0;
pub const PADDING: f32 = 10.0;
/// The kept words show in at most this many lines (the latest words, as live text); the buttons
/// copy and insert all of them.
pub const TEXT_LINES: i32 = 4;
const GAP: f32 = 6.0;
const BUTTON_HEIGHT: f32 = 26.0;
const BUTTON_PADDING: f32 = 12.0;
const BUTTON_GAP: f32 = 8.0;
const CLOSE_SIZE: f32 = 20.0;
/// ✕ sits this far in from the top-right corner.
const CLOSE_INSET: f32 = 6.0;
/// A click this close to ✕ still counts: it is small.
const CLOSE_SLOP: f32 = 4.0;

/// How wide the caption may be (✕ sits beside it), and the words.
pub const CAPTION_WIDTH: f32 = WIDTH - PADDING * 2.0 - CLOSE_SIZE - GAP;
pub const TEXT_WIDTH: f32 = WIDTH - PADDING * 2.0;

/// Colors of the text each system draws, as RGB.
pub const CAPTION_COLOR: (u8, u8, u8) = (0x6b, 0x72, 0x80);
pub const TEXT_COLOR: (u8, u8, u8) = (0x1f, 0x29, 0x37);
pub const COPY_LABEL_COLOR: (u8, u8, u8) = (0x1f, 0x29, 0x37);
pub const INSERT_LABEL_COLOR: (u8, u8, u8) = (0xff, 0xff, 0xff);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptButton {
    /// The words go to the clipboard.
    Copy,
    /// The words go into whatever has the focus now.
    Insert,
    /// The words are thrown away.
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    fn grown(&self, by: f32) -> Area {
        Area { x: self.x - by, y: self.y - by, width: self.width + by * 2.0, height: self.height + by * 2.0 }
    }
}

/// Where the parts of the bubble go, from its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    /// Why the words are here.
    pub caption: Area,
    pub text: Area,
    pub copy: Area,
    pub insert: Area,
    pub close: Area,
}

/// Where everything goes, from the measured heights of the caption and the words (wrapped at
/// `CAPTION_WIDTH` and `TEXT_WIDTH`) and the widths of the two buttons' labels.
pub fn layout(caption_height: f32, text_height: f32, copy_label: f32, insert_label: f32) -> Layout {
    let caption = Area { x: PADDING, y: PADDING, width: CAPTION_WIDTH, height: caption_height };
    let close_bottom = CLOSE_INSET + CLOSE_SIZE;
    let text = Area {
        x: PADDING,
        y: (caption.y + caption.height).max(close_bottom) + GAP,
        width: TEXT_WIDTH,
        height: text_height,
    };
    let row = text.y + text.height + GAP + 2.0;
    let copy = Area { x: PADDING, y: row, width: copy_label + BUTTON_PADDING * 2.0, height: BUTTON_HEIGHT };
    let insert = Area {
        x: copy.x + copy.width + BUTTON_GAP,
        y: row,
        width: insert_label + BUTTON_PADDING * 2.0,
        height: BUTTON_HEIGHT,
    };
    // Wider only for labels a translation made long.
    let width = WIDTH.max(insert.x + insert.width + PADDING);
    let close = Area { x: width - CLOSE_INSET - CLOSE_SIZE, y: CLOSE_INSET, width: CLOSE_SIZE, height: CLOSE_SIZE };
    Layout { width, height: row + BUTTON_HEIGHT + PADDING, caption, text, copy, insert, close }
}

impl Layout {
    /// The button at (x, y) from the bubble's top-left corner.
    pub fn button_at(&self, x: f32, y: f32) -> Option<KeptButton> {
        if self.close.grown(CLOSE_SLOP).contains(x, y) {
            Some(KeptButton::Close)
        } else if self.copy.contains(x, y) {
            Some(KeptButton::Copy)
        } else if self.insert.contains(x, y) {
            Some(KeptButton::Insert)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Layout {
        layout(16.0, 38.0, 30.0, 90.0)
    }

    #[test]
    fn stacks_caption_words_and_buttons() {
        let l = sample();
        assert_eq!(l.width, WIDTH);
        assert!(l.text.y >= l.caption.y + l.caption.height);
        assert!(l.copy.y >= l.text.y + l.text.height);
        assert_eq!(l.copy.y, l.insert.y);
        assert!(l.insert.x >= l.copy.x + l.copy.width);
        assert_eq!(l.height, l.copy.y + l.copy.height + PADDING);
        // ✕ in the top-right corner, beside the caption.
        assert!(l.close.x >= l.caption.x + l.caption.width);
        assert_eq!(l.close.x + l.close.width, WIDTH - CLOSE_INSET);
    }

    #[test]
    fn a_short_caption_still_clears_the_close_button() {
        let l = layout(4.0, 20.0, 30.0, 90.0);
        assert!(l.text.y >= l.close.y + l.close.height);
    }

    #[test]
    fn long_labels_widen_the_bubble_and_keep_close_in_the_corner() {
        let l = layout(16.0, 20.0, 150.0, 200.0);
        assert!(l.width > WIDTH);
        assert_eq!(l.insert.x + l.insert.width + PADDING, l.width);
        assert_eq!(l.close.x + l.close.width, l.width - CLOSE_INSET);
    }

    #[test]
    fn finds_the_button_under_a_click() {
        let l = sample();
        let middle = |a: Area| (a.x + a.width / 2.0, a.y + a.height / 2.0);
        let (x, y) = middle(l.copy);
        assert_eq!(l.button_at(x, y), Some(KeptButton::Copy));
        let (x, y) = middle(l.insert);
        assert_eq!(l.button_at(x, y), Some(KeptButton::Insert));
        let (x, y) = middle(l.close);
        assert_eq!(l.button_at(x, y), Some(KeptButton::Close));
        // Just outside ✕ still closes; the words and the gap between the buttons do nothing.
        assert_eq!(l.button_at(l.close.x - 2.0, l.close.y + 2.0), Some(KeptButton::Close));
        let (x, y) = middle(l.text);
        assert_eq!(l.button_at(x, y), None);
        assert_eq!(l.button_at(l.copy.x + l.copy.width + 1.0, l.copy.y + 5.0), None);
    }
}
