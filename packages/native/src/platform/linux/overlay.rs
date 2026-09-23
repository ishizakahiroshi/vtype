//! The floating mic and its speech bubble on X11 (child plan C7-C3): GTK popup windows, which X11
//! makes override-redirect (no window manager frame, never given the focus), with an RGBA visual
//! so the corners are see-through where a compositor runs. Wayland gets neither: an app cannot
//! place its own window there.
//!
//! GTK rather than raw X11 for these, because the tray already runs GTK's loop and GTK's labels
//! draw the bubble's text (X11 core fonts would not show Japanese).
//!
//! Everything here runs on the GTK thread. The event handlers only touch `IconShared` (Cells),
//! never the `Ui` borrowed by the job queue, the same split as on Windows and macOS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use tiny_skia::Pixmap;
use tray_icon::menu::{ContextMenu, Menu};

use crate::icon_draw::{draw_floating, draw_rounded_panel, to_premultiplied_bgra};
use crate::overlay_logic::{
    button_at, button_shown, fits, part_at, resized_position, resolve_position, scaled_size, tail, Gesture, MicButton,
    MicPart, Press, WheelSteps, SCALE_DEFAULT,
};
use crate::platform::{IconState, PlatformEvent, Rect, VoiceCue};
use crate::protocol::InputMode;
use crate::ripple::{Ripple, FRAME};

const BUBBLE_WIDTH: i32 = 320;
const BUBBLE_PADDING: i32 = 10;
/// Pango sizes are in 1024ths of a point.
const BUBBLE_FONT_SIZE: i32 = 10 * 1024;

fn new_popup(width: i32, height: i32) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.set_decorated(false);
    window.set_accept_focus(false);
    window.set_focus_on_map(false);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_default_size(width, height);
    window.resize(width, height);
    window
}

/// Clears the window to transparent and paints the pixmap (drawn at `scale` pixels per point).
fn paint(cr: &cairo::Context, pm: &Pixmap, scale: f64) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    // Premultiplied BGRA is cairo's ARGB32 on little-endian machines; the stride is w * 4.
    let Ok(surface) =
        cairo::ImageSurface::create_for_data(to_premultiplied_bgra(pm), cairo::Format::ARgb32, w, h, w * 4)
    else {
        return;
    };
    surface.set_device_scale(scale, scale);
    cr.set_operator(cairo::Operator::Over);
    if cr.set_source_surface(&surface, 0.0, 0.0).is_ok() {
        let _ = cr.paint();
    }
}

/// Every monitor's work area (without panels and docks), primary first when there is one.
pub fn work_areas() -> (Vec<Rect>, Rect) {
    let Some(display) = gdk::Display::default() else {
        return (Vec::new(), Rect::default());
    };
    let to_rect = |r: gdk::Rectangle| Rect { x: r.x(), y: r.y(), width: r.width(), height: r.height() };
    let areas: Vec<Rect> =
        (0..display.n_monitors()).filter_map(|i| display.monitor(i)).map(|m| to_rect(m.workarea())).collect();
    let primary =
        display.primary_monitor().map(|m| to_rect(m.workarea())).or_else(|| areas.first().copied()).unwrap_or_default();
    (areas, primary)
}

fn root_point(p: (f64, f64)) -> (i32, i32) {
    (p.0.round() as i32, p.1.round() as i32)
}

struct IconShared {
    events: Sender<PlatformEvent>,
    window: gtk::Window,
    menu: Menu,
    /// The templates menu, rebuilt by the UI each time it opens.
    templates_menu: RefCell<Option<Menu>>,
    look: Cell<IconState>,
    /// The input mode, shown as a badge.
    mode: Cell<InputMode>,
    ripple: RefCell<Ripple>,
    /// Whether the ripple's timer runs, and when it last ticked.
    animating: Cell<bool>,
    last_frame: Cell<Instant>,
    /// What the pointer is on (`None`: off the mic).
    pointer: Cell<Option<MicPart>>,
    press: Cell<Option<Press>>,
    press_origin: Cell<(i32, i32)>,
    /// The corner button the press started on, if any.
    press_button: Cell<Option<MicButton>>,
    pos: Cell<(i32, i32)>,
    /// The window's side, and the size the user chose in percent.
    size: Cell<i32>,
    percent: Cell<u16>,
    /// Still where it goes by default (never dragged): a new size keeps it in the corner.
    at_default: Cell<bool>,
    wheel: Cell<WheelSteps>,
}

thread_local! {
    static ICON: RefCell<Option<Rc<IconShared>>> = const { RefCell::new(None) };
}

