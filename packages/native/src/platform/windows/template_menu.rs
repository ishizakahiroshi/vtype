//! The floating mic's templates list on Windows (`crate::menu::template_list`): a popup menu whose
//! rows are drawn here (owner-draw), so that a template's row can carry an edit and a delete
//! button at its right end, shown while the row is highlighted.
//!
//! A menu only tells which row was chosen, not where on it. A message hook (`WH_MSGFILTER`) watches
//! the pointer inside the menu's loop: it lights the button under the pointer and notes on which
//! part of the row the click landed. A menu also closes on every click, so deleting (and putting
//! back) opens it again at the same place once the daemon has the new list, which reads as the
//! list staying open.
//!
//! Everything here runs on the UI thread. The menu's modal loop calls back into this module
//! (measuring and drawing rows, the hook), so `STATE` is never borrowed across that loop.

use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::mpsc::Sender;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, GetDC, GetPixel,
    GetStockObject, GetSysColor, GetSysColorBrush, GetTextExtentPoint32W, InvalidateRect, MonitorFromPoint, ReleaseDC,
    RoundRect, SelectObject, SetBkMode, SetTextColor, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_MENU,
    COLOR_MENUTEXT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC, HFONT, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, NULL_BRUSH, PS_SOLID, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Controls::{
    CloseThemeData, DrawThemeBackground, OpenThemeData, DRAWITEMSTRUCT, HTHEME, MEASUREITEMSTRUCT,
    MENU_POPUPBACKGROUND, ODS_SELECTED, ODT_MENU,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, SystemParametersInfoForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, GetCursorPos,
    GetForegroundWindow, GetMenuItemRect, MenuItemFromPoint, PostMessageW, RegisterClassExW, SetForegroundWindow,
    SetWindowsHookExW, TrackPopupMenuEx, UnhookWindowsHookEx, WindowFromPoint, HHOOK, HMENU, MF_OWNERDRAW,
    MF_SEPARATOR, MSG, MSGF_MENU, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_TOPALIGN, WH_MSGFILTER, WM_APP, WM_DRAWITEM, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MEASUREITEM, WM_MOUSEMOVE,
    WM_NULL, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::overlay::wide;
use crate::menu::{keeps_list_open, row_action, row_buttons, row_part, RowPart, TemplateList, TemplateRow};
use crate::menu::{ROW_BUTTON, ROW_BUTTONS_WIDTH, ROW_PAD_RIGHT};
use crate::platform::{MenuAction, PlatformEvent};

/// Posted to the owner window: open the menu (outside the UI job that asked for it).
const WM_APP_TRACK: u32 = WM_APP + 10;

/// Row layout in points (96 dpi pixels).
const ROW_HEIGHT: f64 = 30.0;
const PAD_LEFT: f64 = 12.0;
const MIN_WIDTH: f64 = 200.0;
/// The "Undo" pill: padding around its text, and how much shorter than the row it is.
const PILL_PAD: f64 = 8.0;
const PILL_INSET: f64 = 5.0;

/// Segoe MDL2 Assets (Windows 10 and later): Edit and Cancel.
const GLYPH_EDIT: &str = "\u{E70F}";
const GLYPH_DELETE: &str = "\u{E711}";
const ICON_FACE: &str = "Segoe MDL2 Assets";

/// What a menu position holds.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Row(TemplateRow),
    Footer(MenuAction, String),
    Separator,
}

/// One open list: from the click on the mic until it closes for good (a deletion reopens it).
struct Session {
    list: TemplateList,
    /// Where the menu opens (its top-left), kept so that reopening puts it at the same place.
    anchor: POINT,
    /// The app in front before the list, given the focus back when the list closes.
    return_to: HWND,
    /// A deletion (or putting back) closed the menu; the daemon's refresh reopens it.
    awaiting_refresh: bool,
}

/// The menu on screen, for the owner window's measuring and drawing and for the hook.
struct Tracking {
    hmenu: HMENU,
    entries: Vec<Entry>,
    scale: f64,
    fonts: Fonts,
    theme: HTHEME,
    /// The menu's own window, found under the pointer, to redraw when the lit button changes.
    menu_window: HWND,
    /// The row (menu position) and the part of it the pointer is on.
    hover: Option<(usize, RowPart)>,
    /// Where the last click landed.
    clicked: Option<(usize, RowPart)>,
}

