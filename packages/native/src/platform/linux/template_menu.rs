//! The floating mic's templates list on Linux (X11; `crate::menu::template_list`): a GTK menu whose
//! template rows hold the text and an edit and a delete button at the right end, shown while the
//! row is highlighted.
//!
//! A GTK menu does not pass clicks to widgets inside its items, so the item's own button release
//! tells the parts apart by where it landed. Deleting (and putting back) stops the release there,
//! which keeps the menu open, and changes the rows in place (`crate::menu::list_after`, then the
//! daemon's own list when it arrives). Everything else lets the item activate, which closes the
//! menu.
//!
//! Everything here runs on the GTK thread.

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::menu::{keeps_list_open, list_after, row_action, RowPart, TemplateList, TemplateRow, ROW_BUTTON};
use crate::platform::{MenuAction, PlatformEvent};

const GLYPH_EDIT: &str = "✎";
const GLYPH_DELETE: &str = "✕";
/// A button's look while the pointer is elsewhere on the row, and on it.
const DIM: f64 = 0.55;
const LIT: f64 = 1.0;

struct Session {
    menu: gtk::Menu,
    list: TemplateList,
    events: Sender<PlatformEvent>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

fn send(events: &Sender<PlatformEvent>, action: MenuAction) {
    let _ = events.send(PlatformEvent::Menu(action));
}

/// Opens `list` at the pointer, on the screen of `window` (the mic).
pub fn open(list: TemplateList, events: Sender<PlatformEvent>, window: &gtk::Window) {
    let menu = gtk::Menu::new();
    menu.connect_deactivate(|_| {
        // Closed for good (a deletion keeps it open, so this is the end of the list). GTK
        // deactivates the menu before it activates the chosen item, which sends on its own.
        SESSION.with(|s| s.borrow_mut().take());
    });
    SESSION.with(|s| *s.borrow_mut() = Some(Session { menu: menu.clone(), list: list.clone(), events }));
    show_rows(list);
    popup_at_pointer(&menu, window);
}

/// The daemon's list after a deletion: replaces the rows if the list is still open.
pub fn refresh(list: TemplateList) {
    let open = SESSION.with(|s| s.borrow().is_some());
    if open {
        show_rows(list);
    }
}

/// The way `muda` opens a context menu without a click event of its own: at the pointer, with a
/// made-up button press whose time is now (without it GTK closes the menu at once).
fn popup_at_pointer(menu: &gtk::Menu, widget: &gtk::Window) {
    let Some(root) = WidgetExt::screen(widget).and_then(|s| s.root_window()) else { return };
    let pointer = root.display().default_seat().and_then(|seat| seat.pointer());
    let (x, y) = pointer.as_ref().map(|p| p.position()).map(|(_, x, y)| (x, y)).unwrap_or_default();
    let mut event = gdk::Event::new(gdk::EventType::ButtonPress);
    event.set_device(pointer.as_ref());
    let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none().0;
    if !raw.is_null() {
        unsafe { (*raw).button.time = (glib::monotonic_time() / 1000) as u32 };
    }
    menu.popup_at_rect(
        &root,
        &gdk::Rectangle::new(x, y, 0, 0),
        gdk::Gravity::NorthWest,
        gdk::Gravity::NorthWest,
        Some(&event),
    );
}

/// Makes `list` the open menu's rows.
fn show_rows(list: TemplateList) {
    let Some((menu, events)) = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let session = s.as_mut()?;
        session.list = list.clone();
        Some((session.menu.clone(), session.events.clone()))
    }) else {
        return;
    };
    for child in menu.children() {
        menu.remove(&child);
    }
    for row in &list.rows {
        menu.append(&row_item(row, &list, &events));
    }
    if !list.rows.is_empty() {
        menu.append(&gtk::SeparatorMenuItem::new());
    }
    for (action, label) in &list.footer {
        let item = gtk::MenuItem::with_label(label);
        let (action, events) = (*action, events.clone());
        item.connect_activate(move |_| send(&events, action));
        menu.append(&item);
    }
    menu.show_all();
}

/// Deleting or putting back, from a click on a row: the rows change in place.
fn keep_open(events: &Sender<PlatformEvent>, action: MenuAction) {
    send(events, action);
    let next = SESSION.with(|s| s.borrow().as_ref().map(|s| list_after(&s.list, action)));
    if let Some(next) = next {
        // Not while GTK is still delivering the release to the old row.
        glib::idle_add_local_once(move || show_rows(next));
    }
}