/// Opens the templates menu at the mic. Called from GTK's loop, not from a UI job.
pub fn show_templates_menu() {
    with_icon(|s| {
        let menu = s.templates_menu.borrow().clone();
        if let Some(menu) = menu {
            menu.show_context_menu_for_gtk_window(&s.window, None);
        }
    });
}

fn with_icon(f: impl FnOnce(&IconShared)) {
    let icon = ICON.with(|slot| slot.borrow().clone());
    if let Some(s) = icon {
        f(&s);
    }
}

impl IconShared {
    fn draw(&self, cr: &cairo::Context) {
        let scale = self.window.scale_factor().max(1) as f64;
        let px = (f64::from(self.size.get()) * scale).round() as u32;
        let rings = self.ripple.borrow().rings();
        paint(cr, &draw_floating(px, self.look.get(), self.pointer.get(), self.mode.get(), &rings), scale);
    }

    fn place(&self) {
        let (x, y) = self.pos.get();
        self.window.move_(x, y);
    }

    fn resize(&self, size: i32) {
        self.size.set(size);
        self.window.resize(size, size);
    }

    fn zoom(&self, delta: f64) {
        let mut wheel = self.wheel.get();
        let steps = wheel.add(delta, 1.0);
        self.wheel.set(wheel);
        if steps != 0 {
            let _ = self.events.send(PlatformEvent::IconZoom { steps });
        }
    }

    fn press(&self, at: (i32, i32)) {
        self.press.set(Some(Press::new(at)));
        self.press_origin.set(self.pos.get());
        self.press_button.set(self.button_under(at));
    }

    /// The corner button under the root point `at`, among those on screen now.
    fn button_under(&self, at: (i32, i32)) -> Option<MicButton> {
        let (x, y) = self.pos.get();
        let busy_or_hovered = self.pointer.get().is_some() || self.look.get() != IconState::Idle;
        let normal = self.mode.get() == InputMode::Normal;
        button_at((at.0 - x) as f32, (at.1 - y) as f32, self.size.get() as f32, |b| {
            button_shown(b, busy_or_hovered, normal)
        })
    }

    fn drag(&self, now: (i32, i32)) {
        if let Some(mut press) = self.press.get() {
            if press.moved(now) {
                let (dx, dy) = press.offset(now);
                let (ox, oy) = self.press_origin.get();
                self.pos.set((ox + dx, oy + dy));
                self.place();
            }
            self.press.set(Some(press));
        }
    }

    fn release(&self, at: (i32, i32)) {
        if let Some(press) = self.press.take() {
            let event = match press.release(at) {
                Gesture::Click => match self.press_button.take() {
                    Some(button) => PlatformEvent::MicButton(button),
                    None => PlatformEvent::ToggleRequested,
                },
                Gesture::Drag => {
                    self.at_default.set(false);
                    let (x, y) = self.pos.get();
                    PlatformEvent::IconMoved { x, y }
                }
            };
            let _ = self.events.send(event);
        }
    }

    /// The pointer is at the root point `at` over the mic: a new part is drawn and reported. While
    /// pressed the mic moves with the pointer, so the part stays the one pressed.
    fn point_at(&self, at: (i32, i32)) {
        if self.press.get().is_some() && self.pointer.get().is_some() {
            return;
        }
        let (x, y) = self.pos.get();
        let part = part_at((at.0 - x) as f32, (at.1 - y) as f32, self.size.get() as f32);
        if self.pointer.replace(Some(part)) != Some(part) {
            self.window.queue_draw();
            let _ = self.events.send(PlatformEvent::MicHover(Some(part)));
        }
    }

    fn pointer_left(&self) {
        if self.pointer.take().is_some() {
            self.window.queue_draw();
            let _ = self.events.send(PlatformEvent::MicHover(None));
        }
    }

    /// One step of the ripple; false (and the timer stops) once nothing is left to draw.
    fn frame(&self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame.replace(now)).as_secs_f32();
        self.ripple.borrow_mut().tick(dt);
        self.window.queue_draw();
        let active = self.ripple.borrow().active();
        self.animating.set(active);
        active
    }
}

