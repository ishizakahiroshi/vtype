//! The floating mic and its speech bubble (child plan C5-C3): layered, topmost windows that never
//! take the focus, so the app being typed into keeps it.
//!
//! Everything here runs on the UI thread. The window procedures only touch `IconShared` (Cells),
//! never the `Ui` borrowed by the job queue, because Windows calls them re-entrantly from inside
//! SetWindowPos / UpdateLayeredWindow.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC, DeleteObject, DrawTextW, EnumDisplayMonitors,
    GdiFlush, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, SetBkMode, SetTextColor, AC_SRC_ALPHA,
    AC_SRC_OVER, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CALCRECT, DT_CENTER, DT_EDITCONTROL, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
    FW_NORMAL, HDC, HFONT, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor,
    SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW, UpdateLayeredWindow, HWND_TOPMOST, IDC_HAND,
    MA_NOACTIVATE, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE,
    ULW_ALPHA, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETCURSOR,
    WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::beside_field::{follow_position, home_position, keep_on_screen, Anchor, BESIDE_SIZE};
use crate::i18n::t;
use crate::icon_draw::{draw_floating, draw_icon, draw_kept, draw_rounded_panel, to_premultiplied_bgra};
use crate::kept_bubble::{self, Area, KeptButton, Layout, CAPTION_WIDTH, TEXT_LINES, TEXT_WIDTH};
use crate::overlay_logic::{
    bubble_position, button_at, button_shown, fits, part_at, resized_position, resolve_position, scaled_size, tail,
    Gesture, MicButton, MicPart, Press, WheelSteps, SCALE_DEFAULT,
};
use crate::platform::{IconState, PlatformEvent, Rect, VoiceCue};
use crate::protocol::InputMode;
use crate::ripple::{Ripple, FRAME};

pub const ICON_CLASS: &str = "vtypeOverlay";
/// (windows-sys keeps it with the common controls.)
const WM_MOUSELEAVE: u32 = 0x02A3;
/// One notch of the wheel, and the wheel message's "Ctrl is down" bit.
const WHEEL_DELTA: f64 = 120.0;
const MK_CONTROL: usize = 0x0008;
const BUBBLE_CLASS: &str = "vtypeBubble";
const BUBBLE_WIDTH: i32 = 320;
const BUBBLE_PADDING: i32 = 10;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct IconShared {
    events: Sender<PlatformEvent>,
    /// The UI thread's message window, for requests the window procedure must not do itself.
    msg_hwnd: HWND,
    hwnd: Cell<HWND>,
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
    size: Cell<i32>,
    /// The size the user chose, in percent.
    percent: Cell<u16>,
    /// Still where it goes by default (never dragged): a new size keeps it in the corner.
    at_default: Cell<bool>,
    /// Beside the field the focus is in (`Overlay::follow`): a new size there is not saved, or
    /// the next start would put the mic beside a field that is not there.
    at_field: Cell<bool>,
    wheel: Cell<WheelSteps>,
}

/// The ripple's timer, on the mic's own window.
const TIMER_RIPPLE: usize = 1;

thread_local! {
    static ICON: RefCell<Option<Rc<IconShared>>> = const { RefCell::new(None) };
}

fn icon_shared() -> Option<Rc<IconShared>> {
    ICON.with(|slot| slot.borrow().clone())
}

fn cursor() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

fn scale_for(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// Every monitor's work area (the desktop minus the taskbar), primary first.
pub fn work_areas() -> (Vec<Rect>, Rect) {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        let list = &mut *(data as *mut Vec<Rect>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            list.push(to_rect(info.rcWork));
        }
        1
    }
    let mut list: Vec<Rect> = Vec::new();
    let primary = unsafe {
        EnumDisplayMonitors(null_mut(), null(), Some(each), &mut list as *mut Vec<Rect> as LPARAM);
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        to_rect(info.rcWork)
    };
    (list, primary)
}

