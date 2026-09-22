//! The floating mic, drawn in software so every OS shows the same pixels (child plan C5-C3):
//! idle is a grey disc at 40 % opacity (opaque under the pointer), recording is orange with a
//! ring, and "just typed" is green with a check mark.

use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::platform::IconState;

pub const IDLE_OPACITY: f32 = 0.4;
pub const IDLE_RGB: (u8, u8, u8) = (0x4b, 0x55, 0x63);
pub const RECORDING_RGB: (u8, u8, u8) = (0xff, 0x7a, 0x3d);
pub const DONE_RGB: (u8, u8, u8) = (0x2e, 0x7d, 0x4f);

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

/// Premultiplied RGBA → premultiplied BGRA (what Windows' layered windows take).
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

    fn center(pm: &Pixmap) -> tiny_skia::ColorU8 {
        // Just left of the mic glyph, inside the disc.
        pm.pixel(pm.width() * 3 / 10, pm.height() / 2).unwrap().demultiply()
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