fn wire_icon(window: &gtk::Window) {
    window.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::ENTER_NOTIFY_MASK
            | gdk::EventMask::LEAVE_NOTIFY_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK,
    );
    window.connect_draw(|_, cr| {
        with_icon(|s| s.draw(cr));
        glib::Propagation::Stop
    });
    window.connect_realize(|w| {
        if let (Some(gdk_window), Some(display)) = (w.window(), gdk::Display::default()) {
            gdk_window.set_cursor(gdk::Cursor::for_display(&display, gdk::CursorType::Hand2).as_ref());
        }
    });
    window.connect_button_press_event(|w, ev| {
        match ev.button() {
            1 => with_icon(|s| s.press(root_point(ev.root()))),
            3 => {
                let menu = ICON.with(|slot| slot.borrow().as_ref().map(|s| s.menu.clone()));
                if let Some(menu) = menu {
                    menu.show_context_menu_for_gtk_window(w, None);
                }
            }
            _ => {}
        }
        glib::Propagation::Stop
    });
    window.connect_motion_notify_event(|_, ev| {
        with_icon(|s| {
            s.point_at(root_point(ev.root()));
            s.drag(root_point(ev.root()));
        });
        glib::Propagation::Stop
    });
    window.connect_button_release_event(|_, ev| {
        if ev.button() == 1 {
            with_icon(|s| s.release(root_point(ev.root())));
        }
        glib::Propagation::Stop
    });
    // Ctrl+wheel resizes the mic. Up is away from the user: bigger.
    window.connect_scroll_event(|_, ev| {
        if ev.state().contains(gdk::ModifierType::CONTROL_MASK) {
            let delta = match ev.direction() {
                gdk::ScrollDirection::Up => 1.0,
                gdk::ScrollDirection::Down => -1.0,
                gdk::ScrollDirection::Smooth => -ev.delta().1,
                _ => 0.0,
            };
            with_icon(|s| s.zoom(delta));
        }
        glib::Propagation::Stop
    });
    window.connect_enter_notify_event(|_, ev| {
        with_icon(|s| s.point_at(root_point(ev.root())));
        glib::Propagation::Proceed
    });
    window.connect_leave_notify_event(|_, _| {
        with_icon(|s| s.pointer_left());
        glib::Propagation::Proceed
    });
}

struct Bubble {
    window: gtk::Window,
    label: gtk::Label,
    /// The panel behind the text, in points.
    size: Rc<Cell<(i32, i32)>>,
    shown: bool,
}

fn markup(text: &str) -> String {
    format!("<span foreground=\"#1f2937\" size=\"{BUBBLE_FONT_SIZE}\">{}</span>", glib::markup_escape_text(text))
}

pub struct Overlay {
    icon: Rc<IconShared>,
    bubble: Bubble,
    shown: bool,
}

impl Overlay {
    pub fn new(events: Sender<PlatformEvent>, menu: Menu) -> Overlay {
        let side = scaled_size(SCALE_DEFAULT, 1.0);
        let window = new_popup(side, side);
        wire_icon(&window);
        let icon = Rc::new(IconShared {
            events,
            window,
            menu,
            templates_menu: RefCell::new(None),
            look: Cell::new(IconState::Idle),
            mode: Cell::new(InputMode::Normal),
            ripple: RefCell::new(Ripple::default()),
            animating: Cell::new(false),
            last_frame: Cell::new(Instant::now()),
            pointer: Cell::new(None),
            press: Cell::new(None),
            press_origin: Cell::new((0, 0)),
            press_button: Cell::new(None),
            pos: Cell::new((0, 0)),
            size: Cell::new(side),
            percent: Cell::new(SCALE_DEFAULT),
            at_default: Cell::new(true),
            wheel: Cell::new(WheelSteps::default()),
        });
        ICON.with(|slot| *slot.borrow_mut() = Some(icon.clone()));

        let bubble_window = new_popup(BUBBLE_WIDTH, 40);
        let label = gtk::Label::new(None);
        label.set_line_wrap(true);
        label.set_line_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.set_margin_start(BUBBLE_PADDING);
        label.set_margin_end(BUBBLE_PADDING);
        label.set_margin_top(BUBBLE_PADDING);
        label.set_margin_bottom(BUBBLE_PADDING);
        label.set_size_request(BUBBLE_WIDTH - BUBBLE_PADDING * 2, -1);
        bubble_window.add(&label);
        let size = Rc::new(Cell::new((BUBBLE_WIDTH, 40)));
        {
            let size = size.clone();
            // The panel first; the label draws itself after (Proceed).
            bubble_window.connect_draw(move |w, cr| {
                let scale = w.scale_factor().max(1) as f64;
                let (bw, bh) = size.get();
                let panel = draw_rounded_panel(
                    (bw as f64 * scale).round() as u32,
                    (bh as f64 * scale).round() as u32,
                    (10.0 * scale) as f32,
                    (255, 255, 255, 250),
                );
                paint(cr, &panel, scale);
                glib::Propagation::Proceed
            });
        }
        let bubble = Bubble { window: bubble_window, label, size, shown: false };
        Overlay { icon, bubble, shown: false }
    }

