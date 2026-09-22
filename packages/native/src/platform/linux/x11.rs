//! X11 (child plan C7-C2, C7-C3): typing with XTest, and whether the active window is full screen.
//!
//! Typing works like xdotool: keys that type nothing on this keyboard are lent a keysym for a
//! moment, pressed through XTest, and given back afterwards. That types any character, whatever
//! the keyboard layout.

use std::cell::RefCell;
use std::thread;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, Keycode, Keysym, Window, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use crate::config::InjectMethod;
use crate::linux_setup::{keysym_batches, keysym_for, XK_CONTROL_L, XK_RETURN, XK_TAB, XK_V};
use crate::platform::{InjectOutcome, PlatformError};

/// How long an app gets to notice a changed keyboard mapping before the keys come.
const MAPPING_SETTLE: Duration = Duration::from_millis(20);
const PASTE_RESTORE_DELAY: Duration = Duration::from_millis(300);
/// At most this many keys are lent at once.
const MAX_SPARE: usize = 16;

fn err(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(e.to_string())
}

struct Keyboard {
    conn: RustConnection,
    root: Window,
    min: Keycode,
    per: u8,
    /// `per` keysyms for each keycode from `min` up.
    map: Vec<Keysym>,
}

impl Keyboard {
    fn open() -> Result<Keyboard, PlatformError> {
        let (conn, screen) = x11rb::connect(None).map_err(err)?;
        conn.xtest_get_version(2, 2).map_err(err)?.reply().map_err(|_| err("the XTEST extension is missing"))?;
        let setup = conn.setup();
        let root = setup.roots[screen].root;
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn.get_keyboard_mapping(min, max - min + 1).map_err(err)?.reply().map_err(err)?;
        Ok(Keyboard { conn, root, min, per: reply.keysyms_per_keycode, map: reply.keysyms })
    }

    fn codes(&self) -> impl Iterator<Item = (Keycode, &[Keysym])> {
        let per = self.per.max(1) as usize;
        self.map.chunks(per).enumerate().map(move |(i, syms)| (self.min + i as u8, syms))
    }

    /// A key whose first (unshifted) keysym is `keysym`.
    fn keycode_for(&self, keysym: Keysym) -> Option<Keycode> {
        self.codes().find(|(_, syms)| syms.first() == Some(&keysym)).map(|(code, _)| code)
    }

    /// Keys that type nothing, from the top of the range down.
    fn spare_keycodes(&self) -> Vec<Keycode> {
        let mut spare: Vec<Keycode> =
            self.codes().filter(|(_, syms)| syms.iter().all(|s| *s == 0)).map(|(code, _)| code).collect();
        spare.reverse();
        spare.truncate(MAX_SPARE);
        spare
    }

    fn set_mapping(&self, code: Keycode, keysym: Keysym) -> Result<(), PlatformError> {
        let syms = vec![keysym; self.per.max(1) as usize];
        self.conn.change_keyboard_mapping(1, code, self.per.max(1), &syms).map_err(err)?;
        Ok(())
    }

    fn key(&self, code: Keycode, down: bool) -> Result<(), PlatformError> {
        let kind = if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT };
        self.conn.xtest_fake_input(kind, code, CURRENT_TIME, self.root, 0, 0, 0).map_err(err)?;
        Ok(())
    }

    fn tap(&self, code: Keycode) -> Result<(), PlatformError> {
        self.key(code, true)?;
        self.key(code, false)
    }

    /// Waits until the server has handled everything sent so far.
    fn sync(&self) -> Result<(), PlatformError> {
        self.conn.get_input_focus().map_err(err)?.reply().map_err(err)?;
        Ok(())
    }

    /// Types `keysyms` on lent keys, a group at a time, and gives the keys back.
    fn type_keysyms(&self, keysyms: &[Keysym], spare: &[Keycode]) -> Result<(), PlatformError> {
        for batch in keysym_batches(keysyms, spare.len()) {
            let mut lent: Vec<(Keysym, Keycode)> = Vec::new();
            for &k in &batch {
                if !lent.iter().any(|(s, _)| *s == k) {
                    let code = spare[lent.len()];
                    self.set_mapping(code, k)?;
                    lent.push((k, code));
                }
            }
            self.sync()?;
            thread::sleep(MAPPING_SETTLE);
            for k in &batch {
                if let Some((_, code)) = lent.iter().find(|(s, _)| s == k) {
                    self.tap(*code)?;
                }
            }
            self.sync()?;
            thread::sleep(MAPPING_SETTLE);
            for (_, code) in &lent {
                self.set_mapping(*code, 0)?;
            }
        }
        self.sync()
    }

    /// A named key (Return, Tab) on its own keycode, or lent one when the keyboard has none.
    fn press_named(&self, keysym: Keysym, spare: &[Keycode]) -> Result<(), PlatformError> {
        match self.keycode_for(keysym) {
            Some(code) => {
                self.tap(code)?;
                self.sync()
            }
            None => self.type_keysyms(&[keysym], spare),
        }
    }
}