fn button(glyph: &str, name: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(glyph));
    label.set_size_request(ROW_BUTTON as i32, -1);
    label.set_tooltip_text(Some(name));
    label.set_opacity(0.0);
    label
}

fn inside(widget: &impl IsA<gtk::Widget>, x: f64) -> bool {
    let a = widget.allocation();
    x >= f64::from(a.x()) && x < f64::from(a.x() + a.width())
}

fn row_item(row: &TemplateRow, list: &TemplateList, events: &Sender<PlatformEvent>) -> gtk::MenuItem {
    let item = gtk::MenuItem::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    item.add(&content);
    match row {
        TemplateRow::Template { label, .. } => {
            let text = gtk::Label::new(Some(label));
            text.set_xalign(0.0);
            text.set_hexpand(true);
            let edit = button(GLYPH_EDIT, &list.edit_name);
            let delete = button(GLYPH_DELETE, &list.delete_name);
            content.pack_start(&text, true, true, 0);
            content.pack_start(&edit, false, false, 0);
            content.pack_start(&delete, false, false, 0);
            // The buttons show on the highlighted row only; the one under the pointer is lit.
            let part_at = {
                let (edit, delete) = (edit.clone(), delete.clone());
                move |item: &gtk::MenuItem, x: f64| {
                    let x = f64::from(item.allocation().x()) + x;
                    if inside(&edit, x) {
                        RowPart::Edit
                    } else if inside(&delete, x) {
                        RowPart::Delete
                    } else {
                        RowPart::Text
                    }
                }
            };
            {
                let (edit, delete) = (edit.clone(), delete.clone());
                item.connect_select(move |_| {
                    edit.set_opacity(DIM);
                    delete.set_opacity(DIM);
                });
            }
            {
                let (edit, delete) = (edit.clone(), delete.clone());
                item.connect_deselect(move |_| {
                    edit.set_opacity(0.0);
                    delete.set_opacity(0.0);
                });
            }
            item.add_events(gdk::EventMask::POINTER_MOTION_MASK);
            {
                let part_at = part_at.clone();
                item.connect_motion_notify_event(move |item, event| {
                    let part = part_at(item, event.position().0);
                    edit.set_opacity(if part == RowPart::Edit { LIT } else { DIM });
                    delete.set_opacity(if part == RowPart::Delete { LIT } else { DIM });
                    glib::Propagation::Proceed
                });
            }
            let (row_release, events_release) = (row.clone(), events.clone());
            item.connect_button_release_event(move |item, event| {
                let action = row_action(&row_release, part_at(item, event.position().0));
                match action {
                    MenuAction::InsertTemplate(_) => glib::Propagation::Proceed,
                    action if keeps_list_open(action) => {
                        keep_open(&events_release, action);
                        glib::Propagation::Stop
                    }
                    action => {
                        send(&events_release, action);
                        if let Some(menu) = item.parent().and_then(|p| p.downcast::<gtk::Menu>().ok()) {
                            menu.popdown();
                        }
                        glib::Propagation::Stop
                    }
                }
            });
            // A click on the text (or Enter) activates the item: in goes the template.
            let (row_activate, events) = (row.clone(), events.clone());
            item.connect_activate(move |_| send(&events, row_action(&row_activate, RowPart::Text)));
        }
        TemplateRow::Deleted { message, undo, .. } => {
            let text = gtk::Label::new(Some(message));
            text.set_xalign(0.0);
            text.set_hexpand(true);
            text.set_opacity(0.7);
            let pill = gtk::Frame::new(None);
            let pill_label = gtk::Label::new(Some(undo));
            pill_label.set_margin_start(6);
            pill_label.set_margin_end(6);
            pill.add(&pill_label);
            content.pack_start(&text, true, true, 0);
            content.pack_start(&pill, false, false, 0);
            // The whole row puts it back; a click keeps the list open, Enter closes it.
            let events_release = events.clone();
            item.connect_button_release_event(move |_, _| {
                keep_open(&events_release, MenuAction::UndoDeleteTemplate);
                glib::Propagation::Stop
            });
            let events = events.clone();
            item.connect_activate(move |_| send(&events, MenuAction::UndoDeleteTemplate));
        }
    }
    item
}
