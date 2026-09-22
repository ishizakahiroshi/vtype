//! The floating mic and its speech bubble (child plan C6-C3): borderless, non-activating panels
//! that float over every Space and over full-screen apps, so the app being typed into keeps the
//! focus.
//!
//! Everything here runs on the main thread. The view's event methods only touch `IconShared`
//! (Cells), never the `Ui` borrowed by the job queue, the same split as on Windows.
//!
//! Coordinates: AppKit counts from the bottom-left of the main screen, y up. The rest of vtype
//! (the saved position, `overlay_logic`, Accessibility) counts from the top-left, y down, in
//! points. The conversion happens only in this file.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use objc2::rc::Retained;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSCursor, NSEvent, NSFloatingWindowLevel, NSFont, NSImage, NSPanel, NSScreen,
    NSTextField, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSData, NSPoint, NSRect, NSSize, NSString};
use tiny_skia::Pixmap;
use tray_icon::menu::{ContextMenu, Menu};

use crate::beside_field::{keep_on_screen, BESIDE_SIZE};
use crate::icon_draw::{draw_icon, draw_rounded_panel};
use crate::overlay_logic::{resolve_position, tail, Gesture, Press, ICON_SIZE};
use crate::platform::{IconState, PlatformEvent, Rect};

const BUBBLE_WIDTH: f64 = 320.0;
const BUBBLE_PADDING: f64 = 10.0;
const BUBBLE_FONT_SIZE: f64 = 13.0;

/// What a view is for: the pointer means something different on each.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The floating mic: click, drag, right-click menu, hover.
    FloatingMic,
    /// The mic beside a field: click only.
    BesideMic,
    /// The bubble's background: nothing.
    Picture,
}

pub struct ViewIvars {
    image: RefCell<Option<Retained<NSImage>>>,
    role: Role,
}

define_class!(
    /// A view that draws one image, and for the mics, reports what the pointer does.
    #[unsafe(super(NSView))]
    #[name = "VtypeImageView"]
    #[ivars = ViewIvars]
    pub struct ImageView;

    impl ImageView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if let Some(image) = self.ivars().image.borrow().as_ref() {
                image.drawInRect(self.bounds());
            }
        }

        // The first click on an inactive app's panel still counts as a click.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if self.ivars().role == Role::FloatingMic {
                with_icon(|s| s.press());
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, _event: &NSEvent) {
            if self.ivars().role == Role::FloatingMic {
                with_icon(|s| s.drag());
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            match self.ivars().role {
                Role::FloatingMic => with_icon(|s| s.release()),
                Role::BesideMic => BESIDE_EVENTS.with(|slot| {
                    if let Some(events) = slot.borrow().as_ref() {
                        let _ = events.send(PlatformEvent::ToggleRequested);
                    }
                }),
                Role::Picture => {}
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &NSEvent) {
            if self.ivars().role == Role::FloatingMic {
                // Clone the menu out first: it runs a modal loop.
                let menu = icon_shared().and_then(|s| s.menu.borrow().clone());
                if let Some(menu) = menu {
                    let view = self as *const ImageView as *const c_void;
                    unsafe { menu.show_context_menu_for_nsview(view, None) };
                }
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if self.ivars().role == Role::FloatingMic {
                with_icon(|s| s.set_hover(true));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if self.ivars().role == Role::FloatingMic {
                with_icon(|s| s.set_hover(false));
            }
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            if self.ivars().role != Role::Picture {
                self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
            }
        }
    }
);

impl ImageView {
    fn new(mtm: MainThreadMarker, size: NSSize, role: Role) -> Retained<ImageView> {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
        let this = mtm.alloc::<ImageView>().set_ivars(ViewIvars { image: RefCell::new(None), role });
        let view: Retained<ImageView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        if role == Role::FloatingMic {
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    frame,
                    options,
                    Some(&view),
                    None,
                )
            };
            view.addTrackingArea(&area);
        }
        view
    }
    fn set_image(&self, image: Option<Retained<NSImage>>) {
        *self.ivars().image.borrow_mut() = image;
        self.setNeedsDisplay(true);
    }
}