struct State {
    owner: HWND,
    events: Option<Sender<PlatformEvent>>,
    session: Option<Session>,
    tracking: Option<Tracking>,
}

thread_local! {
    static STATE: RefCell<State> =
        const { RefCell::new(State { owner: null_mut(), events: None, session: None, tracking: None }) };
}

/// Creates the (never shown) window that owns the menu. Call once on the UI thread.
pub fn init(events: Sender<PlatformEvent>) {
    let class = wide("vtypeTemplates");
    let owner = unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(owner_proc),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    if owner.is_null() {
        tracing::warn!("could not create the templates menu's window");
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.owner = owner;
        s.events = Some(events);
    });
}

/// Opens `list` at the pointer (the mic's templates button was just clicked).
pub fn show(list: TemplateList) {
    let mut anchor = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut anchor) };
    // The mic never takes the focus, so the app the user was in is still in front.
    let return_to = unsafe { GetForegroundWindow() };
    let owner = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.session = Some(Session { list, anchor, return_to, awaiting_refresh: false });
        s.owner
    });
    unsafe { PostMessageW(owner, WM_APP_TRACK, 0, 0) };
}

/// The daemon's list after a deletion (or putting back): reopens the menu the click closed.
pub fn refresh(list: TemplateList) {
    let owner = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let owner = s.owner;
        match s.session.as_mut() {
            Some(session) if session.awaiting_refresh => {
                session.list = list;
                session.awaiting_refresh = false;
                Some(owner)
            }
            _ => None,
        }
    });
    if let Some(owner) = owner {
        unsafe { PostMessageW(owner, WM_APP_TRACK, 0, 0) };
    }
}

