//! The floating mic, drawn in software so every OS shows the same pixels (child plan C5-C3):
//! idle is a grey disc at 40 % opacity (opaque under the pointer), recording is orange with a
//! ring, and "just typed" is green with a check mark.
//!
//! Its window (`draw_floating`) adds the ripple while the user speaks and four small buttons at
//! the corners: the templates top left (a list), the input mode top right ("A" English, "カ"
//! katakana, a circular arrow for the normal mode), send bottom right, clear bottom left. The
//! glyphs are drawn as strokes, so no font is needed.

use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::overlay_logic::{button_shown, MicButton, BUTTON_RADIUS, ICON_SIZE, MIC_SIZE};
use crate::platform::IconState;
use crate::protocol::InputMode;

pub const IDLE_OPACITY: f32 = 0.4;
pub const IDLE_RGB: (u8, u8, u8) = (0x4b, 0x55, 0x63);
pub const RECORDING_RGB: (u8, u8, u8) = (0xff, 0x7a, 0x3d);
pub const DONE_RGB: (u8, u8, u8) = (0x2e, 0x7d, 0x4f);
pub const MODE_RGB: (u8, u8, u8) = (0x4f, 0x46, 0xe5);
pub const SEND_RGB: (u8, u8, u8) = (0x63, 0x66, 0xf1);
pub const CLEAR_RGB: (u8, u8, u8) = (0x6b, 0x72, 0x80);
pub const TEMPLATES_RGB: (u8, u8, u8) = (0x0d, 0x94, 0x88);

fn paint(rgb: (u8, u8, u8), alpha: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(rgb.0, rgb.1, rgb.2, alpha);
    p.anti_alias = true;
    p
}

/// `size` × `size` pixels, premultiplied RGBA.
pub fn draw_icon(size: u32, state: IconState, hover: bool) -> Pixmap {
    let mut pm = Pixmap::new(size.max(8), size.max(8)).expect("non-zero size");
    let s = pm.width() as f32;
    let c = s / 2.0;
    let rgb = match state {
        IconState::Idle => IDLE_RGB,
        IconState::Recording => RECORDING_RGB,
        IconState::Done => DONE_RGB,
    };

    if state == IconState::Recording {
        if let Some(ring) = PathBuilder::from_circle(c, c, s * 0.47) {
            pm.stroke_path(
                &ring,
                &paint(rgb, 110),
                &Stroke { width: s * 0.05, ..Stroke::default() },
                Transform::identity(),
                None,
            );
        }
    }
    if let Some(disc) = PathBuilder::from_circle(c, c, s * 0.40) {
        pm.fill_path(&disc, &paint(rgb, 255), FillRule::Winding, Transform::identity(), None);
    }

    let white = paint((255, 255, 255), 255);
    let line = Stroke { width: (s * 0.06).max(1.5), line_cap: LineCap::Round, ..Stroke::default() };
    if state == IconState::Done {
        let mut pb = PathBuilder::new();
        pb.move_to(s * 0.33, s * 0.51);
        pb.line_to(s * 0.45, s * 0.63);
        pb.line_to(s * 0.68, s * 0.39);
        if let Some(check) = pb.finish() {
            let bold = Stroke { width: (s * 0.08).max(2.0), ..line.clone() };
            pm.stroke_path(&check, &white, &bold, Transform::identity(), None);
        }
    } else {
        // Capsule body.
        let (w, h) = (s * 0.16, s * 0.27);
        if let Some(body) = rounded_rect(c - w / 2.0, s * 0.25, w, h, w / 2.0) {
            pm.fill_path(&body, &white, FillRule::Winding, Transform::identity(), None);
        }
        // Holder, stem and foot.
        let mut pb = PathBuilder::new();
        pb.move_to(c - s * 0.15, s * 0.44);
        pb.quad_to(c - s * 0.15, s * 0.62, c, s * 0.62);
        pb.quad_to(c + s * 0.15, s * 0.62, c + s * 0.15, s * 0.44);
        pb.move_to(c, s * 0.62);
        pb.line_to(c, s * 0.72);
        pb.move_to(c - s * 0.09, s * 0.72);
        pb.line_to(c + s * 0.09, s * 0.72);
        if let Some(holder) = pb.finish() {
            pm.stroke_path(&holder, &white, &line, Transform::identity(), None);
        }
    }

    if state == IconState::Idle && !hover {
        fade(&mut pm, IDLE_OPACITY);
    }
    pm
}

