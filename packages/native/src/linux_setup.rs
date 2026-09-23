//! The Linux decisions that are plain text in, plain text out (child plan C7-C1), so they are
//! tested on every OS: which kind of session this is, the autostart entry, the GNOME custom
//! shortcut (the only way to get a global shortcut on GNOME's Wayland), and X11 keysyms.
// On other systems this is built only for its tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
    /// Neither (a console, a remote shell): no tray, no typing.
    Unknown,
}

/// `XDG_SESSION_TYPE` first; without it, which display variable is set.
pub fn detect_session(xdg_session_type: Option<&str>, wayland_display: Option<&str>, display: Option<&str>) -> Session {
    let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    match xdg_session_type.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("wayland") => Session::Wayland,
        Some("x11") => Session::X11,
        _ if set(wayland_display) => Session::Wayland,
        _ if set(display) => Session::X11,
        _ => Session::Unknown,
    }
}

pub fn current_session() -> Session {
    let var = |k: &str| std::env::var(k).ok();
    detect_session(var("XDG_SESSION_TYPE").as_deref(), var("WAYLAND_DISPLAY").as_deref(), var("DISPLAY").as_deref())
}

pub fn is_gnome(xdg_current_desktop: Option<&str>) -> bool {
    xdg_current_desktop.is_some_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")))
}

// --- autostart ------------------------------------------------------------------------------

pub fn autostart_path(config_dir: &Path) -> PathBuf {
    config_dir.join("autostart").join("vtype.desktop")
}

/// One argument of an `Exec=` line (Desktop Entry Specification): quoted when it has to be, with
/// `"` `` ` `` `$` `\` escaped inside the quotes, and `%` doubled.
pub fn desktop_exec_arg(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    if !arg.chars().any(reserved) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn autostart_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=vtype\nComment=Voice input into any app\nExec={} daemon\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_arg(&exe.to_string_lossy())
    )
}

/// The .deb's entry, for every user of the machine (Cargo.toml's `package.metadata.deb`).
pub const SYSTEM_AUTOSTART: &str = "/etc/xdg/autostart/vtype.desktop";

/// A user's entry that turns the system's off for them (the specification's `Hidden`).
pub fn autostart_off_entry() -> String {
    "[Desktop Entry]\nType=Application\nName=vtype\nHidden=true\n".to_string()
}

fn entry_is_off(entry: &str) -> bool {
    entry.lines().map(str::trim).any(|l| l == "Hidden=true" || l == "X-GNOME-Autostart-enabled=false")
}

/// Whether vtype starts at login: the user's entry wins over the system's of the same name.
pub fn autostart_on(user_entry: Option<&str>, system_entry: bool) -> bool {
    match user_entry {
        Some(entry) => !entry_is_off(entry),
        None => system_entry,
    }
}

/// What to write as the user's entry to switch starting at login, or `None` to remove it.
pub fn autostart_user_entry(enabled: bool, exe: &Path, system_entry: bool) -> Option<String> {
    match (enabled, system_entry) {
        (true, _) => Some(autostart_entry(exe)),
        (false, true) => Some(autostart_off_entry()),
        (false, false) => None,
    }
}

/// The user's entry, rewritten to start `exe` when it starts another copy of vtype (moved, or
/// another build); `None` when it starts `exe` already or is off.
pub fn repointed_entry(user_entry: &str, exe: &Path) -> Option<String> {
    let wanted = autostart_entry(exe);
    let exec = |entry: &str| entry.lines().find(|l| l.starts_with("Exec=")).map(str::to_string);
    let differs = exec(user_entry).is_some_and(|line| Some(line) != exec(&wanted));
    (!entry_is_off(user_entry) && differs).then_some(wanted)
}

// --- GNOME custom shortcut ------------------------------------------------------------------

pub const MEDIA_KEYS_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
pub const CUSTOM_KEYBINDINGS_KEY: &str = "custom-keybindings";
pub const CUSTOM_KEYBINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
pub const VTYPE_KEYBINDING_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/vtype/";
pub const SHORTCUT_NAME: &str = "vtype";
/// The same keys as the other systems' default (Ctrl+Alt+Space), in GTK's notation.
pub const GNOME_BINDING: &str = "<Control><Alt>space";