/// The menu positions for `list`: its rows, a separator, the footer.
fn entries(list: &TemplateList) -> Vec<Entry> {
    let mut entries: Vec<Entry> = list.rows.iter().cloned().map(Entry::Row).collect();
    if !entries.is_empty() {
        entries.push(Entry::Separator);
    }
    entries.extend(list.footer.iter().map(|(action, label)| Entry::Footer(*action, label.clone())));
    entries
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRACK => {
            track();
            0
        }
        WM_MEASUREITEM => {
            let mis = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            if mis.CtlType != ODT_MENU {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            STATE.with(|s| {
                if let Some(t) = s.borrow().tracking.as_ref() {
                    if let Some(entry) = t.entries.get(mis.itemID.wrapping_sub(1) as usize) {
                        let (w, h) = measure(entry, &t.fonts, t.scale);
                        mis.itemWidth = w;
                        mis.itemHeight = h;
                    }
                }
            });
            1
        }
        WM_DRAWITEM => {
            let dis = &*(lparam as *const DRAWITEMSTRUCT);
            if dis.CtlType != ODT_MENU {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            STATE.with(|s| {
                if let Some(t) = s.borrow().tracking.as_ref() {
                    let pos = dis.itemID.wrapping_sub(1) as usize;
                    if let Some(entry) = t.entries.get(pos) {
                        let hover = t.hover.filter(|(at, _)| *at == pos).map(|(_, part)| part);
                        let selected = dis.itemState & ODS_SELECTED != 0;
                        paint(dis.hDC, dis.rcItem, entry, selected, hover, &t.fonts, t.theme, t.scale);
                    }
                }
            });
            1
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Opens the menu for the current session, waits for it, and acts on the choice.
fn track() {
    let Some((owner, list, anchor)) = STATE.with(|s| {
        let s = s.borrow();
        s.session.as_ref().map(|session| (s.owner, session.list.clone(), session.anchor))
    }) else {
        return;
    };
    let entries = entries(&list);
    let scale = f64::from(dpi_at(anchor)) / 96.0;
    let fonts = Fonts::new(scale);
    let theme_class = wide("MENU");
    unsafe {
        let hmenu = CreatePopupMenu();
        for (pos, entry) in entries.iter().enumerate() {
            if *entry == Entry::Separator {
                AppendMenuW(hmenu, MF_SEPARATOR, 0, null());
            } else {
                // Owner-drawn: the last argument is only data, the id is the position + 1.
                AppendMenuW(hmenu, MF_OWNERDRAW, pos + 1, null());
            }
        }
        let theme = OpenThemeData(owner, theme_class.as_ptr());
        STATE.with(|s| {
            s.borrow_mut().tracking = Some(Tracking {
                hmenu,
                entries: entries.clone(),
                scale,
                fonts,
                theme,
                menu_window: null_mut(),
                hover: None,
                clicked: None,
            });
        });
        // The owner must be in front, or a click elsewhere would not close the menu.
        SetForegroundWindow(owner);
        let hook: HHOOK = SetWindowsHookExW(WH_MSGFILTER, Some(hook_proc), null_mut(), GetCurrentThreadId());
        let chosen =
            TrackPopupMenuEx(hmenu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, anchor.x, anchor.y, owner, null());
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        // So that the next menu opens properly (the documented companion to SetForegroundWindow).
        PostMessageW(owner, WM_NULL, 0, 0);
        let tracking = STATE.with(|s| s.borrow_mut().tracking.take());
        DestroyMenu(hmenu);
        if !theme == 0 {
            CloseThemeData(theme);
        }
        let clicked = tracking.as_ref().and_then(|t| t.clicked);
        drop(tracking);
        finish(chosen, clicked, &entries);
    }
}

/// What the choice `chosen` (a menu id, 0 for none) does, given where the click landed.
fn choice(chosen: i32, clicked: Option<(usize, RowPart)>, entries: &[Entry]) -> Option<MenuAction> {
    let pos = usize::try_from(chosen).ok()?.checked_sub(1)?;
    // A click decides the part; Enter on a highlighted row means its text.
    let part = clicked.filter(|(at, _)| *at == pos).map_or(RowPart::Text, |(_, part)| part);
    match entries.get(pos)? {
        Entry::Row(row) => Some(row_action(row, part)),
        Entry::Footer(action, _) => Some(*action),
        Entry::Separator => None,
    }
}

fn finish(chosen: i32, clicked: Option<(usize, RowPart)>, entries: &[Entry]) {
    let action = choice(chosen, clicked, entries);
    let keep_open = action.is_some_and(keeps_list_open);
    let (events, return_to) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let return_to = if keep_open {
            if let Some(session) = s.session.as_mut() {
                session.awaiting_refresh = true;
            }
            None
        } else {
            s.session.take().map(|session| session.return_to)
        };
        (s.events.clone(), return_to)
    });
    // The template (or the copy of the selection) goes to the app the user was in.
    if let Some(app) = return_to.filter(|h| !h.is_null()) {
        unsafe { SetForegroundWindow(app) };
    }
    if let (Some(action), Some(events)) = (action, events) {
        let _ = events.send(PlatformEvent::Menu(action));
    }
}

/// Sees the pointer inside the menu's loop: lights the button under it, notes where clicks land.
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == MSGF_MENU as i32 && lparam != 0 {
        let msg = &*(lparam as *const MSG);
        match msg.message {
            WM_MOUSEMOVE => pointer_at(msg.pt, false),
            WM_LBUTTONDOWN | WM_LBUTTONUP => pointer_at(msg.pt, true),
            _ => {}
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn pointer_at(pt: POINT, click: bool) {
    let redraw = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let owner = s.owner;
        let t = s.tracking.as_mut()?;
        let pos = unsafe { MenuItemFromPoint(owner, t.hmenu, pt) };
        let part = usize::try_from(pos).ok().and_then(|pos| {
            let row = matches!(t.entries.get(pos), Some(Entry::Row(TemplateRow::Template { .. })));
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let found = row && unsafe { GetMenuItemRect(owner, t.hmenu, pos as u32, &mut r) } != 0;
            found.then(|| (pos, row_part(f64::from(pt.x - r.left), f64::from(r.right - r.left), t.scale)))
        });
        if click {
            t.clicked = part.or_else(|| usize::try_from(pos).ok().map(|pos| (pos, RowPart::Text)));
        }
        if part == t.hover {
            return None;
        }
        t.hover = part;
        if part.is_some() {
            t.menu_window = unsafe { WindowFromPoint(pt) };
        }
        Some(t.menu_window)
    });
    if let Some(window) = redraw.filter(|w| !w.is_null()) {
        unsafe { InvalidateRect(window, null(), 1) };
    }
}

/// The DPI of the monitor at `pt`, 96 when it cannot be told.
fn dpi_at(pt: POINT) -> u32 {
    let (mut x, mut y) = (96u32, 96u32);
    let monitor = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } < 0 {
        return 96;
    }
    x
}

/// The menu font at the menu's DPI, and the icon font at the same size.
struct Fonts {
    text: HFONT,
    icons: HFONT,
}

impl Fonts {
    fn new(scale: f64) -> Fonts {
        let dpi = (scale * 96.0).round() as u32;
        unsafe {
            let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
            metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                &mut metrics as *mut NONCLIENTMETRICSW as *mut _,
                0,
                dpi,
            );
            let text = CreateFontIndirectW(&metrics.lfMenuFont);
            let mut icon = metrics.lfMenuFont;
            icon.lfFaceName = [0; 32];
            for (slot, unit) in icon.lfFaceName.iter_mut().zip(ICON_FACE.encode_utf16()) {
                *slot = unit;
            }
            Fonts { text, icons: CreateFontIndirectW(&icon) }
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.text as HGDIOBJ);
            DeleteObject(self.icons as HGDIOBJ);
        }
    }
}

