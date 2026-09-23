//! The floating mic's templates list on macOS (`crate::menu::template_list`): an NSMenu whose rows
//! are views drawn here, so that a template's row can carry an edit and a delete button at its
//! right end, shown while the row is highlighted.
//!
//! A view in a menu gets the clicks itself and the menu stays open, so deleting (and putting back)
//! changes the rows in place; everything else ends the menu's tracking. While the menu tracks, the
//! main queue runs nothing else (the menu was opened from it), so the new rows come from
//! `crate::menu::list_after` rather than from the daemon, whose list replaces them only if it
//! arrives while the menu is still open.
//!
//! Everything here runs on the main thread.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::Sender;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSMenu, NSMenuItem,
    NSStringDrawing, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{MainThreadMarker, NSAttributedStringKey, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::menu::{
    keeps_list_open, list_after, row_action, row_buttons, row_part, RowPart, TemplateList, TemplateRow, ROW_BUTTON,
    ROW_BUTTONS_WIDTH, ROW_PAD_RIGHT,
};
use crate::platform::{MenuAction, PlatformEvent};

/// Row layout in points.
const ROW_HEIGHT: f64 = 24.0;
const PAD_LEFT: f64 = 14.0;
const MIN_WIDTH: f64 = 200.0;
/// The highlight's inset from the menu's sides, and its corners.
const HOT_INSET: f64 = 5.0;
const HOT_RADIUS: f64 = 4.0;
/// The "Undo" pill: padding around its text, and how much shorter than the row it is.
const PILL_PAD: f64 = 8.0;
const PILL_INSET: f64 = 3.0;
const GLYPH_EDIT: &str = "✎";
const GLYPH_DELETE: &str = "✕";

/// What a row view shows.
#[derive(Clone, Debug)]
enum Kind {
    Row(TemplateRow),
    Footer(MenuAction, String),
}

pub struct RowIvars {
    kind: Kind,
    /// The button the pointer is on (template rows only).
    hover: Cell<Option<RowPart>>,
}

define_class!(
    /// One row of the templates list.
    #[unsafe(super(NSView))]
    #[name = "VtypeTemplateRow"]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        // Top-left origin, like the rest of the layout.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.clicked(self.part_at(event));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.set_hover(Some(self.part_at(event)));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_hover(None);
        }
    }
);

impl RowView {
    fn new(mtm: MainThreadMarker, kind: Kind, width: f64) -> Retained<RowView> {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, ROW_HEIGHT));
        let this = mtm.alloc::<RowView>().set_ivars(RowIvars { kind, hover: Cell::new(None) });
        let view: Retained<RowView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
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
        view
    }

    fn highlighted(&self) -> bool {
        self.enclosingMenuItem().is_some_and(|item| item.isHighlighted())
    }

    /// The part of a template row under the pointer (`Text` on any other row).
    fn part_at(&self, event: &NSEvent) -> RowPart {
        match &self.ivars().kind {
            Kind::Row(TemplateRow::Template { .. }) => {
                let at = self.convertPoint_fromView(event.locationInWindow(), None);
                row_part(at.x, self.bounds().size.width, 1.0)
            }
            _ => RowPart::Text,
        }
    }

    fn set_hover(&self, part: Option<RowPart>) {
        let part = part.filter(|_| matches!(self.ivars().kind, Kind::Row(TemplateRow::Template { .. })));
        if self.ivars().hover.replace(part) != part {
            self.setNeedsDisplay(true);
        }
    }

    fn clicked(&self, part: RowPart) {
        // Rebuilding the rows below lets go of this view; keep it until this method is done.
        let _keep = self.retain();
        let action = match &self.ivars().kind {
            Kind::Row(row) => row_action(row, part),
            Kind::Footer(action, _) => *action,
        };
        let (events, menu) = SESSION.with(|s| {
            let s = s.borrow();
            s.as_ref().map(|s| (s.events.clone(), s.menu.clone())).unzip()
        });
        if let Some(events) = events {
            let _ = events.send(PlatformEvent::Menu(action));
        }
        if keeps_list_open(action) {
            let next = SESSION.with(|s| s.borrow().as_ref().map(|s| list_after(&s.list, action)));
            if let Some(next) = next {
                show_rows(next);
            }
        } else if let Some(menu) = menu {
            menu.cancelTracking();
        }
    }

    fn draw(&self) {
        let bounds = self.bounds();
        let width = bounds.size.width;
        let lit = self.highlighted();
        if lit {
            let hot = NSRect::new(
                NSPoint::new(HOT_INSET, 0.0),
                NSSize::new((width - 2.0 * HOT_INSET).max(0.0), bounds.size.height),
            );
            NSColor::selectedContentBackgroundColor().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(hot, HOT_RADIUS, HOT_RADIUS).fill();
        }
        let font = NSFont::menuFontOfSize(0.0);
        let text = if lit { NSColor::selectedMenuItemTextColor() } else { NSColor::labelColor() };
        let gray = if lit { text.colorWithAlphaComponent(0.7) } else { NSColor::secondaryLabelColor() };
        match &self.ivars().kind {
            Kind::Row(TemplateRow::Template { label, .. }) => {
                draw_text(label, PAD_LEFT, &font, &text, bounds);
                if lit {
                    for (part, left, right) in row_buttons(width, 1.0) {
                        let button = NSRect::new(
                            NSPoint::new(left, (bounds.size.height - ROW_BUTTON) / 2.0),
                            NSSize::new(right - left, ROW_BUTTON),
                        );
                        if self.ivars().hover.get() == Some(part) {
                            NSColor::colorWithWhite_alpha(1.0, 0.25).setFill();
                            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(button, 4.0, 4.0).fill();
                        }
                        let glyph = if part == RowPart::Edit { GLYPH_EDIT } else { GLYPH_DELETE };
                        let size = text_size(glyph, &font);
                        draw_text(glyph, left + (right - left - size.width) / 2.0, &font, &text, bounds);
                    }
                }
            }
            Kind::Row(TemplateRow::Deleted { message, undo, .. }) => {
                draw_text(message, PAD_LEFT, &font, &gray, bounds);
                let pill_width = text_size(undo, &font).width + 2.0 * PILL_PAD;
                let pill = NSRect::new(
                    NSPoint::new(width - ROW_PAD_RIGHT - pill_width, PILL_INSET),
                    NSSize::new(pill_width, bounds.size.height - 2.0 * PILL_INSET),
                );
                let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill, 4.0, 4.0);
                if lit {
                    NSColor::colorWithWhite_alpha(1.0, 0.25).setFill();
                    path.fill();
                } else {
                    gray.setStroke();
                    path.stroke();
                }
                draw_text(undo, pill.origin.x + PILL_PAD, &font, &text, bounds);
            }
            Kind::Footer(_, label) => draw_text(label, PAD_LEFT, &font, &text, bounds),
        }
    }
}