/// A PNG of the pixmap, shown at `size` points (the pixmap is drawn at the screen's scale).
fn image_from(pm: &Pixmap, size: NSSize) -> Option<Retained<NSImage>> {
    let png = pm.encode_png().ok()?;
    let data = NSData::with_bytes(&png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    image.setSize(size);
    Some(image)
}

fn new_panel(mtm: MainThreadMarker, size: NSSize, view: &NSView, ignores_mouse: bool) -> Retained<NSPanel> {
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc::<NSPanel>(),
        rect,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(false);
    panel.setIgnoresMouseEvents(ignores_mouse);
    panel.setContentView(Some(view));
    panel
}

fn main_screen_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).firstObject().map(|s| s.frame().size.height).unwrap_or(0.0)
}

fn to_top_left(r: NSRect, main_height: f64) -> Rect {
    Rect {
        x: r.origin.x.round() as i32,
        y: (main_height - (r.origin.y + r.size.height)).round() as i32,
        width: r.size.width.round() as i32,
        height: r.size.height.round() as i32,
    }
}

/// Every screen's visible frame (without the menu bar and the Dock), main screen first.
pub fn work_areas(mtm: MainThreadMarker) -> (Vec<Rect>, Rect) {
    let h = main_screen_height(mtm);
    let areas: Vec<Rect> = NSScreen::screens(mtm).iter().map(|s| to_top_left(s.visibleFrame(), h)).collect();
    let primary = areas.first().copied().unwrap_or_default();
    (areas, primary)
}

/// Every screen's whole frame.
pub fn screen_frames(mtm: MainThreadMarker) -> Vec<Rect> {
    let h = main_screen_height(mtm);
    NSScreen::screens(mtm).iter().map(|s| to_top_left(s.frame(), h)).collect()
}

fn cursor(mtm: MainThreadMarker) -> (i32, i32) {
    let p = NSEvent::mouseLocation();
    (p.x.round() as i32, (main_screen_height(mtm) - p.y).round() as i32)
}

/// Puts a panel's top-left corner at `pos` (top-left space).
fn place(panel: &NSPanel, mtm: MainThreadMarker, pos: (i32, i32), height: f64) {
    let origin = NSPoint::new(pos.0 as f64, main_screen_height(mtm) - pos.1 as f64 - height);
    panel.setFrameOrigin(origin);
}

struct IconShared {
    mtm: MainThreadMarker,
    events: Sender<PlatformEvent>,
    panel: Retained<NSPanel>,
    view: Retained<ImageView>,
    menu: RefCell<Option<Menu>>,
    look: Cell<IconState>,
    hover: Cell<bool>,
    press: Cell<Option<Press>>,
    press_origin: Cell<(i32, i32)>,
    pos: Cell<(i32, i32)>,
}

thread_local! {
    static ICON: RefCell<Option<Rc<IconShared>>> = const { RefCell::new(None) };
}

fn icon_shared() -> Option<Rc<IconShared>> {
    ICON.with(|slot| slot.borrow().clone())
}

fn with_icon(f: impl FnOnce(&IconShared)) {
    if let Some(s) = icon_shared() {
        f(&s);
    }
}

impl IconShared {
    fn render(&self) {
        let scale = self.panel.backingScaleFactor();
        let px = (ICON_SIZE as f64 * scale).round().max(1.0) as u32;
        let pm = draw_icon(px, self.look.get(), self.hover.get());
        let size = NSSize::new(ICON_SIZE as f64, ICON_SIZE as f64);
        self.view.set_image(image_from(&pm, size));
    }

    fn place(&self) {
        place(&self.panel, self.mtm, self.pos.get(), ICON_SIZE as f64);
    }

    fn press(&self) {
        self.press.set(Some(Press::new(cursor(self.mtm))));
        self.press_origin.set(self.pos.get());
    }

    fn drag(&self) {
        if let Some(mut press) = self.press.get() {
            let now = cursor(self.mtm);
            if press.moved(now) {
                let (dx, dy) = press.offset(now);
                let (ox, oy) = self.press_origin.get();
                self.pos.set((ox + dx, oy + dy));
                self.place();
            }
            self.press.set(Some(press));
        }
    }