/// The floating mic's whole window, `size` × `size` pixels: the mic (`draw_icon`, at
/// `MIC_SIZE / ICON_SIZE` of the window) in the middle, the ripple's `rings` (spread 0..1,
/// opacity 0..1, from `Ripple::rings`) spreading from its edge to the window's, and the corner
/// buttons that `button_shown` allows.
pub fn draw_floating(size: u32, state: IconState, hover: bool, mode: InputMode, rings: &[(f32, f32)]) -> Pixmap {
    let mut pm = Pixmap::new(size.max(8), size.max(8)).expect("non-zero size");
    let s = pm.width() as f32;
    let mic = ((s * MIC_SIZE as f32 / ICON_SIZE as f32).round() as u32).max(8);
    let from = mic as f32 * 0.40;
    let to = s * 0.48;
    for &(t, alpha) in rings {
        let alpha = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        if let Some(ring) = PathBuilder::from_circle(s / 2.0, s / 2.0, from + (to - from) * t) {
            let stroke = Stroke { width: (s * 0.035).max(1.0), ..Stroke::default() };
            pm.stroke_path(&ring, &paint(RECORDING_RGB, alpha), &stroke, Transform::identity(), None);
        }
    }
    let offset = ((pm.width() - mic) / 2) as i32;
    let icon = draw_icon(mic, state, hover);
    pm.draw_pixmap(offset, offset, icon.as_ref(), &PixmapPaint::default(), Transform::identity(), None);

    let busy_or_hovered = hover || state != IconState::Idle;
    let mut buttons = Pixmap::new(pm.width(), pm.height()).expect("non-zero size");
    for button in MicButton::ALL {
        if button_shown(button, busy_or_hovered, mode == InputMode::Normal) {
            draw_button(&mut buttons, button, mode);
        }
    }
    // Like the mic, the lone mode badge is faint until the pointer comes.
    let opacity = if busy_or_hovered { 1.0 } else { IDLE_OPACITY };
    let layer = PixmapPaint { opacity, ..PixmapPaint::default() };
    pm.draw_pixmap(0, 0, buttons.as_ref(), &layer, Transform::identity(), None);
    pm
}