fn to_rect(r: RECT) -> Rect {
    Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }
}

/// Puts premultiplied BGRA pixels on a layered window, at (x, y). `after_blit` may draw on the
/// device context before the window is updated (the bubble's text).
fn update_layered(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, bgra: &[u8], after_blit: impl FnOnce(HDC, &mut [u8])) {
    unsafe {
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = null_mut();
        let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !dib.is_null() && !bits.is_null() {
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            pixels.copy_from_slice(&bgra[..pixels.len()]);
            let old = SelectObject(mem, dib);
            after_blit(mem, pixels);
            let dst = POINT { x, y };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(hwnd, screen, &dst, &size, mem, &src, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            DeleteObject(dib);
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
    }
}

impl IconShared {
    fn render(&self) {
        let hwnd = self.hwnd.get();
        let size = self.size.get();
        let rings = self.ripple.borrow().rings();
        let pm = draw_floating(size as u32, self.look.get(), self.pointer.get(), self.mode.get(), &rings);
        let (x, y) = self.pos.get();
        update_layered(hwnd, x, y, size, size, &to_premultiplied_bgra(&pm), |_, _| {});
    }

    /// The pointer is at the screen point `at` over the mic: a new part is drawn and reported. While
    /// pressed the mic moves with the pointer, so the part stays the one pressed.
    fn point_at(&self, at: (i32, i32)) {
        if self.press.get().is_some() && self.pointer.get().is_some() {
            return;
        }
        let (x, y) = self.pos.get();
        let part = part_at((at.0 - x) as f32, (at.1 - y) as f32, self.size.get() as f32);
        if self.pointer.replace(Some(part)) != Some(part) {
            self.render();
            let _ = self.events.send(PlatformEvent::MicHover(Some(part)));
        }
    }

    fn pointer_left(&self) {
        if self.pointer.take().is_some() {
            self.render();
            let _ = self.events.send(PlatformEvent::MicHover(None));
        }
    }

    /// The corner button under the screen point `at`, among those on screen now.
    fn button_under(&self, at: (i32, i32)) -> Option<MicButton> {
        let (x, y) = self.pos.get();
        let busy_or_hovered = self.pointer.get().is_some() || self.look.get() != IconState::Idle;
        let normal = self.mode.get() == InputMode::Normal;
        button_at((at.0 - x) as f32, (at.1 - y) as f32, self.size.get() as f32, |b| {
            button_shown(b, busy_or_hovered, normal)
        })
    }

    fn set_look(&self, look: IconState) {
        self.look.set(look);
        self.ripple.borrow_mut().set_recording(look == IconState::Recording);
    }

    fn cue(&self, cue: VoiceCue) {
        self.ripple.borrow_mut().cue(cue);
        if self.ripple.borrow().active() && !self.animating.replace(true) {
            self.last_frame.set(Instant::now());
            unsafe { SetTimer(self.hwnd.get(), TIMER_RIPPLE, FRAME.as_millis() as u32, None) };
        }
    }

    /// One step of the ripple; the timer stops once nothing is left to draw.
    fn frame(&self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame.replace(now)).as_secs_f32();
        self.ripple.borrow_mut().tick(dt);
        self.render();
        if !self.ripple.borrow().active() {
            unsafe { KillTimer(self.hwnd.get(), TIMER_RIPPLE) };
            self.animating.set(false);
        }
    }
}

unsafe extern "system" fn icon_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(s) = icon_shared() else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    match msg {
        // Clicking the mic must leave the focus in the app being typed into.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
            1
        }
        WM_LBUTTONDOWN => {
            SetCapture(hwnd);
            let at = cursor();
            s.press.set(Some(Press::new(at)));
            s.press_origin.set(s.pos.get());
            s.press_button.set(s.button_under(at));
            0
        }
        WM_MOUSEMOVE => {
            if s.pointer.get().is_none() {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
            s.point_at(cursor());
            if let Some(mut press) = s.press.get() {
                let now = cursor();
                if press.moved(now) {
                    let (dx, dy) = press.offset(now);
                    let (ox, oy) = s.press_origin.get();
                    let pos = (ox + dx, oy + dy);
                    s.pos.set(pos);
                    SetWindowPos(hwnd, HWND_TOPMOST, pos.0, pos.1, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                }
                s.press.set(Some(press));
            }
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            if let Some(press) = s.press.take() {
                let event = match press.release(cursor()) {
                    Gesture::Click => match s.press_button.take() {
                        Some(button) => PlatformEvent::MicButton(button),
                        None => PlatformEvent::ToggleRequested,
                    },
                    Gesture::Drag => {
                        s.at_default.set(false);
                        s.at_field.set(false);
                        let (x, y) = s.pos.get();
                        PlatformEvent::IconMoved { x, y }
                    }
                };
                let _ = s.events.send(event);
            }
            0
        }
        // Ctrl+wheel resizes the mic. (Windows sends the wheel to the window under the pointer
        // unless "scroll inactive windows" is switched off.)
        WM_MOUSEWHEEL if wparam & MK_CONTROL != 0 => {
            let delta = f64::from(((wparam >> 16) & 0xFFFF) as u16 as i16);
            let mut wheel = s.wheel.get();
            let steps = wheel.add(delta, WHEEL_DELTA);
            s.wheel.set(wheel);
            if steps != 0 {
                let _ = s.events.send(PlatformEvent::IconZoom { steps });
            }
            0
        }
        WM_MOUSELEAVE => {
            s.pointer_left();
            0
        }
        WM_RBUTTONUP => {
            // The menu belongs to the UI state, which may be borrowed right now.
            PostMessageW(s.msg_hwnd, super::ui::WM_APP_MENU, 0, 0);
            0
        }
        WM_TIMER if wparam == TIMER_RIPPLE => {
            s.frame();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn bubble_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // "Insert" types into the app that has the focus: the bubble must not take it.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR if kept_button_under_pointer().is_some() => {
            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
            1
        }
        WM_LBUTTONDOWN => {
            KEPT_PRESS.with(|p| p.set(kept_button_under_pointer()));
            0
        }
        // A button counts when the press started and ended on it.
        WM_LBUTTONUP => {
            let pressed = KEPT_PRESS.with(Cell::take);
            if let (Some(button), Some(s)) =
                (pressed.filter(|b| kept_button_under_pointer() == Some(*b)), icon_shared())
            {
                let _ = s.events.send(PlatformEvent::KeptButton(button));
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_class(name: &str, proc_: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT) {
    let name_w = wide(name);
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: GetModuleHandleW(null()),
            lpszClassName: name_w.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&class);
    }
}

fn create_layered(class: &str) -> HWND {
    let class_w = wide(class);
    let title = wide("vtype");
    unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class_w.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    }
}

pub struct Overlay {
    icon: Rc<IconShared>,
    bubble: HWND,
    shown: bool,
    bubble_shown: bool,
}

impl Overlay {
    pub fn new(events: Sender<PlatformEvent>, msg_hwnd: HWND) -> Overlay {
        register_class(ICON_CLASS, icon_proc);
        register_class(BUBBLE_CLASS, bubble_proc);
        let hwnd = create_layered(ICON_CLASS);
        let bubble = create_layered(BUBBLE_CLASS);
        let icon = Rc::new(IconShared {
            events,
            msg_hwnd,
            hwnd: Cell::new(hwnd),
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
            size: Cell::new(scaled_size(SCALE_DEFAULT, 1.0)),
            percent: Cell::new(SCALE_DEFAULT),
            at_default: Cell::new(true),
            at_field: Cell::new(false),
            wheel: Cell::new(WheelSteps::default()),
        });
        ICON.with(|slot| *slot.borrow_mut() = Some(icon.clone()));
        Overlay { icon, bubble, shown: false, bubble_shown: false }
    }

    pub fn hwnd(&self) -> HWND {
        self.icon.hwnd.get()
    }

    /// Shows the mic in `look`, at the saved position when it is still on a screen.
    pub fn show(&mut self, look: IconState, saved: Option<(i32, i32)>) {
        let hwnd = self.icon.hwnd.get();
        let size = scaled_size(self.icon.percent.get(), scale_for(hwnd).into());
        self.icon.size.set(size);
        if !self.shown {
            let (areas, primary) = work_areas();
            self.icon.pos.set(resolve_position(saved, size, &areas, primary));
            self.icon.at_default.set(saved.is_none_or(|p| !fits(p, size, &areas)));
            self.icon.at_field.set(false);
        }
        self.icon.set_look(look);
        self.icon.render();
        if !self.shown {
            unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
            self.shown = true;
        }
    }

    pub fn set_look(&mut self, look: IconState) {
        self.icon.set_look(look);
        if self.shown {
            self.icon.render();
        }
    }

    /// The mic's size in percent. On screen it changes around its centre (or stays in the
    /// corner); a mic the user moved reports where it went so the position is saved.
    pub fn set_scale(&mut self, percent: u16) {
        if self.icon.percent.replace(percent) == percent || !self.shown {
            return;
        }
        self.hide_bubble();
        let old = self.icon.size.get();
        let new = scaled_size(percent, scale_for(self.icon.hwnd.get()).into());
        let (areas, primary) = work_areas();
        let at_default = self.icon.at_default.get();
        let pos = resized_position(self.icon.pos.get(), old, new, at_default, &areas, primary);
        self.icon.size.set(new);
        self.icon.pos.set(pos);
        self.icon.render();
        if !at_default && !self.icon.at_field.get() {
            let _ = self.icon.events.send(PlatformEvent::IconMoved { x: pos.0, y: pos.1 });
        }
    }

    /// The ripple follows what the recognizer heard (only while the mic is on screen).
    pub fn voice_cue(&mut self, cue: VoiceCue) {
        if self.shown {
            self.icon.cue(cue);
        }
    }

    pub fn look(&self) -> IconState {
        self.icon.look.get()
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        if self.icon.mode.replace(mode) != mode && self.shown {
            self.icon.render();
        }
    }

    pub fn hide(&mut self) {
        unsafe { ShowWindow(self.icon.hwnd.get(), SW_HIDE) };
        self.shown = false;
        self.hide_bubble();
    }

    /// Whether the mic may be moved for the user: it is on screen, not held, and the pointer is
    /// not on it (the focus coming back from a template menu must not pull it from under the hand).
    fn free_to_move(&self) -> bool {
        if !self.shown || self.icon.press.get().is_some() {
            return false;
        }
        let size = self.icon.size.get();
        let (x, y) = self.icon.pos.get();
        let (cx, cy) = cursor();
        !(cx >= x && cx < x + size && cy >= y && cy < y + size)
    }

    /// Puts the mic at `pos`, and reports whether it moved.
    fn move_to(&mut self, pos: (i32, i32)) -> bool {
        if pos == self.icon.pos.get() {
            return false;
        }
        // The bubble was placed over the old spot; the next words place it over the new one.
        self.hide_bubble();
        self.icon.pos.set(pos);
        unsafe { SetWindowPos(self.icon.hwnd.get(), HWND_TOPMOST, pos.0, pos.1, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE) };
        true
    }

    /// Brings the mic to the field the user clicked or tabbed into, and reports whether it moved.
    /// It stays there until `go_home`; the saved position is left alone, so the next start is
    /// where the user put it. Not while it is held, nor while the pointer is on it.
    pub fn follow(&mut self, anchor: Anchor) -> bool {
        if !self.free_to_move() {
            return false;
        }
        let (areas, primary) = work_areas();
        let pos = follow_position(anchor, self.icon.size.get(), &areas, primary);
        if !self.move_to(pos) {
            return false;
        }
        // A later resize keeps it here instead of sending it back to the corner, unsaved.
        self.icon.at_default.set(false);
        self.icon.at_field.set(true);
        true
    }

    /// Sends the mic back to the bottom-right corner of the screen the pointer is on (the focus
    /// left the fields), and reports whether it moved. Not saved: the next start is where the user
    /// put it. A later resize keeps it in that corner. Not while it is held, nor while the pointer
    /// is on it.
    pub fn go_home(&mut self) -> bool {
        if !self.free_to_move() {
            return false;
        }
        let (areas, primary) = work_areas();
        let pos = home_position(cursor(), self.icon.size.get(), &areas, primary);
        self.icon.at_default.set(true);
        self.icon.at_field.set(false);
        self.move_to(pos)
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// At most `max_lines`; a longer text loses its start (live text: the latest words matter).
    /// As wide as the text, up to `BUBBLE_WIDTH`, so it sits centred over the mic.
    pub fn show_bubble(&mut self, text: &str, max_lines: i32) {
        KEPT.with(|k| k.set(None));
        let scale = scale_for(self.icon.hwnd.get());
        let max_width = (BUBBLE_WIDTH as f32 * scale).round() as i32;
        let pad = (BUBBLE_PADDING as f32 * scale).round() as i32;
        let font_px = (14.0 * scale).round() as i32;
        unsafe {
            let font = message_font(font_px);
            // Measure: at most `max_lines`; drop the start until it fits.
            let screen = GetDC(null_mut());
            let measure = CreateCompatibleDC(screen);
            let old_font = SelectObject(measure, font);
            let flags = DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;
            let mut one = wide("Xg");
            let mut line = RECT { left: 0, top: 0, right: max_width - pad * 2, bottom: 0 };
            DrawTextW(measure, one.as_mut_ptr(), -1, &mut line, flags | DT_CALCRECT);
            let max_height = line.bottom * max_lines + 1;
            let mut shown = text.to_string();
            let mut limit = text.chars().count();
            // DT_CALCRECT narrows `right` to the widest line when the text is shorter.
            let (text_width, text_height) = loop {
                let mut w = wide(&shown);
                let mut r = RECT { left: 0, top: 0, right: max_width - pad * 2, bottom: 0 };
                DrawTextW(measure, w.as_mut_ptr(), -1, &mut r, flags | DT_CALCRECT);
                if r.bottom <= max_height || limit <= 4 {
                    break (r.right.min(max_width - pad * 2), r.bottom.min(max_height));
                }
                limit = (limit * 4 / 5).max(4);
                shown = tail(text, limit);
            };
            SelectObject(measure, old_font);
            DeleteDC(measure);
            ReleaseDC(null_mut(), screen);

            let (width, height) = (text_width + pad * 2, text_height + pad * 2);
            let panel = draw_rounded_panel(width as u32, height as u32, 10.0 * scale, (255, 255, 255, 250));
            let alpha: Vec<u8> = panel.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            let (areas, primary) = work_areas();
            let gap = (8.0 * scale) as i32;
            let (x, y) =
                bubble_position(self.icon.pos.get(), self.icon.size.get(), (width, height), gap, &areas, primary);
            update_layered(self.bubble, x, y, width, height, &to_premultiplied_bgra(&panel), |dc, pixels| {
                let old = SelectObject(dc, font);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, 0x0037_291f); // #1f2937 as 0x00BBGGRR
                let mut w = wide(&shown);
                let mut r = RECT { left: pad, top: pad, right: width - pad, bottom: height - pad };
                DrawTextW(dc, w.as_mut_ptr(), -1, &mut r, flags);
                GdiFlush();
                SelectObject(dc, old);
                // GDI writes alpha 0 where it draws; the body is opaque, so give it back.
                for (i, a) in alpha.iter().enumerate() {
                    if *a >= 250 {
                        pixels[i * 4 + 3] = *a;
                    }
                }
            });
            DeleteObject(font);
            if !self.bubble_shown {
                ShowWindow(self.bubble, SW_SHOWNOACTIVATE);
                self.bubble_shown = true;
            }
        }
    }

    /// The kept words' bubble (plan C11) above the mic, with its buttons.
    pub fn show_kept(&mut self, words: &str) {
        let scale = scale_for(self.icon.hwnd.get());
        let labels = [t("native_keptCaption"), t("native_keptCopy"), t("native_keptInsert")];
        let (layout, width, height, bgra) = render_kept(words, [&labels[0], &labels[1], &labels[2]], scale);
        let (areas, primary) = work_areas();
        let gap = (8.0 * scale) as i32;
        let (x, y) = bubble_position(self.icon.pos.get(), self.icon.size.get(), (width, height), gap, &areas, primary);
        update_layered(self.bubble, x, y, width, height, &bgra, |_, _| {});
        KEPT.with(|k| k.set(Some(KeptHit { layout, scale, origin: (x, y) })));
        if !self.bubble_shown {
            unsafe { ShowWindow(self.bubble, SW_SHOWNOACTIVATE) };
            self.bubble_shown = true;
        }
    }

    pub fn hide_bubble(&mut self) {
        KEPT.with(|k| k.set(None));
        if self.bubble_shown {
            unsafe { ShowWindow(self.bubble, SW_HIDE) };
            self.bubble_shown = false;
        }
    }
}

/// The system's message font at `px` pixels (Yu Gothic UI on Japanese Windows): "Segoe UI" has
/// no Japanese, and GDI's stand-in draws the kana shrunk beside the Latin letters.
unsafe fn message_font(px: i32) -> HFONT {
    let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, &mut metrics as *mut NONCLIENTMETRICSW as *mut _, 0);
    let mut logfont = metrics.lfMessageFont;
    logfont.lfHeight = -px;
    logfont.lfWidth = 0;
    logfont.lfWeight = FW_NORMAL as i32;
    // Not ClearType: its colored fringes are wrong on a layered window's alpha.
    logfont.lfQuality = ANTIALIASED_QUALITY;
    CreateFontIndirectW(&logfont)
}

// --- the kept words' bubble (plan C11) ----------------------------------------------------------

/// Where the kept words' bubble is and what is in it, for its window procedure.
#[derive(Clone, Copy)]
struct KeptHit {
    layout: Layout,
    /// Pixels per unit of `layout`.
    scale: f32,
    origin: (i32, i32),
}

thread_local! {
    /// Set while the bubble shows kept words.
    static KEPT: Cell<Option<KeptHit>> = const { Cell::new(None) };
    /// The kept words' button the press started on.
    static KEPT_PRESS: Cell<Option<KeptButton>> = const { Cell::new(None) };
}

/// The kept words' button under the pointer, while the bubble shows them.
fn kept_button_under_pointer() -> Option<KeptButton> {
    let hit = KEPT.with(Cell::get)?;
    let (x, y) = cursor();
    hit.layout.button_at((x - hit.origin.0) as f32 / hit.scale, (y - hit.origin.1) as f32 / hit.scale)
}

const WRAPPED: DRAW_TEXT_FORMAT = DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;
const CENTRED: DRAW_TEXT_FORMAT = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX;

/// The size of `text` in `font`: wrapped at `wrap` pixels, or on one line.
unsafe fn measure(dc: HDC, font: HFONT, text: &str, wrap: Option<i32>) -> (i32, i32) {
    let old = SelectObject(dc, font);
    let mut w = wide(text);
    let (flags, right) = match wrap {
        Some(width) => (WRAPPED, width),
        None => (DT_SINGLELINE | DT_NOPREFIX, 0),
    };
    let mut r = RECT { left: 0, top: 0, right, bottom: 0 };
    DrawTextW(dc, w.as_mut_ptr(), -1, &mut r, flags | DT_CALCRECT);
    SelectObject(dc, old);
    (r.right, r.bottom)
}

unsafe fn draw_text(dc: HDC, font: HFONT, text: &str, mut rect: RECT, flags: DRAW_TEXT_FORMAT, rgb: (u8, u8, u8)) {
    let old = SelectObject(dc, font);
    SetTextColor(dc, u32::from(rgb.0) | u32::from(rgb.1) << 8 | u32::from(rgb.2) << 16);
    let mut w = wide(text);
    DrawTextW(dc, w.as_mut_ptr(), -1, &mut rect, flags);
    SelectObject(dc, old);
}

/// Draws the kept words' bubble at `scale` pixels per unit: its layout, its size in pixels, and
/// its premultiplied BGRA. `labels` are the caption and the two buttons'.
fn render_kept(words: &str, labels: [&str; 3], scale: f32) -> (Layout, i32, i32, Vec<u8>) {
    let [caption, copy, insert] = labels;
    let px = |units: f32| (units * scale).round() as i32;
    let unit = |pixels: i32| pixels as f32 / scale;
    unsafe {
        let caption_font = message_font(px(12.0));
        let words_font = message_font(px(14.0));
        let button_font = message_font(px(13.0));
        let dc = CreateCompatibleDC(null_mut());
        // At most TEXT_LINES of the words: the latest, as the speech bubble shows them.
        let (_, caption_height) = measure(dc, caption_font, caption, Some(px(CAPTION_WIDTH)));
        let (_, line) = measure(dc, words_font, "Xg", Some(px(TEXT_WIDTH)));
        let max_height = line * TEXT_LINES + 1;
        let mut shown = words.to_string();
        let mut limit = words.chars().count();
        let words_height = loop {
            let (_, h) = measure(dc, words_font, &shown, Some(px(TEXT_WIDTH)));
            if h <= max_height || limit <= 4 {
                break h.min(max_height);
            }
            limit = (limit * 4 / 5).max(4);
            shown = tail(words, limit);
        };
        let (copy_width, _) = measure(dc, button_font, copy, None);
        let (insert_width, _) = measure(dc, button_font, insert, None);
        let layout =
            kept_bubble::layout(unit(caption_height), unit(words_height), unit(copy_width), unit(insert_width));

        let panel = draw_kept(&layout, scale);
        let (width, height) = (panel.width() as i32, panel.height() as i32);
        let alpha: Vec<u8> = panel.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = null_mut();
        let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mut out = to_premultiplied_bgra(&panel);
        if !dib.is_null() && !bits.is_null() {
            let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, out.len());
            pixels.copy_from_slice(&out);
            let old = SelectObject(dc, dib);
            SetBkMode(dc, TRANSPARENT as i32);
            let rect =
                |a: Area| RECT { left: px(a.x), top: px(a.y), right: px(a.x + a.width), bottom: px(a.y + a.height) };
            draw_text(dc, caption_font, caption, rect(layout.caption), WRAPPED, kept_bubble::CAPTION_COLOR);
            draw_text(dc, words_font, &shown, rect(layout.text), WRAPPED, kept_bubble::TEXT_COLOR);
            draw_text(dc, button_font, copy, rect(layout.copy), CENTRED, kept_bubble::COPY_LABEL_COLOR);
            draw_text(dc, button_font, insert, rect(layout.insert), CENTRED, kept_bubble::INSERT_LABEL_COLOR);
            GdiFlush();
            // GDI writes alpha 0 where it draws; the body is opaque, so give it back.
            for (i, a) in alpha.iter().enumerate() {
                if *a >= 250 {
                    pixels[i * 4 + 3] = *a;
                }
            }
            out.copy_from_slice(pixels);
            SelectObject(dc, old);
            DeleteObject(dib);
        }
        DeleteDC(dc);
        for font in [caption_font, words_font, button_font] {
            DeleteObject(font);
        }
        (layout, width, height, out)
    }
}

