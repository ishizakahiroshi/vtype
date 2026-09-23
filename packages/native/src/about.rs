//! "About vtype": where the settings page's links go. The page asks for a link by its name and the
//! daemon opens it in the usual browser (the settings window is a Chrome profile of its own), so
//! the page can have nothing else opened.
//!
//! The extension's options page shows the same links (`packages/extension/src/shared/about.ts`);
//! both test against `tests/fixtures/about-links.json`, so they cannot drift apart.

use serde::{Deserialize, Serialize};

const REPO: &str = "https://github.com/ishizakahiroshi/vtype";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AboutLink {
    Privacy,
    Homepage,
    Source,
    License,
    /// The licences of the crates inside, attached to the release of `version`.
    Notices,
}

impl AboutLink {
    pub fn url(self, version: &str) -> String {
        match self {
            AboutLink::Privacy => format!("{REPO}/blob/main/PRIVACY.md"),
            AboutLink::Homepage => "https://ishizakahiroshi.com/".to_string(),
            AboutLink::Source => REPO.to_string(),
            AboutLink::License => format!("{REPO}/blob/main/LICENSE"),
            AboutLink::Notices => format!("{REPO}/releases/download/native-v{version}/THIRD_PARTY_NOTICES.txt"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared with packages/extension/tests/about.test.ts.
    #[test]
    fn the_same_links_as_the_extension() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/about-links.json")).unwrap();
        let version = fixture["version"].as_str().unwrap();
        let links = fixture["links"].as_object().unwrap();
        assert_eq!(links.len(), 5);
        for (name, url) in links {
            let link: AboutLink = serde_json::from_value(serde_json::Value::String(name.clone())).unwrap();
            assert_eq!(link.url(version), url.as_str().unwrap(), "{name}");
        }
    }
}
