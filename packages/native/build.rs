// Takes the desktop app's strings from the extension's dictionaries, which stay the one place a
// user-facing string lives (parent plan D22). Only keys that start with `native_` are taken.
//
// The build fails when en and ja do not have the same `native_` keys, or when the Rust source
// asks for a key (`t("native_…")` / `t_with("native_…"`) that the dictionaries do not have.
// A typo in a key would otherwise show the key itself to the user.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const LOCALES: [&str; 2] = ["en", "ja"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let locales_dir = manifest_dir.join("../extension/_locales");
    println!("cargo:rerun-if-changed=src");

    let mut tables: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for code in LOCALES {
        let path = locales_dir.join(code).join("messages.json");
        println!("cargo:rerun-if-changed={}", path.display());
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let json: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()));
        let mut table = BTreeMap::new();
        for (key, entry) in json.as_object().expect("messages.json must be an object") {
            if !key.starts_with("native_") {
                continue;
            }
            let message =
                entry.get("message").and_then(|m| m.as_str()).unwrap_or_else(|| panic!("{code}/{key} has no message"));
            table.insert(key.clone(), message.to_string());
        }
        tables.insert(code, table);
    }

    let en: Vec<&String> = tables["en"].keys().collect();
    for code in LOCALES {
        let keys: Vec<&String> = tables[code].keys().collect();
        if keys != en {
            let missing: Vec<_> = en.iter().filter(|k| !tables[code].contains_key(k.as_str())).collect();
            let extra: Vec<_> = keys.iter().filter(|k| !tables["en"].contains_key(k.as_str())).collect();
            panic!("_locales/{code} does not have the same native_ keys as en (missing {missing:?}, extra {extra:?})");
        }
    }

    let mut used = Vec::new();
    collect_used_keys(&manifest_dir.join("src"), &mut used);
    for key in &used {
        for code in LOCALES {
            if !tables[code].contains_key(key) {
                panic!("the source uses {key}, which _locales/{code}/messages.json does not have");
            }
        }
    }

    let mut out = String::new();
    for code in LOCALES {
        out.push_str(&format!("pub static {}: &[(&str, &str)] = &[\n", code.to_uppercase()));
        for (key, message) in &tables[code] {
            out.push_str(&format!("    ({key:?}, {message:?}),\n"));
        }
        out.push_str("];\n");
    }
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("native_messages.rs");
    fs::write(dest, out).unwrap();
}

fn collect_used_keys(dir: &Path, used: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_used_keys(&path, used);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            for needle in ["t(\"", "t_with(\""] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(needle) {
                    let before = rest[..i].chars().last();
                    rest = &rest[i + needle.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let end = rest.find('"').unwrap_or(0);
                    let key = &rest[..end];
                    // Only real keys; a doc comment's `t("native_…")` is not one.
                    let is_key =
                        key.len() > "native_".len() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if key.starts_with("native_") && is_key && !used.iter().any(|k| k == key) {
                        used.push(key.to_string());
                    }
                }
            }
        }
    }
}
