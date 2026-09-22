//! "Report a problem": the GitHub issue form, opened with what we know filled in. Nothing is sent
//! anywhere; the user reads the form and submits it (parent plan D23, D24).
//!
//! The extension builds the same URL in `packages/extension/src/shared/report.ts`; both test
//! against `tests/fixtures/report-url.json`, so they cannot drift apart.

pub const ISSUE_FORM: &str = "https://github.com/ishizakahiroshi/vtype/issues/new";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Extension,
    Desktop,
}

impl Surface {
    /// The option label in `.github/ISSUE_TEMPLATE/bug_report.yml` (field `surface`).
    pub fn option_label(self) -> &'static str {
        match self {
            Surface::Extension => "Chrome extension / Chrome 拡張",
            Surface::Desktop => "Desktop app / デスクトップ版",
        }
    }
}

pub struct ReportInfo<'a> {
    pub surface: Surface,
    pub version: &'a str,
    pub os: &'a str,
    pub browser: &'a str,
}

pub fn bug_report_url(info: &ReportInfo<'_>) -> String {
    format!(
        "{ISSUE_FORM}?template=bug_report.yml&surface={}&version={}&os={}&browser={}",
        encode_uri_component(info.surface.option_label()),
        encode_uri_component(info.version),
        encode_uri_component(info.os),
        encode_uri_component(info.browser),
    )
}

/// JavaScript's `encodeURIComponent`: keeps `A-Z a-z 0-9 - _ . ! ~ * ' ( )`, percent-encodes the
/// UTF-8 bytes of everything else with upper-case hex.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_javascript() {
        assert_eq!(encode_uri_component("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(encode_uri_component("-_.!~*'()"), "-_.!~*'()");
    }
}