    /// Shows the mic in `look`, at the saved position when it is still on a screen.
    pub fn show(&mut self, look: IconState, saved: Option<(i32, i32)>) {
        if !self.shown {
            let size = scaled_size(self.icon.percent.get(), 1.0);
            self.icon.resize(size);
            let (areas, primary) = work_areas();
            self.icon.pos.set(resolve_position(saved, size, &areas, primary));
            self.icon.at_default.set(saved.is_none_or(|p| !fits(p, size, &areas)));
            self.icon.place();
        }
        self.icon.look.set(look);
        self.icon.ripple.borrow_mut().set_recording(look == IconState::Recording);
        self.icon.window.queue_draw();
        if !self.shown {
            self.icon.window.show_all();
            self.shown = true;
        }
    }

    pub fn set_look(&mut self, look: IconState) {
        self.icon.look.set(look);
        self.icon.ripple.borrow_mut().set_recording(look == IconState::Recording);
        if self.shown {
            self.icon.window.queue_draw();
        }
    }

    pub fn look(&self) -> IconState {
        self.icon.look.get()
    }

    /// The mic's size in percent. On screen it changes around its centre (or stays in the
    /// corner); a moved mic reports where it went so the position is saved.
    pub fn set_scale(&mut self, percent: u16) {
        if self.icon.percent.replace(percent) == percent || !self.shown {
            return;
        }
        self.hide_bubble();
        let old = self.icon.size.get();
        let new = scaled_size(percent, 1.0);
        let (areas, primary) = work_areas();
        let at_default = self.icon.at_default.get();
        let pos = resized_position(self.icon.pos.get(), old, new, at_default, &areas, primary);
        self.icon.resize(new);
        self.icon.pos.set(pos);
        self.icon.place();
        self.icon.window.queue_draw();
        if !at_default {
            let _ = self.icon.events.send(PlatformEvent::IconMoved { x: pos.0, y: pos.1 });
        }
    }

    pub fn set_templates_menu(&mut self, menu: Menu) {
        *self.icon.templates_menu.borrow_mut() = Some(menu);
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        if self.icon.mode.replace(mode) != mode && self.shown {
            self.icon.window.queue_draw();
        }
    }

    /// The ripple follows what the recognizer heard (only while the mic is on screen).
    pub fn voice_cue(&mut self, cue: VoiceCue) {
        if !self.shown {
            return;
        }
        self.icon.ripple.borrow_mut().cue(cue);
        if self.icon.ripple.borrow().active() && !self.icon.animating.replace(true) {
            self.icon.last_frame.set(Instant::now());
            glib::timeout_add_local(FRAME, || {
                let mut going = false;
                with_icon(|s| going = s.frame());
                if going {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
    }

    pub fn hide(&mut self) {
        self.icon.window.hide();
        self.shown = false;
        self.hide_bubble();
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    fn text_height(&self, text: &str) -> i32 {
        self.bubble.label.set_markup(&markup(text));
        let (_, natural) = self.bubble.label.preferred_height_for_width(BUBBLE_WIDTH);
        natural - BUBBLE_PADDING * 2
    }

    /// At most `max_lines`; a longer text loses its start (live text: the latest words matter).
    pub fn show_bubble(&mut self, text: &str, max_lines: i32) {
        let max_height = self.text_height("Xg") * max_lines + 1;
        let mut shown = text.to_string();
        let mut limit = text.chars().count();
        let text_height = loop {
            let h = self.text_height(&shown);
            if h <= max_height || limit <= 4 {
                break h.min(max_height);
            }
            limit = (limit * 4 / 5).max(4);
            shown = tail(text, limit);
        };
        let height = text_height + BUBBLE_PADDING * 2;
        self.bubble.size.set((BUBBLE_WIDTH, height));
        self.bubble.window.resize(BUBBLE_WIDTH, height);

        // Above the mic, right edges lined up, kept on the mic's screen.
        let (ix, iy) = self.icon.pos.get();
        let (areas, primary) = work_areas();
        let area = areas
            .iter()
            .copied()
            .find(|a| ix >= a.x && ix < a.x + a.width && iy >= a.y && iy < a.y + a.height)
            .unwrap_or(primary);
        let x =
            (ix + self.icon.size.get() - BUBBLE_WIDTH).clamp(area.x, (area.x + area.width - BUBBLE_WIDTH).max(area.x));
        let y = (iy - height - 8).max(area.y);
        self.bubble.window.move_(x, y);
        self.bubble.window.queue_draw();
        if !self.bubble.shown {
            self.bubble.window.show_all();
            self.bubble.shown = true;
        }
    }

    pub fn hide_bubble(&mut self) {
        if self.bubble.shown {
            self.bubble.window.hide();
            self.bubble.shown = false;
        }
    }
}
