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
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, EnumDisplayMonitors,
    GdiFlush, GetDC, GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, SetBkMode, SetTextColor, AC_SRC_ALPHA,
    AC_SRC_OVER, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CALCRECT, DT_EDITCONTROL, DT_NOPREFIX, DT_WORDBREAK, FF_DONTCARE, FW_NORMAL,
    HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, KillTimer, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor,
    SetTimer, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, IDC_HAND, MA_NOACTIVATE, SWP_NOACTIVATE,
    SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::beside_field::{keep_on_screen, BESIDE_SIZE};
use crate::icon_draw::{draw_floating, draw_icon, draw_rounded_panel, to_premultiplied_bgra};
use crate::overlay_logic::{
    button_at, button_shown, fits, resized_position, resolve_position, scaled_size, tail, Gesture, MicButton, Press,
    WheelSteps, SCALE_DEFAULT,
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
    hover: Cell<bool>,
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
        let pm = draw_floating(size as u32, self.look.get(), self.hover.get(), self.mode.get(), &rings);
        let (x, y) = self.pos.get();
        update_layered(hwnd, x, y, size, size, &to_premultiplied_bgra(&pm), |_, _| {});
    }

    /// The corner button under the screen point `at`, among those on screen now.
    fn button_under(&self, at: (i32, i32)) -> Option<MicButton> {
        let (x, y) = self.pos.get();
        let busy_or_hovered = self.hover.get() || self.look.get() != IconState::Idle;
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
            if !s.hover.get() {
                s.hover.set(true);
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
                s.render();
            }
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
            s.hover.set(false);
            s.render();
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
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
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
            hover: Cell::new(false),
            press: Cell::new(None),
            press_origin: Cell::new((0, 0)),
            press_button: Cell::new(None),
            pos: Cell::new((0, 0)),
            size: Cell::new(scaled_size(SCALE_DEFAULT, 1.0)),
            percent: Cell::new(SCALE_DEFAULT),
            at_default: Cell::new(true),
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
    /// corner); a moved mic reports where it went so the position is saved.
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
        if !at_default {
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

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// At most `max_lines`; a longer text loses its start (live text: the latest words matter).
    pub fn show_bubble(&mut self, text: &str, max_lines: i32) {
        let scale = scale_for(self.icon.hwnd.get());
        let width = (BUBBLE_WIDTH as f32 * scale).round() as i32;
        let pad = (BUBBLE_PADDING as f32 * scale).round() as i32;
        let font_px = (14.0 * scale).round() as i32;
        let face = wide("Segoe UI");
        unsafe {
            let font = CreateFontW(
                -font_px,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                FF_DONTCARE as u32,
                face.as_ptr(),
            );
            // Measure: at most `max_lines`; drop the start until it fits.
            let screen = GetDC(null_mut());
            let measure = CreateCompatibleDC(screen);
            let old_font = SelectObject(measure, font);
            let flags = DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;
            let mut one = wide("Xg");
            let mut line = RECT { left: 0, top: 0, right: width - pad * 2, bottom: 0 };
            DrawTextW(measure, one.as_mut_ptr(), -1, &mut line, flags | DT_CALCRECT);
            let max_height = line.bottom * max_lines + 1;
            let mut shown = text.to_string();
            let mut limit = text.chars().count();
            let text_height = loop {
                let mut w = wide(&shown);
                let mut r = RECT { left: 0, top: 0, right: width - pad * 2, bottom: 0 };
                DrawTextW(measure, w.as_mut_ptr(), -1, &mut r, flags | DT_CALCRECT);
                if r.bottom <= max_height || limit <= 4 {
                    break r.bottom.min(max_height);
                }
                limit = (limit * 4 / 5).max(4);
                shown = tail(text, limit);
            };
            SelectObject(measure, old_font);
            DeleteDC(measure);
            ReleaseDC(null_mut(), screen);

            let height = text_height + pad * 2;
            let panel = draw_rounded_panel(width as u32, height as u32, 10.0 * scale, (255, 255, 255, 250));
            let alpha: Vec<u8> = panel.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect();
            let (ix, iy) = self.icon.pos.get();
            let size = self.icon.size.get();
            let (areas, primary) = work_areas();
            let area = areas
                .iter()
                .copied()
                .find(|a| ix >= a.x && ix < a.x + a.width && iy >= a.y && iy < a.y + a.height)
                .unwrap_or(primary);
            let x = (ix + size - width).clamp(area.x, area.x + area.width - width);
            let y = (iy - height - (8.0 * scale) as i32).max(area.y);
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

    pub fn hide_bubble(&mut self) {
        if self.bubble_shown {
            unsafe { ShowWindow(self.bubble, SW_HIDE) };
            self.bubble_shown = false;
        }
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
