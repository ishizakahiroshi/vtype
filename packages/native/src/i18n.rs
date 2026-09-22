//! User-facing strings. The dictionaries are the extension's `_locales/*/messages.json` (keys that
//! start with `native_`), compiled in by build.rs. Call `t("native_…")` with a literal key so that
//! build.rs and the extension's i18n test can see which keys are used.

use std::sync::OnceLock;

mod tables {
    include!(concat!(env!("OUT_DIR"), "/native_messages.rs"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ja,
}

static LANG: OnceLock<Lang> = OnceLock::new();

/// `ja…` reads Japanese, everything else English.
pub fn lang_for(ui_language: &str) -> Lang {
    if ui_language.trim().to_ascii_lowercase().starts_with("ja") {
        Lang::Ja
    } else {
        Lang::En
    }
}

/// Sets the language once, from the OS UI language. Later calls are ignored.
pub fn init(ui_language: &str) {
    let _ = LANG.set(lang_for(ui_language));
}

fn table(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::En => tables::EN,
        Lang::Ja => tables::JA,
    }
}

pub fn lookup(lang: Lang, key: &str) -> String {
    table(lang)
        .iter()
        .chain(tables::EN.iter())
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| key.to_string())
}

pub fn t(key: &str) -> String {
    lookup(*LANG.get().unwrap_or(&Lang::En), key)
}

/// `t` with `{name}` placeholders filled in.
pub fn t_with(key: &str, vars: &[(&str, &str)]) -> String {
    let mut text = t(key);
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_japanese_only_for_ja() {
        assert_eq!(lang_for("ja-JP"), Lang::Ja);
        assert_eq!(lang_for("ja"), Lang::Ja);
        assert_eq!(lang_for("en-US"), Lang::En);
        assert_eq!(lang_for("fr"), Lang::En);
        assert_eq!(lang_for(""), Lang::En);
    }

    #[test]
    fn both_tables_have_the_same_keys() {
        let mut en: Vec<_> = tables::EN.iter().map(|(k, _)| *k).collect();
        let mut ja: Vec<_> = tables::JA.iter().map(|(k, _)| *k).collect();
        en.sort();
        ja.sort();
        assert!(!en.is_empty());
        assert_eq!(en, ja);
    }

    #[test]
    fn looks_up_each_language_and_falls_back_to_the_key() {
        assert_eq!(lookup(Lang::En, "native_trayQuit"), "Quit vtype");
        assert_eq!(lookup(Lang::Ja, "native_trayQuit"), "vtype を終了");
        assert_eq!(lookup(Lang::Ja, "native_noSuchKey"), "native_noSuchKey");
    }

    #[test]
    fn fills_placeholders() {
        assert!(
            t_with("native_notifyHotkeyFailed", &[("hotkey", "Ctrl+Alt+Space")])
                .contains("Ctrl+Alt+Space")
        );
    }
}