    fn release(&self) {
        if let Some(press) = self.press.take() {
            let event = match press.release(cursor(self.mtm)) {
                Gesture::Click => PlatformEvent::ToggleRequested,
                Gesture::Drag => {
                    let (x, y) = self.pos.get();
                    PlatformEvent::IconMoved { x, y }
                }
            };
            let _ = self.events.send(event);
        }
    }

    fn set_hover(&self, hover: bool) {
        if self.hover.get() != hover {
            self.hover.set(hover);
            self.render();
        }
    }
}

struct Bubble {
    panel: Retained<NSPanel>,
    view: Retained<ImageView>,
    label: Retained<NSTextField>,
    shown: bool,
}

pub struct Overlay {
    icon: Rc<IconShared>,
    bubble: Bubble,
    shown: bool,
}

impl Overlay {
    pub fn new(events: Sender<PlatformEvent>, mtm: MainThreadMarker, menu: Menu) -> Overlay {
        let icon_size = NSSize::new(ICON_SIZE as f64, ICON_SIZE as f64);
        let view = ImageView::new(mtm, icon_size, Role::FloatingMic);
        let panel = new_panel(mtm, icon_size, &view, false);
        let icon = Rc::new(IconShared {
            mtm,
            events,
            panel,
            view,
            menu: RefCell::new(Some(menu)),
            look: Cell::new(IconState::Idle),
            hover: Cell::new(false),
            press: Cell::new(None),
            press_origin: Cell::new((0, 0)),
            pos: Cell::new((0, 0)),
        });
        ICON.with(|slot| *slot.borrow_mut() = Some(icon.clone()));

        let bubble_size = NSSize::new(BUBBLE_WIDTH, 40.0);
        let bubble_view = ImageView::new(mtm, bubble_size, Role::Picture);
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(BUBBLE_FONT_SIZE)));
        label.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0x1f as f64 / 255.0,
            0x29 as f64 / 255.0,
            0x37 as f64 / 255.0,
            1.0,
        )));
        label.setMaximumNumberOfLines(2);
        label.setPreferredMaxLayoutWidth(BUBBLE_WIDTH - BUBBLE_PADDING * 2.0);
        bubble_view.addSubview(&label);
        let bubble_panel = new_panel(mtm, bubble_size, &bubble_view, true);
        let bubble = Bubble { panel: bubble_panel, view: bubble_view, label, shown: false };
        Overlay { icon, bubble, shown: false }
    }

    /// Shows the mic in `look`, at the saved position when it is still on a screen.
    pub fn show(&mut self, look: IconState, saved: Option<(i32, i32)>) {
        if !self.shown {
            let (areas, primary) = work_areas(self.icon.mtm);
            self.icon.pos.set(resolve_position(saved, ICON_SIZE, &areas, primary));
            self.icon.place();
        }
        self.icon.look.set(look);
        self.icon.render();
        if !self.shown {
            self.icon.panel.orderFrontRegardless();
            self.shown = true;
        }
    }

    pub fn set_look(&mut self, look: IconState) {
        self.icon.look.set(look);
        if self.shown {
            self.icon.render();
        }
    }

    pub fn look(&self) -> IconState {
        self.icon.look.get()
    }

    pub fn hide(&mut self) {
        self.icon.panel.orderOut(None);
        self.shown = false;
        self.hide_bubble();
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    fn text_height(&self, text: &str, width: f64) -> f64 {
        self.bubble.label.setStringValue(&NSString::from_str(text));
        self.bubble.label.sizeThatFits(NSSize::new(width, 10_000.0)).height.ceil()
    }

    /// At most `max_lines`; a longer text loses its start (live text: the latest words matter).
    pub fn show_bubble(&mut self, text: &str, max_lines: i32) {
        let mtm = self.icon.mtm;
        let inner = BUBBLE_WIDTH - BUBBLE_PADDING * 2.0;
        let max_height = self.text_height("Xg", inner) * f64::from(max_lines) + 1.0;
        let mut shown = text.to_string();
        let mut limit = text.chars().count();
        let text_height = loop {
            let h = self.text_height(&shown, inner);
            if h <= max_height || limit <= 4 {
                break h.min(max_height);
            }
            limit = (limit * 4 / 5).max(4);
            shown = tail(text, limit);
        };
        let height = text_height + BUBBLE_PADDING * 2.0;
        let size = NSSize::new(BUBBLE_WIDTH, height);

        let scale = self.icon.panel.backingScaleFactor();
        let panel_px = draw_rounded_panel(
            (BUBBLE_WIDTH * scale).round() as u32,
            (height * scale).round() as u32,
            (10.0 * scale) as f32,
            (255, 255, 255, 250),
        );
        self.bubble.panel.setContentSize(size);
        self.bubble.view.setFrameSize(size);
        self.bubble.view.set_image(image_from(&panel_px, size));
        self.bubble
            .label
            .setFrame(NSRect::new(NSPoint::new(BUBBLE_PADDING, BUBBLE_PADDING), NSSize::new(inner, text_height)));

        // Above the mic, right edges lined up, kept on the mic's screen.
        let (ix, iy) = self.icon.pos.get();
        let (areas, primary) = work_areas(mtm);
        let area = areas
            .iter()
            .copied()
            .find(|a| ix >= a.x && ix < a.x + a.width && iy >= a.y && iy < a.y + a.height)
            .unwrap_or(primary);
        let (w, h) = (BUBBLE_WIDTH.round() as i32, height.round() as i32);
        let x = (ix + ICON_SIZE - w).clamp(area.x, (area.x + area.width - w).max(area.x));
        let y = (iy - h - 8).max(area.y);
        place(&self.bubble.panel, mtm, (x, y), height);
        if !self.bubble.shown {
            self.bubble.panel.orderFrontRegardless();
            self.bubble.shown = true;
        }
    }

    pub fn hide_bubble(&mut self) {
        if self.bubble.shown {
            self.bubble.panel.orderOut(None);
            self.bubble.shown = false;
        }
    }
}