/// One corner button: a disc with a white rim, and its glyph.
fn draw_button(pm: &mut Pixmap, button: MicButton, mode: InputMode) {
    let s = pm.width() as f32;
    let (fx, fy) = button.center();
    let (x, y, r) = (s * fx, s * fy, s * BUTTON_RADIUS);
    let rgb = match button {
        MicButton::Templates => TEMPLATES_RGB,
        MicButton::Mode => MODE_RGB,
        MicButton::Send => SEND_RGB,
        MicButton::Clear => CLEAR_RGB,
    };
    let white = paint((255, 255, 255), 255);
    if let Some(rim) = PathBuilder::from_circle(x, y, r) {
        pm.fill_path(&rim, &white, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(disc) = PathBuilder::from_circle(x, y, r - (s * 0.025).max(1.0)) {
        pm.fill_path(&disc, &paint(rgb, 255), FillRule::Winding, Transform::identity(), None);
    }
    // The glyph fits a square of half-side g around the centre.
    let g = r * 0.5;
    let stroke = Stroke {
        width: (s * 0.045).max(1.2),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let mut pb = PathBuilder::new();
    match (button, mode) {
        (MicButton::Mode, InputMode::En) => {
            pb.move_to(x - 0.8 * g, y + g);
            pb.line_to(x, y - g);
            pb.line_to(x + 0.8 * g, y + g);
            pb.move_to(x - 0.45 * g, y + 0.3 * g);
            pb.line_to(x + 0.45 * g, y + 0.3 * g);
        }
        (MicButton::Mode, InputMode::Kana) => {
            // カ: the bar with its hook, then the left-falling stroke.
            pb.move_to(x - 0.85 * g, y - 0.4 * g);
            pb.line_to(x + 0.75 * g, y - 0.4 * g);
            pb.line_to(x + 0.6 * g, y + 0.9 * g);
            pb.line_to(x + 0.25 * g, y + 0.7 * g);
            pb.move_to(x - 0.05 * g, y - g);
            pb.quad_to(x - 0.1 * g, y + 0.4 * g, x - 0.85 * g, y + g);
        }
        (MicButton::Mode, InputMode::Normal) => {
            // A circular arrow: "click to change the mode".
            let steps = 10;
            for i in 0..=steps {
                let a = (40.0 + 270.0 * i as f32 / steps as f32).to_radians();
                let (px, py) = (x + 0.85 * g * a.cos(), y - 0.85 * g * a.sin());
                if i == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            let end = 310f32.to_radians();
            let (ex, ey) = (x + 0.85 * g * end.cos(), y - 0.85 * g * end.sin());
            pb.move_to(ex - 0.55 * g, ey - 0.05 * g);
            pb.line_to(ex, ey);
            pb.line_to(ex + 0.05 * g, ey - 0.6 * g);
        }
        (MicButton::Templates, _) => {
            // A list: three lines, the last one shorter.
            for (dy, w) in [(-0.6, 0.75), (0.0, 0.75), (0.6, 0.35)] {
                pb.move_to(x - 0.75 * g, y + dy * g);
                pb.line_to(x - 0.75 * g + 2.0 * w * g, y + dy * g);
            }
        }
        (MicButton::Send, _) => {
            let mut tri = PathBuilder::new();
            tri.move_to(x - 0.65 * g, y - 0.85 * g);
            tri.line_to(x + 0.9 * g, y);
            tri.line_to(x - 0.65 * g, y + 0.85 * g);
            tri.close();
            if let Some(tri) = tri.finish() {
                pm.fill_path(&tri, &white, FillRule::Winding, Transform::identity(), None);
            }
        }
        (MicButton::Clear, _) => {
            pb.move_to(x - 0.7 * g, y - 0.7 * g);
            pb.line_to(x + 0.7 * g, y + 0.7 * g);
            pb.move_to(x + 0.7 * g, y - 0.7 * g);
            pb.line_to(x - 0.7 * g, y + 0.7 * g);
        }
    }
    if let Some(glyph) = pb.finish() {
        pm.stroke_path(&glyph, &white, &stroke, Transform::identity(), None);
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Rounded rectangle filled with `rgba`, for the speech bubble's body.
pub fn draw_rounded_panel(width: u32, height: u32, radius: f32, rgba: (u8, u8, u8, u8)) -> Pixmap {
    let mut pm = Pixmap::new(width.max(1), height.max(1)).expect("non-zero size");
    pm.fill(Color::TRANSPARENT);
    if let Some(path) = rounded_rect(0.5, 0.5, width as f32 - 1.0, height as f32 - 1.0, radius) {
        pm.fill_path(&path, &paint((rgba.0, rgba.1, rgba.2), rgba.3), FillRule::Winding, Transform::identity(), None);
    }
    pm
}

/// Multiplies every (premultiplied) channel, i.e. the whole image's opacity.
fn fade(pm: &mut Pixmap, opacity: f32) {
    for px in pm.data_mut().iter_mut() {
        *px = (*px as f32 * opacity).round() as u8;
    }
}

/// Premultiplied RGBA → straight RGBA (what tray icons take).
pub fn to_straight_rgba(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

/// Premultiplied RGBA → premultiplied BGRA (what Windows' layered windows and cairo's ARGB32
/// take). macOS draws through a PNG instead.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn to_premultiplied_bgra(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for &[r, g, b, a] in pm.data().as_chunks::<4>().0 {
        out.extend_from_slice(&[b, g, r, a]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a check: draws every look of the floating mic into one PNG to look at by eye, since the
    /// pixel tests cannot say whether a glyph reads well.
    /// `cargo test icon_sheet -- --ignored`, then open `vtype-icon-sheet.png` in the temp folder
    /// (or the path in `VTYPE_ICON_SHEET`).
    #[test]
    #[ignore]
    fn icon_sheet() {
        use crate::ripple::{Ripple, VoiceCue};
        let mut ripple = Ripple::default();
        ripple.set_recording(true);
        ripple.cue(VoiceCue::Speech);
        for _ in 0..12 {
            ripple.tick(0.033);
        }
        let rings = ripple.rings();
        let none = Vec::new();
        let looks = [
            (IconState::Idle, false, InputMode::Normal, &none),
            (IconState::Idle, true, InputMode::Normal, &none),
            (IconState::Idle, true, InputMode::En, &none),
            (IconState::Idle, true, InputMode::Kana, &none),
            (IconState::Recording, false, InputMode::Kana, &rings),
            (IconState::Done, false, InputMode::Normal, &none),
        ];
        let (big, gap, small) = (168u32, 12u32, ICON_SIZE as u32);
        let mut sheet = Pixmap::new((big + gap) * looks.len() as u32 + gap, big + small + gap * 3).unwrap();
        sheet.fill(Color::from_rgba8(30, 30, 30, 255));
        for (i, (state, hover, mode, rings)) in looks.iter().enumerate() {
            let x = (gap + (big + gap) * i as u32) as i32;
            for (size, y) in [(big, gap), (small, big + gap * 2)] {
                let pm = draw_floating(size, *state, *hover, *mode, rings);
                sheet.draw_pixmap(x, y as i32, pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        }
        let path = std::env::var_os("VTYPE_ICON_SHEET")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("vtype-icon-sheet.png"));
        sheet.save_png(&path).unwrap();
        println!("wrote {}", path.display());
    }

    fn center(pm: &Pixmap) -> tiny_skia::ColorU8 {
        // Just left of the mic glyph, inside the disc.
        pm.pixel(pm.width() * 3 / 10, pm.height() / 2).unwrap().demultiply()
    }

    /// Inside a corner button, off its glyph (between the centre and the rim, towards the top).
    fn button_pixel(pm: &Pixmap, button: MicButton) -> tiny_skia::ColorU8 {
        let s = pm.width() as f32;
        let (fx, fy) = button.center();
        let (x, y) = (s * fx, s * (fy - BUTTON_RADIUS * 0.75));
        pm.pixel(x as u32, y as u32).unwrap().demultiply()
    }

    #[test]
    fn the_buttons_show_while_hovered_or_busy() {
        let hovered = draw_floating(56, IconState::Idle, true, InputMode::Normal, &[]);
        for (button, rgb) in [
            (MicButton::Templates, TEMPLATES_RGB),
            (MicButton::Mode, MODE_RGB),
            (MicButton::Send, SEND_RGB),
            (MicButton::Clear, CLEAR_RGB),
        ] {
            let c = button_pixel(&hovered, button);
            assert_eq!((c.red(), c.green(), c.blue()), rgb, "{button:?}");
        }
        let recording = draw_floating(56, IconState::Recording, false, InputMode::Normal, &[]);
        assert_eq!(button_pixel(&recording, MicButton::Send).alpha(), 255);
        // Idle and not hovered: no buttons, and the normal mode has no badge.
        let idle = draw_floating(56, IconState::Idle, false, InputMode::Normal, &[]);
        for button in MicButton::ALL {
            assert_eq!(button_pixel(&idle, button).alpha(), 0, "{button:?}");
        }
    }

    #[test]
    fn the_mode_badge_stays_for_english_and_katakana_and_the_glyphs_differ() {
        let en = draw_floating(56, IconState::Idle, false, InputMode::En, &[]);
        let c = button_pixel(&en, MicButton::Mode);
        // Faded, so the colour comes back only to within rounding.
        let near = |a: u8, b: u8| a.abs_diff(b) <= 4;
        assert!(near(c.red(), MODE_RGB.0) && near(c.green(), MODE_RGB.1) && near(c.blue(), MODE_RGB.2), "{c:?}");
        assert!(c.alpha() > 0 && c.alpha() < 255, "faint while idle: {}", c.alpha());
        assert_eq!(button_pixel(&en, MicButton::Send).alpha(), 0);
        let modes = [InputMode::Normal, InputMode::En, InputMode::Kana];
        let drawn: Vec<_> = modes.iter().map(|&m| draw_floating(56, IconState::Idle, true, m, &[])).collect();
        assert_ne!(drawn[0].data(), drawn[1].data());
        assert_ne!(drawn[1].data(), drawn[2].data());
        assert_ne!(drawn[0].data(), drawn[2].data());
    }

    #[test]
    fn the_floating_window_has_the_mic_in_the_middle_and_the_rings_around_it() {
        let quiet = draw_floating(56, IconState::Recording, false, InputMode::Normal, &[]);
        // The mic (40 px, 8 px in) is orange left of its glyph; the window's edge is empty.
        let c = quiet.pixel(8 + 40 * 3 / 10, 28).unwrap().demultiply();
        assert_eq!((c.red(), c.green(), c.blue()), RECORDING_RGB);
        assert_eq!(quiet.pixel(1, 28).unwrap().alpha(), 0);
        // A ring near the window's edge.
        let rippling = draw_floating(56, IconState::Recording, false, InputMode::Normal, &[(1.0, 0.7)]);
        let edge = rippling.pixel((56.0 * 0.02) as u32 + 1, 28).unwrap();
        assert!(edge.alpha() > 0, "no ring at the edge");
    }

    #[test]
    fn recording_is_orange() {
        let pm = draw_icon(40, IconState::Recording, false);
        let c = center(&pm);
        assert_eq!((c.red(), c.green(), c.blue()), RECORDING_RGB);
        assert_eq!(c.alpha(), 255);
    }

    #[test]
    fn idle_is_faint_until_hovered() {
        let idle = draw_icon(40, IconState::Idle, false);
        let a = center(&idle).alpha() as f32 / 255.0;
        assert!((a - IDLE_OPACITY).abs() < 0.02, "alpha {a}");
        let hovered = draw_icon(40, IconState::Idle, true);
        assert_eq!(center(&hovered).alpha(), 255);
    }

    #[test]
    fn done_is_green_and_corners_are_transparent() {
        let pm = draw_icon(40, IconState::Done, false);
        let c = pm.pixel(8, 20).unwrap().demultiply();
        assert_eq!((c.red(), c.green(), c.blue()), DONE_RGB);
        assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
    }

    #[test]
    fn converts_channel_order() {
        let pm = draw_icon(16, IconState::Recording, false);
        let bgra = to_premultiplied_bgra(&pm);
        let rgba = pm.data();
        assert_eq!(bgra.len(), rgba.len());
        assert_eq!((bgra[0], bgra[2]), (rgba[2], rgba[0]));
        assert_eq!(to_straight_rgba(&pm).len(), rgba.len());
    }
}