pub fn type_text(text: &str) -> Result<(), PlatformError> {
    let kb = Keyboard::open()?;
    let spare = kb.spare_keycodes();
    if spare.is_empty() {
        return Err(err("no free key to lend on this keyboard"));
    }
    let mut run: Vec<Keysym> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let named = match c {
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                Some(XK_RETURN)
            }
            '\t' => Some(XK_TAB),
            _ => None,
        };
        match named {
            Some(keysym) => {
                kb.type_keysyms(&std::mem::take(&mut run), &spare)?;
                kb.press_named(keysym, &spare)?;
            }
            None => run.push(keysym_for(c)),
        }
    }
    kb.type_keysyms(&run, &spare)
}

pub fn paste_text(text: &str) -> Result<(), PlatformError> {
    let kb = Keyboard::open()?;
    let (Some(ctrl), Some(v)) = (kb.keycode_for(XK_CONTROL_L), kb.keycode_for(XK_V)) else {
        return Err(err("no Control or V key on this keyboard"));
    };
    let mut clipboard = arboard::Clipboard::new().map_err(err)?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text.to_string()).map_err(err)?;
    kb.key(ctrl, true)?;
    kb.tap(v)?;
    kb.key(ctrl, false)?;
    kb.sync()?;
    thread::sleep(PASTE_RESTORE_DELAY);
    match previous {
        Some(old) => {
            let _ = clipboard.set_text(old);
        }
        None => tracing::info!("the clipboard held no text before; it keeps the pasted text"),
    }
    Ok(())
}

pub fn inject(text: &str, method: InjectMethod) -> Result<InjectOutcome, PlatformError> {
    if method == InjectMethod::Paste {
        return paste_text(text).map(|()| InjectOutcome::Pasted);
    }
    match type_text(text) {
        Ok(()) => Ok(InjectOutcome::Typed),
        Err(e) if method == InjectMethod::Auto => {
            tracing::info!(error = %e, "typing failed; pasting instead");
            paste_text(text).map(|()| InjectOutcome::Pasted)
        }
        Err(e) => Err(e),
    }
}

struct Watch {
    conn: RustConnection,
    root: Window,
    active: u32,
    state: u32,
    fullscreen: u32,
}

thread_local! {
    static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

fn open_watch() -> Option<Watch> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &[u8]| conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom);
    let (active, state, fullscreen) =
        (atom(b"_NET_ACTIVE_WINDOW")?, atom(b"_NET_WM_STATE")?, atom(b"_NET_WM_STATE_FULLSCREEN")?);
    Some(Watch { conn, root, active, state, fullscreen })
}

impl Watch {
    fn active_is_fullscreen(&self) -> Option<bool> {
        let win = self
            .conn
            .get_property(false, self.root, self.active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()?;
        if win == 0 {
            return Some(false);
        }
        let states = self.conn.get_property(false, win, self.state, AtomEnum::ATOM, 0, 64).ok()?.reply().ok()?;
        let fullscreen = states.value32()?.any(|a| a == self.fullscreen);
        Some(fullscreen)
    }
}

/// Whether the active window asked the window manager for full screen. No answer means no.
pub fn active_window_is_fullscreen() -> bool {
    WATCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = open_watch();
        }
        match slot.as_ref().map(|w| w.active_is_fullscreen()) {
            Some(Some(v)) => v,
            Some(None) => {
                *slot = None; // the connection may have gone; try again next time
                false
            }
            None => false,
        }
    })
}