fn px(points: f64, scale: f64) -> i32 {
    (points * scale).round() as i32
}

/// The width of `text` in `font`.
fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE { cx: 0, cy: 0 };
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        GetTextExtentPoint32W(hdc, units.as_ptr(), units.len() as i32, &mut size);
        SelectObject(hdc, old);
    }
    size.cx
}

/// How big a menu position is, in pixels.
fn measure(entry: &Entry, fonts: &Fonts, scale: f64) -> (u32, u32) {
    let hdc = unsafe { GetDC(null_mut()) };
    let width = match entry {
        Entry::Row(TemplateRow::Template { label, .. }) => {
            px(PAD_LEFT, scale) + text_width(hdc, fonts.text, label) + px(ROW_BUTTONS_WIDTH, scale)
        }
        Entry::Row(TemplateRow::Deleted { message, undo, .. }) => {
            px(PAD_LEFT, scale)
                + text_width(hdc, fonts.text, message)
                + px(PAD_LEFT, scale)
                + pill_width(hdc, fonts, undo, scale)
                + px(ROW_PAD_RIGHT, scale)
        }
        Entry::Footer(_, label) => px(PAD_LEFT * 2.0, scale) + text_width(hdc, fonts.text, label),
        Entry::Separator => 0,
    };
    unsafe { ReleaseDC(null_mut(), hdc) };
    (width.max(px(MIN_WIDTH, scale)) as u32, px(ROW_HEIGHT, scale) as u32)
}

fn pill_width(hdc: HDC, fonts: &Fonts, undo: &str, scale: f64) -> i32 {
    text_width(hdc, fonts.text, undo) + px(PILL_PAD * 2.0, scale)
}

/// `color` darker by `percent` (of 100): the highlighted row on the menu's background, and the
/// lit button on the highlighted row, whatever the theme's colours.
fn darker(color: COLORREF, percent: u32) -> COLORREF {
    let part = |shift: u32| (((color >> shift) & 0xFF) * (100 - percent) / 100) << shift;
    part(0) | part(8) | part(16)
}

/// How much darker the highlighted row is than the menu, and a lit button than the row.
const ROW_SHADE: u32 = 7;
const BUTTON_SHADE: u32 = 10;
/// The highlighted row's inset from the menu's sides and its corners, in points.
const HOT_INSET: f64 = 4.0;
const HOT_RADIUS: f64 = 8.0;

fn draw_text(hdc: HDC, font: HFONT, color: COLORREF, text: &str, mut rect: RECT, format: u32) {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            units.as_mut_ptr(),
            units.len() as i32,
            &mut rect,
            format | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
    }
}

/// A rounded box: filled with `fill` (none for `None`), outlined with `line` (none for `None`).
fn rounded(hdc: HDC, rect: RECT, radius: i32, fill: Option<COLORREF>, line: Option<COLORREF>) {
    unsafe {
        let brush = fill.map(|c| CreateSolidBrush(c));
        let pen = CreatePen(PS_SOLID, 1, line.or(fill).unwrap_or(0));
        let old_brush = SelectObject(hdc, brush.map_or(GetStockObject(NULL_BRUSH), |b| b as HGDIOBJ));
        let old_pen = SelectObject(hdc, pen as HGDIOBJ);
        RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(pen as HGDIOBJ);
        if let Some(b) = brush {
            DeleteObject(b as HGDIOBJ);
        }
    }
}

