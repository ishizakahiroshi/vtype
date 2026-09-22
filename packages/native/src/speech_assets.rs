//! The speech page (packages/extension/dist-desktop/), compiled in by build.rs so the daemon can
//! serve it without files next to the executable (standalone plan C3).

include!(concat!(env!("OUT_DIR"), "/speech_assets.rs"));

/// The file at `path` (relative, `/`-separated) with its Content-Type.
pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    SPEECH_ASSETS.iter().find(|(p, _, _)| *p == path).map(|(_, ty, bytes)| (*ty, *bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_and_the_dictionary_are_in() {
        let (ty, html) = asset("speech.html").unwrap();
        assert_eq!(ty, "text/html; charset=utf-8");
        assert!(std::str::from_utf8(html).unwrap().contains("speech.js"));
        assert_eq!(asset("speech.js").unwrap().0, "text/javascript; charset=utf-8");
        let (ty, dict) = asset("dict/base.dat.gz").unwrap();
        assert_eq!(ty, "application/octet-stream");
        assert!(!dict.is_empty());
        assert!(asset("dict/LICENSE-kuromoji.txt").is_some());
        assert_eq!(SPEECH_ASSETS.iter().filter(|(p, _, _)| p.ends_with(".dat.gz")).count(), 12);
    }

    #[test]
    fn unknown_paths_are_none() {
        assert!(asset("").is_none());
        assert!(asset("../speech.html").is_none());
        assert!(asset("/speech.html").is_none());
    }
}