/// The paths in `gsettings get … custom-keybindings` output: `@as []` or `['/a/', '/b/']`.
pub fn parse_gsettings_paths(output: &str) -> Vec<String> {
    let s = output.trim();
    let s = s.strip_prefix("@as").unwrap_or(s).trim();
    let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    inner
        .split(',')
        .map(|p| p.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// A GVariant string literal (what `gsettings set` takes).
pub fn gvariant_string(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

pub fn gvariant_string_array(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    format!("[{}]", items.iter().map(|s| gvariant_string(s)).collect::<Vec<_>>().join(", "))
}

/// The list with vtype's shortcut added, or `None` when it is already there.
pub fn with_vtype_shortcut(paths: &[String]) -> Option<Vec<String>> {
    if paths.iter().any(|p| p == VTYPE_KEYBINDING_PATH) {
        return None;
    }
    let mut out = paths.to_vec();
    out.push(VTYPE_KEYBINDING_PATH.to_string());
    Some(out)
}

/// The list without vtype's shortcut, or `None` when it was not there.
pub fn without_vtype_shortcut(paths: &[String]) -> Option<Vec<String>> {
    if !paths.iter().any(|p| p == VTYPE_KEYBINDING_PATH) {
        return None;
    }
    Some(paths.iter().filter(|p| *p != VTYPE_KEYBINDING_PATH).cloned().collect())
}

/// The command GNOME runs for the shortcut (it splits it like a shell would).
pub fn toggle_command(exe: &Path) -> String {
    let exe = exe.to_string_lossy();
    if exe.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        format!("{exe} toggle")
    } else {
        format!("'{}' toggle", exe.replace('\'', r"'\''"))
    }
}

// --- X11 ------------------------------------------------------------------------------------

/// The keysym that types `c`: Latin-1 keysyms are the code point itself; everything else lives at
/// 0x0100_0000 + code point.
pub fn keysym_for(c: char) -> u32 {
    let cp = c as u32;
    if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) {
        cp
    } else {
        0x0100_0000 + cp
    }
}

/// The text's keysyms cut into groups that fit the spare keycodes: each group has at most `spare`
/// different keysyms, in order. One keyboard-mapping change per group instead of one per
/// character (apps pick a new mapping up lazily, so fewer changes mean fewer wrong characters).
pub fn keysym_batches(keysyms: &[u32], spare: usize) -> Vec<Vec<u32>> {
    let spare = spare.max(1);
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut current: Vec<u32> = Vec::new();
    let mut distinct: Vec<u32> = Vec::new();
    for &k in keysyms {
        if !distinct.contains(&k) {
            if distinct.len() == spare {
                out.push(std::mem::take(&mut current));
                distinct.clear();
            }
            distinct.push(k);
        }
        current.push(k);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub const XK_RETURN: u32 = 0xff0d;
pub const XK_TAB: u32 = 0xff09;
pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_V: u32 = 0x0076;
pub const XK_A: u32 = 0x0061;
pub const XK_C: u32 = 0x0063;
pub const XK_BACKSPACE: u32 = 0xff08;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_session_apart() {
        assert_eq!(detect_session(Some("wayland"), None, Some(":0")), Session::Wayland);
        assert_eq!(detect_session(Some("x11"), Some("wayland-0"), None), Session::X11);
        assert_eq!(detect_session(Some("tty"), Some("wayland-0"), None), Session::Wayland);
        assert_eq!(detect_session(None, None, Some(":1")), Session::X11);
        assert_eq!(detect_session(None, Some(" "), None), Session::Unknown);
        assert_eq!(detect_session(None, None, None), Session::Unknown);
    }

    #[test]
    fn finds_gnome_in_the_desktop_list() {
        assert!(is_gnome(Some("ubuntu:GNOME")));
        assert!(is_gnome(Some("GNOME")));
        assert!(!is_gnome(Some("KDE")));
        assert!(!is_gnome(None));
    }

    #[test]
    fn autostart_runs_the_daemon() {
        let entry = autostart_entry(Path::new("/opt/vtype/vtype"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=/opt/vtype/vtype daemon\n"));
        assert!(autostart_path(Path::new("/var/lib/vtype-test/.config")).ends_with("autostart/vtype.desktop"));
        assert_eq!(desktop_exec_arg("/opt/my apps/vtype"), "\"/opt/my apps/vtype\"");
        assert_eq!(desktop_exec_arg("/opt/a$b/100%"), "\"/opt/a\\$b/100%%\"");
    }

    #[test]
    fn the_users_entry_decides_over_the_systems() {
        let on = autostart_entry(Path::new("/opt/vtype/vtype"));
        let off = autostart_off_entry();
        assert!(autostart_on(None, true), "the .deb's entry");
        assert!(!autostart_on(None, false));
        assert!(autostart_on(Some(&on), false));
        assert!(!autostart_on(Some(&off), true), "turned off for this user");
        assert!(!autostart_on(Some("[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n"), true));
    }

    #[test]
    fn switching_writes_or_removes_the_users_entry() {
        let exe = Path::new("/opt/vtype/vtype");
        assert_eq!(autostart_user_entry(true, exe, false), Some(autostart_entry(exe)));
        assert_eq!(autostart_user_entry(true, exe, true), Some(autostart_entry(exe)));
        // Off: a system entry needs overriding; without one there is nothing to leave behind.
        assert_eq!(autostart_user_entry(false, exe, true), Some(autostart_off_entry()));
        assert_eq!(autostart_user_entry(false, exe, false), None);
        let off = autostart_user_entry(false, exe, true).unwrap();
        assert!(!autostart_on(Some(&off), true));
    }

    #[test]
    fn an_entry_for_another_copy_is_pointed_here() {
        let old = autostart_entry(Path::new("/opt/old/vtype"));
        let here = Path::new("/opt/vtype/vtype");
        assert_eq!(repointed_entry(&old, here), Some(autostart_entry(here)));
        assert_eq!(repointed_entry(&autostart_entry(here), here), None);
        // Off stays off.
        assert_eq!(repointed_entry(&autostart_off_entry(), here), None);
    }

    #[test]
    fn reads_and_writes_the_gsettings_list() {
        assert!(parse_gsettings_paths("@as []\n").is_empty());
        let paths = parse_gsettings_paths("['/org/x/custom0/', '/org/x/custom1/']\n");
        assert_eq!(paths, vec!["/org/x/custom0/".to_string(), "/org/x/custom1/".to_string()]);
        assert_eq!(gvariant_string_array(&[]), "@as []");
        assert_eq!(gvariant_string_array(&paths), "['/org/x/custom0/', '/org/x/custom1/']");
        assert_eq!(gvariant_string("it's"), r"'it\'s'");
    }

    #[test]
    fn adds_and_removes_only_our_shortcut() {
        let others = vec!["/org/x/custom0/".to_string()];
        let added = with_vtype_shortcut(&others).unwrap();
        assert_eq!(added, vec!["/org/x/custom0/".to_string(), VTYPE_KEYBINDING_PATH.to_string()]);
        assert_eq!(with_vtype_shortcut(&added), None);
        assert_eq!(without_vtype_shortcut(&added), Some(others.clone()));
        assert_eq!(without_vtype_shortcut(&others), None);
    }

    #[test]
    fn quotes_the_toggle_command_when_needed() {
        assert_eq!(toggle_command(Path::new("/usr/bin/vtype")), "/usr/bin/vtype toggle");
        assert_eq!(toggle_command(Path::new("/opt/my apps/vtype")), "'/opt/my apps/vtype' toggle");
    }

    #[test]
    fn keysyms_for_latin1_and_beyond() {
        assert_eq!(keysym_for('a'), 0x61);
        assert_eq!(keysym_for('é'), 0xe9);
        assert_eq!(keysym_for('あ'), 0x0100_3042);
        assert_eq!(keysym_for('🎤'), 0x0101_f3a4);
    }

    #[test]
    fn batches_fit_the_spare_keycodes() {
        // a b a c b d with 2 spare keycodes: [a b a] [c b] [d]
        assert_eq!(keysym_batches(&[1, 2, 1, 3, 2, 4], 2), vec![vec![1, 2, 1], vec![3, 2], vec![4]]);
        assert_eq!(keysym_batches(&[5, 5, 5], 1), vec![vec![5, 5, 5]]);
        assert!(keysym_batches(&[], 4).is_empty());
    }
}
