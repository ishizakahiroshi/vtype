//! Starting with the user's session on macOS: a LaunchAgent plist in `~/Library/LaunchAgents`
//! (child plan C6-C1). Plain text in, plain text out, so it is tested on every OS.
// On other systems this is built only for its tests.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

pub const LABEL: &str = "com.ishizakahiroshi.vtype";

pub fn plist_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{LABEL}.plist"))
}

/// Runs `<exe> daemon` once at login and does not restart it (Quit in the menu means quit).
pub fn plist(exe: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#
    )
}

/// The plist, rewritten to start `exe` when it starts another copy of vtype (moved, or another
/// build); `None` when it starts `exe` already.
pub fn repointed_plist(text: &str, exe: &Path) -> Option<String> {
    let wanted = plist(exe);
    (text != wanted).then_some(wanted)
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_the_daemon_at_login_without_keeping_it_alive() {
        let text = plist(Path::new("/opt/vtype test/bin/vtype"));
        assert!(text.contains("<string>com.ishizakahiroshi.vtype</string>"));
        assert!(text.contains("<string>/opt/vtype test/bin/vtype</string>\n    <string>daemon</string>"));
        assert!(text.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(text.contains("<key>KeepAlive</key>\n  <false/>"));
    }

    #[test]
    fn escapes_the_path() {
        assert!(plist(Path::new("/opt/a&b<c>/vtype")).contains("<string>/opt/a&amp;b&lt;c&gt;/vtype</string>"));
    }

    #[test]
    fn a_plist_for_another_copy_is_pointed_here() {
        let here = Path::new("/Applications/vtype/vtype");
        let old = plist(Path::new("/Volumes/Old/vtype/vtype"));
        assert_eq!(repointed_plist(&old, here), Some(plist(here)));
        assert_eq!(repointed_plist(&plist(here), here), None);
    }

    #[test]
    fn lives_in_the_users_launch_agents() {
        let path = plist_path(Path::new("/var/lib/vtype-test"));
        assert!(path.ends_with("Library/LaunchAgents/com.ishizakahiroshi.vtype.plist"));
    }
}