fn attributes(font: &NSFont, color: &NSColor) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    let keys: [&NSAttributedStringKey; 2] = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font.as_ref(), color.as_ref()];
    NSDictionary::from_slices(&keys, &values)
}

fn text_size(text: &str, font: &NSFont) -> NSSize {
    let attrs = attributes(font, &NSColor::labelColor());
    unsafe { NSString::from_str(text).sizeWithAttributes(Some(&attrs)) }
}

/// Draws `text` at `x`, centred in the row's height.
fn draw_text(text: &str, x: f64, font: &NSFont, color: &NSColor, bounds: NSRect) {
    let attrs = attributes(font, color);
    let s = NSString::from_str(text);
    let height = unsafe { s.sizeWithAttributes(Some(&attrs)) }.height;
    let at = NSPoint::new(x, (bounds.size.height - height) / 2.0);
    unsafe { s.drawAtPoint_withAttributes(at, Some(&attrs)) };
}

/// How wide a row must be to show `kind` whole.
fn natural_width(kind: &Kind, font: &NSFont) -> f64 {
    match kind {
        Kind::Row(TemplateRow::Template { label, .. }) => PAD_LEFT + text_size(label, font).width + ROW_BUTTONS_WIDTH,
        Kind::Row(TemplateRow::Deleted { message, undo, .. }) => {
            PAD_LEFT
                + text_size(message, font).width
                + PAD_LEFT
                + text_size(undo, font).width
                + 2.0 * PILL_PAD
                + ROW_PAD_RIGHT
        }
        Kind::Footer(_, label) => 2.0 * PAD_LEFT + text_size(label, font).width,
    }
}

struct Session {
    list: TemplateList,
    menu: Retained<NSMenu>,
    events: Sender<PlatformEvent>,
    mtm: MainThreadMarker,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Opens `list` at the pointer and returns when it closes (the menu runs a modal loop, so call
/// this outside any borrow of the UI).
pub fn open(list: TemplateList, events: Sender<PlatformEvent>, mtm: MainThreadMarker) {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    SESSION.with(|s| *s.borrow_mut() = Some(Session { list: list.clone(), menu: menu.clone(), events, mtm }));
    show_rows(list);
    menu.popUpMenuPositioningItem_atLocation_inView(None, NSEvent::mouseLocation(), None);
    SESSION.with(|s| s.borrow_mut().take());
}

/// The daemon's list after a deletion: replaces the rows if the list is still open.
pub fn refresh(list: TemplateList) {
    let open = SESSION.with(|s| s.borrow().is_some());
    if open {
        show_rows(list);
    }
}

/// Makes `list` the open menu's rows.
fn show_rows(list: TemplateList) {
    let Some((menu, mtm)) = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let session = s.as_mut()?;
        session.list = list.clone();
        Some((session.menu.clone(), session.mtm))
    }) else {
        return;
    };
    let mut kinds: Vec<Option<Kind>> = list.rows.iter().cloned().map(|row| Some(Kind::Row(row))).collect();
    if !kinds.is_empty() {
        kinds.push(None);
    }
    kinds.extend(list.footer.iter().map(|(action, label)| Some(Kind::Footer(*action, label.clone()))));
    let font = NSFont::menuFontOfSize(0.0);
    let width = kinds.iter().flatten().map(|kind| natural_width(kind, &font)).fold(MIN_WIDTH, f64::max).ceil();
    menu.removeAllItems();
    for kind in kinds {
        let item = match kind {
            None => NSMenuItem::separatorItem(mtm),
            Some(kind) => {
                let item = NSMenuItem::new(mtm);
                item.setView(Some(&RowView::new(mtm, kind, width)));
                item
            }
        };
        menu.addItem(&item);
    }
}