// --- the mic beside the text field (child plan C8) ---------------------------------------------

thread_local! {
    static BESIDE_EVENTS: RefCell<Option<Sender<PlatformEvent>>> = const { RefCell::new(None) };
}

/// The small mic next to the focused field: a panel like the floating mic's, 26 pt, always opaque
/// (it only shows while a field has the focus), click only.
pub struct BesideMic {
    mtm: MainThreadMarker,
    panel: Retained<NSPanel>,
    view: Retained<ImageView>,
    shown: bool,
    look: IconState,
}

impl BesideMic {
    pub fn new(events: Sender<PlatformEvent>, mtm: MainThreadMarker) -> BesideMic {
        BESIDE_EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
        let size = NSSize::new(BESIDE_SIZE as f64, BESIDE_SIZE as f64);
        let view = ImageView::new(mtm, size, Role::BesideMic);
        let panel = new_panel(mtm, size, &view, false);
        BesideMic { mtm, panel, view, shown: false, look: IconState::Idle }
    }

    fn render(&self) {
        let scale = self.panel.backingScaleFactor();
        let px = (BESIDE_SIZE as f64 * scale).round().max(1.0) as u32;
        let size = NSSize::new(BESIDE_SIZE as f64, BESIDE_SIZE as f64);
        self.view.set_image(image_from(&draw_icon(px, self.look, true), size));
    }

    /// Puts the mic at `pos` (points from the top-left of the main screen), kept on that screen.
    pub fn show(&mut self, pos: (i32, i32), look: IconState) {
        let (areas, primary) = work_areas(self.mtm);
        let pos = keep_on_screen(pos, BESIDE_SIZE, &areas, primary);
        place(&self.panel, self.mtm, pos, BESIDE_SIZE as f64);
        self.look = look;
        self.render();
        if !self.shown {
            self.panel.orderFrontRegardless();
            self.shown = true;
        }
    }

    pub fn set_look(&mut self, look: IconState) {
        self.look = look;
        if self.shown {
            self.render();
        }
    }

    pub fn hide(&mut self) {
        if self.shown {
            self.panel.orderOut(None);
            self.shown = false;
        }
    }
}