/// Draws one menu position into `rc`: the theme's menu background (highlighted when `selected`),
/// its text, and for a highlighted template row the two buttons, `hover` lit.
#[allow(clippy::too_many_arguments)]
fn paint(
    hdc: HDC,
    rc: RECT,
    entry: &Entry,
    selected: bool,
    hover: Option<RowPart>,
    fonts: &Fonts,
    theme: HTHEME,
    scale: f64,
) {
    unsafe {
        if theme == 0 {
            let brush = GetSysColorBrush(if selected { COLOR_HIGHLIGHT } else { COLOR_MENU });
            FillRect(hdc, &rc, brush);
        } else {
            DrawThemeBackground(theme, hdc, MENU_POPUPBACKGROUND, 0, &rc, null());
            // The theme's own highlight (`MENU_POPUPITEM`) is the square light blue of Windows 10;
            // Windows 11 draws its menus with a rounded grey one, so that is drawn here instead.
            if selected {
                let background = GetPixel(hdc, rc.left + 1, (rc.top + rc.bottom) / 2);
                let inset = px(HOT_INSET, scale);
                let hot =
                    RECT { left: rc.left + inset, right: rc.right - inset, top: rc.top + 1, bottom: rc.bottom - 1 };
                rounded(hdc, hot, px(HOT_RADIUS, scale), Some(darker(background, ROW_SHADE)), None);
            }
        }
        SetBkMode(hdc, TRANSPARENT as i32);
    }
    // A themed menu highlights in a light shade and keeps dark text; the classic one inverts.
    let text = unsafe { GetSysColor(if selected && theme == 0 { COLOR_HIGHLIGHTTEXT } else { COLOR_MENUTEXT }) };
    let gray = unsafe { GetSysColor(COLOR_GRAYTEXT) };
    let middle = (rc.top + rc.bottom) / 2;
    let width = f64::from(rc.right - rc.left);
    let mut text_rect = RECT { left: rc.left + px(PAD_LEFT, scale), ..rc };
    match entry {
        Entry::Row(TemplateRow::Template { label, .. }) => {
            // The buttons' room is kept even when they are hidden, so the text never jumps.
            text_rect.right = rc.right - px(ROW_BUTTONS_WIDTH, scale);
            draw_text(hdc, fonts.text, text, label, text_rect, DT_LEFT | DT_END_ELLIPSIS);
            if selected {
                let half = px(ROW_BUTTON, scale) / 2;
                for (part, left, right) in row_buttons(width, scale) {
                    let (left, right) = (rc.left + left.round() as i32, rc.left + right.round() as i32);
                    let button = RECT { left, right, top: middle - half, bottom: middle + half };
                    let lit = hover == Some(part);
                    if lit {
                        let shade = darker(unsafe { GetPixel(hdc, button.left + 1, button.top + 1) }, BUTTON_SHADE);
                        rounded(hdc, button, px(6.0, scale), Some(shade), None);
                    }
                    let glyph = if part == RowPart::Edit { GLYPH_EDIT } else { GLYPH_DELETE };
                    draw_text(hdc, fonts.icons, if lit { text } else { gray }, glyph, button, DT_CENTER);
                }
            }
        }
        Entry::Row(TemplateRow::Deleted { message, undo, .. }) => {
            let hdc_width = unsafe {
                let screen = GetDC(null_mut());
                let w = pill_width(screen, fonts, undo, scale);
                ReleaseDC(null_mut(), screen);
                w
            };
            let right = rc.right - px(ROW_PAD_RIGHT, scale);
            let inset = px(PILL_INSET, scale);
            let pill = RECT { left: right - hdc_width, right, top: rc.top + inset, bottom: rc.bottom - inset };
            text_rect.right = pill.left - px(PAD_LEFT, scale);
            draw_text(hdc, fonts.text, gray, message, text_rect, DT_LEFT | DT_END_ELLIPSIS);
            // The whole row puts it back; the pill says so, filled while the row is highlighted.
            let fill = selected.then(|| darker(unsafe { GetPixel(hdc, pill.left + 2, middle) }, BUTTON_SHADE));
            rounded(hdc, pill, px(6.0, scale), fill, Some(gray));
            draw_text(hdc, fonts.text, text, undo, pill, DT_CENTER);
        }
        Entry::Footer(_, label) => draw_text(hdc, fonts.text, text, label, text_rect, DT_LEFT | DT_END_ELLIPSIS),
        Entry::Separator => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> TemplateList {
        crate::menu::template_list(&["hello".into(), "b".into()], Some((1, "gone")))
    }

    #[test]
    fn positions_are_the_rows_a_separator_and_the_footer() {
        let e = entries(&list());
        assert!(matches!(e[0], Entry::Row(TemplateRow::Template { index: 0, .. })));
        assert!(matches!(e[1], Entry::Row(TemplateRow::Deleted { .. })));
        assert!(matches!(e[2], Entry::Row(TemplateRow::Template { index: 1, .. })));
        assert_eq!(e[3], Entry::Separator);
        assert!(matches!(e[4], Entry::Footer(MenuAction::AddSelectionAsTemplate, _)));
        assert!(matches!(e[5], Entry::Footer(MenuAction::OpenSettings, _)));
        // No templates: no separator above the footer.
        let empty = entries(&crate::menu::template_list(&[], None));
        assert!(matches!(empty[0], Entry::Footer(..)));
    }

    #[test]
    fn the_click_decides_what_a_chosen_row_does() {
        let e = entries(&list());
        assert_eq!(choice(0, None, &e), None, "closed without a choice");
        assert_eq!(choice(1, None, &e), Some(MenuAction::InsertTemplate(0)), "Enter means the text");
        assert_eq!(choice(1, Some((0, RowPart::Delete)), &e), Some(MenuAction::DeleteTemplate(0)));
        assert_eq!(choice(3, Some((2, RowPart::Edit)), &e), Some(MenuAction::EditTemplate(1)));
        // A click noted on another row does not count for this one.
        assert_eq!(choice(3, Some((0, RowPart::Delete)), &e), Some(MenuAction::InsertTemplate(1)));
        assert_eq!(choice(2, Some((1, RowPart::Text)), &e), Some(MenuAction::UndoDeleteTemplate));
        assert_eq!(choice(6, None, &e), Some(MenuAction::OpenSettings));
        assert_eq!(choice(4, None, &e), None, "the separator");
    }

    #[test]
    fn a_shade_darker_keeps_the_hue() {
        assert_eq!(darker(0x00FF_FFFF, 10), 0x00E5_E5E5);
        assert_eq!(darker(0x0064_C800, 50), 0x0032_6400);
        assert_eq!(darker(0, 10), 0);
    }

    /// Draws the list into a PNG to look at: `VTYPE_TEMPLATE_PNG=<path> cargo test -- --ignored
    /// template_menu_png`. Rows: the first highlighted with its delete button lit, the deleted
    /// row, the second template plain, the footer.
    #[test]
    #[ignore]
    fn template_menu_png() {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        };
        let path = std::env::var("VTYPE_TEMPLATE_PNG").expect("VTYPE_TEMPLATE_PNG");
        // e.g. `ja` for the Japanese strings (English otherwise).
        if let Ok(lang) = std::env::var("VTYPE_TEMPLATE_LANG") {
            crate::i18n::init(&lang);
        }
        let scale = 1.5;
        let fonts = Fonts::new(scale);
        let texts = ["ボタンが出るか確認してください。".to_string(), "git commit push して".to_string()];
        let e = entries(&crate::menu::template_list(&texts, Some((1, "お世話になっております。"))));
        let sizes: Vec<(u32, u32)> = e
            .iter()
            .map(|x| if *x == Entry::Separator { (0, px(9.0, scale) as u32) } else { measure(x, &fonts, scale) })
            .collect();
        let width = sizes.iter().map(|s| s.0).max().unwrap() as i32 + px(40.0, scale);
        let height: i32 = sizes.iter().map(|s| s.1 as i32).sum();
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut std::ffi::c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(dc, bitmap as HGDIOBJ);
            let class = wide("MENU");
            let theme = OpenThemeData(null_mut(), class.as_ptr());
            let mut top = 0;
            for (i, (entry, (_, h))) in e.iter().zip(&sizes).enumerate() {
                let rc = RECT { left: 0, top, right: width, bottom: top + *h as i32 };
                if *entry == Entry::Separator {
                    DrawThemeBackground(theme, dc, MENU_POPUPBACKGROUND, 0, &rc, null());
                } else {
                    let hover = (i == 0).then_some(RowPart::Delete);
                    paint(dc, rc, entry, i == 0 || i == 1, hover, &fonts, theme, scale);
                }
                top += *h as i32;
            }
            let pixels = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
            let mut rgba = Vec::with_capacity(pixels.len());
            for p in pixels.chunks(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
            let pm =
                tiny_skia::Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(width as u32, height as u32).unwrap())
                    .unwrap();
            pm.save_png(&path).unwrap();
            if !theme == 0 {
                CloseThemeData(theme);
            }
            SelectObject(dc, old);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(dc);
        }
    }
}