// --- the mic beside the text field (child plan C8) ---------------------------------------------

const BESIDE_CLASS: &str = "vtypeBeside";

thread_local! {
    static BESIDE_EVENTS: RefCell<Option<Sender<PlatformEvent>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn beside_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Pressing it must leave the focus in the field it sits beside.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_HAND));
            1
        }
        WM_LBUTTONUP => {
            BESIDE_EVENTS.with(|slot| {
                if let Some(events) = slot.borrow().as_ref() {
                    let _ = events.send(PlatformEvent::ToggleRequested);
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// The small mic next to the focused field: a layered window like the floating mic, 26 px, always
/// opaque (it only shows while a field has the focus), no dragging.
pub struct BesideMic {
    hwnd: HWND,
    shown: bool,
    look: IconState,
    pos: (i32, i32),
}

impl BesideMic {
    pub fn new(events: Sender<PlatformEvent>) -> BesideMic {
        register_class(BESIDE_CLASS, beside_proc);
        BESIDE_EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
        BesideMic { hwnd: create_layered(BESIDE_CLASS), shown: false, look: IconState::Idle, pos: (0, 0) }
    }

    fn size(&self) -> i32 {
        (BESIDE_SIZE as f32 * scale_for(self.hwnd)).round() as i32
    }

    fn render(&self) {
        let size = self.size();
        let pm = draw_icon(size as u32, self.look, true);
        update_layered(self.hwnd, self.pos.0, self.pos.1, size, size, &to_premultiplied_bgra(&pm), |_, _| {});
    }

    /// Puts the mic at `pos` (screen pixels), kept on that screen.
    pub fn show(&mut self, pos: (i32, i32), look: IconState) {
        let (areas, primary) = work_areas();
        self.pos = keep_on_screen(pos, self.size(), &areas, primary);
        self.look = look;
        self.render();
        if !self.shown {
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
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
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.shown = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{lookup, Lang};

    /// Draws the kept words' bubble into a PNG to look at: `VTYPE_KEPT_PNG=<path> cargo test --
    /// --ignored kept_bubble_png`. Japanese at 150 % above English at 100 %, each on a grey
    /// backdrop so the rounded corners show.
    #[test]
    #[ignore]
    fn kept_bubble_png() {
        let path = std::env::var("VTYPE_KEPT_PNG").expect("VTYPE_KEPT_PNG");
        let labels = |lang| ["native_keptCaption", "native_keptCopy", "native_keptInsert"].map(|key| lookup(lang, key));
        let ja = labels(Lang::Ja);
        let en = labels(Lang::En);
        let words = "明日の会議は 10 時からに変更になりました。資料は前日までに共有します。よろしくお願いします。";
        let bubbles = [
            render_kept(words, [&ja[0], &ja[1], &ja[2]], 1.5),
            render_kept("Please send me the latest version of the slides.", [&en[0], &en[1], &en[2]], 1.0),
        ];
        let margin = 16;
        let width = bubbles.iter().map(|b| b.1).max().unwrap() + margin * 2;
        let height = bubbles.iter().map(|b| b.2 + margin).sum::<i32>() + margin;
        let mut sheet = tiny_skia::Pixmap::new(width as u32, height as u32).unwrap();
        sheet.fill(tiny_skia::Color::from_rgba8(0x9c, 0xa3, 0xaf, 255));
        let mut top = margin;
        for (_, w, h, bgra) in &bubbles {
            let rgba: Vec<u8> = bgra.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            let size = tiny_skia::IntSize::from_wh(*w as u32, *h as u32).unwrap();
            let bubble = tiny_skia::Pixmap::from_vec(rgba, size).unwrap();
            let paint = tiny_skia::PixmapPaint::default();
            sheet.draw_pixmap(margin, top, bubble.as_ref(), &paint, tiny_skia::Transform::identity(), None);
            top += h + margin;
        }
        sheet.save_png(&path).unwrap();
    }
}
