//! GNOME's custom shortcut for vtype (child plan C7-C1): on Wayland no app may grab keys, so
//! `vtype install` asks GNOME itself to run `vtype toggle` on Ctrl+Alt+Space. A shortcut already
//! named "vtype" (the user's own) is left alone. `vtype uninstall` takes ours out again.

use std::path::Path;
use std::process::Command;

use crate::linux_setup::{
    gvariant_string, gvariant_string_array, is_gnome, parse_gsettings_paths, toggle_command, with_vtype_shortcut,
    without_vtype_shortcut, CUSTOM_KEYBINDINGS_KEY, CUSTOM_KEYBINDING_SCHEMA, GNOME_BINDING, MEDIA_KEYS_SCHEMA,
    SHORTCUT_NAME, VTYPE_KEYBINDING_PATH,
};

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gsettings").args(args).output().map_err(|e| format!("gsettings: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn binding_schema(path: &str) -> String {
    format!("{CUSTOM_KEYBINDING_SCHEMA}:{path}")
}

fn current_paths() -> Result<Vec<String>, String> {
    gsettings(&["get", MEDIA_KEYS_SCHEMA, CUSTOM_KEYBINDINGS_KEY]).map(|s| parse_gsettings_paths(&s))
}

/// True when one of the user's own shortcuts (not ours) is already called "vtype".
fn name_taken(paths: &[String]) -> bool {
    paths.iter().filter(|p| *p != VTYPE_KEYBINDING_PATH).any(|p| {
        gsettings(&["get", &binding_schema(p), "name"])
            .map(|name| name.trim().trim_matches('\'') == SHORTCUT_NAME)
            .unwrap_or(false)
    })
}

pub fn desktop_is_gnome() -> bool {
    is_gnome(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref())
}

pub fn install(exe: &Path) {
    if !desktop_is_gnome() {
        return;
    }
    let result = (|| -> Result<&'static str, String> {
        let paths = current_paths()?;
        if name_taken(&paths) {
            return Ok("a shortcut named vtype already exists; left it as it is");
        }
        let schema = binding_schema(VTYPE_KEYBINDING_PATH);
        gsettings(&["set", &schema, "name", &gvariant_string(SHORTCUT_NAME)])?;
        gsettings(&["set", &schema, "command", &gvariant_string(&toggle_command(exe))])?;
        gsettings(&["set", &schema, "binding", &gvariant_string(GNOME_BINDING)])?;
        if let Some(list) = with_vtype_shortcut(&paths) {
            gsettings(&["set", MEDIA_KEYS_SCHEMA, CUSTOM_KEYBINDINGS_KEY, &gvariant_string_array(&list)])?;
        }
        Ok("added the GNOME shortcut Ctrl+Alt+Space")
    })();
    match result {
        Ok(msg) => println!("{msg}"),
        Err(e) => eprintln!("could not add the GNOME shortcut: {e}"),
    }
}

pub fn uninstall() {
    if !desktop_is_gnome() {
        return;
    }
    let result = (|| -> Result<bool, String> {
        let paths = current_paths()?;
        let Some(list) = without_vtype_shortcut(&paths) else { return Ok(false) };
        gsettings(&["set", MEDIA_KEYS_SCHEMA, CUSTOM_KEYBINDINGS_KEY, &gvariant_string_array(&list)])?;
        let schema = binding_schema(VTYPE_KEYBINDING_PATH);
        for key in ["name", "command", "binding"] {
            let _ = gsettings(&["reset", &schema, key]);
        }
        Ok(true)
    })();
    match result {
        Ok(true) => println!("removed the GNOME shortcut"),
        Ok(false) => {}
        Err(e) => eprintln!("could not remove the GNOME shortcut: {e}"),
    }
}
