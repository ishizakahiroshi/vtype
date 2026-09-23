//! The tray menu as data. Every OS builds its native menu from this list, so the items, their
//! order and their strings are the same everywhere.

use crate::i18n::{t, t_with};
use crate::platform::{MenuAction, TrayState};
use crate::protocol::InputMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Check { action: MenuAction, label: String, checked: bool },
    Radio { action: MenuAction, label: String, checked: bool },
    Action { action: MenuAction, label: String },
    Submenu { label: String, items: Vec<MenuItem> },
    Separator,
}

pub fn tooltip() -> String {
    t("native_trayTooltip")
}

/// Linux trays (AppIndicator) report no clicks, so there the menu itself starts and stops.
pub const MENU_STARTS_RECORDING: bool = cfg!(target_os = "linux");

pub fn tray_menu(state: &TrayState) -> Vec<MenuItem> {
    menu_items(state, MENU_STARTS_RECORDING)
}

pub fn menu_items(state: &TrayState, with_record_item: bool) -> Vec<MenuItem> {
    let mode = |m: InputMode, label: String| MenuItem::Radio {
        action: MenuAction::SetMode(m),
        label,
        checked: state.mode == m,
    };
    let mut items = Vec::new();
    if with_record_item {
        items.push(MenuItem::Action { action: MenuAction::ToggleRecording, label: t("native_trayToggleRecording") });
        items.push(MenuItem::Separator);
    }
    items.extend([
        MenuItem::Check {
            action: MenuAction::ToggleIconVisible,
            label: t("native_trayShowIcon"),
            checked: state.icon_visible,
        },
        MenuItem::Check {
            action: MenuAction::ToggleHideOnFullscreen,
            label: t("native_trayHideOnFullscreen"),
            checked: state.hide_on_fullscreen,
        },
        MenuItem::Separator,
        mode(InputMode::Normal, t("native_trayModeNormal")),
        mode(InputMode::En, t("native_trayModeEn")),
        mode(InputMode::Kana, t("native_trayModeKana")),
        MenuItem::Separator,
        MenuItem::Action { action: MenuAction::AddSelectionAsTemplate, label: t("native_menuAddSelection") },
        MenuItem::Action { action: MenuAction::OpenSettings, label: t("native_trayOpenSettings") },
        MenuItem::Action { action: MenuAction::ReportBug, label: t("native_trayReportBug") },
        MenuItem::Action { action: MenuAction::CopyDiagnostics, label: t("native_trayCopyDiagnostics") },
        MenuItem::Separator,
        MenuItem::Action { action: MenuAction::Quit, label: t("native_trayQuit") },
    ]);
    items
}

/// How much of a template its menu item shows.
pub const TEMPLATE_LABEL_CHARS: usize = 30;

/// A template as a menu item: its first line, cut to `TEMPLATE_LABEL_CHARS`, with "…" when
/// something was left out. `&` is doubled, or the menu would read it as a keyboard shortcut.
pub fn template_label(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let cut: String = first.chars().take(TEMPLATE_LABEL_CHARS).collect();
    let more = first.chars().count() > TEMPLATE_LABEL_CHARS || text.trim().lines().count() > 1;
    let label = if more { format!("{cut}…") } else { cut };
    label.replace('&', "&&")
}

/// The floating mic's templates menu: the templates, then adding the selection, editing or
/// deleting one of them (a submenu each, so that putting one in stays a single click), putting
/// back the one `deleted` last, and the settings page.
pub fn template_menu(templates: &[String], deleted: Option<&str>) -> Vec<MenuItem> {
    let each = |action: fn(usize) -> MenuAction| -> Vec<MenuItem> {
        templates
            .iter()
            .enumerate()
            .map(|(i, text)| MenuItem::Action { action: action(i), label: template_label(text) })
            .collect()
    };
    let mut items = each(MenuAction::InsertTemplate);
    if !items.is_empty() {
        items.push(MenuItem::Separator);
    }
    items.push(MenuItem::Action { action: MenuAction::AddSelectionAsTemplate, label: t("native_menuAddSelection") });
    if !templates.is_empty() {
        items.push(MenuItem::Submenu { label: t("native_menuTemplateEdit"), items: each(MenuAction::EditTemplate) });
        items
            .push(MenuItem::Submenu { label: t("native_menuTemplateDelete"), items: each(MenuAction::DeleteTemplate) });
    }
    if let Some(text) = deleted {
        // The label is already escaped for the menu.
        let label = t_with("native_menuTemplateUndoDelete", &[("template", &template_label(text))]);
        items.push(MenuItem::Action { action: MenuAction::UndoDeleteTemplate, label });
    }
    items.push(MenuItem::Action { action: MenuAction::OpenSettings, label: t("native_menuEditTemplates") });
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(items: &[MenuItem]) -> Vec<MenuAction> {
        items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { action, .. } => Some(*action),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_templates_menu_lists_them_then_adding_editing_and_deleting() {
        let templates = ["お世話になっております。".to_string(), "A & B\nsecond line".to_string()];
        let menu = template_menu(&templates, None);
        assert_eq!(
            actions(&menu),
            vec![
                MenuAction::InsertTemplate(0),
                MenuAction::InsertTemplate(1),
                MenuAction::AddSelectionAsTemplate,
                MenuAction::OpenSettings
            ]
        );
        assert!(matches!(&menu[1], MenuItem::Action { label, .. } if label == "A && B…"));
        let subs: Vec<_> = menu
            .iter()
            .filter_map(|i| match i {
                MenuItem::Submenu { items, .. } => Some(actions(items)),
                _ => None,
            })
            .collect();
        assert_eq!(
            subs,
            vec![
                vec![MenuAction::EditTemplate(0), MenuAction::EditTemplate(1)],
                vec![MenuAction::DeleteTemplate(0), MenuAction::DeleteTemplate(1)],
            ]
        );
        // Nothing saved yet: only adding and the settings page.
        assert_eq!(template_menu(&[], None).len(), 2);
        // The one deleted last can be put back, even when it was the only one.
        let undo = template_menu(&[], Some("A & B"));
        assert_eq!(
            actions(&undo),
            vec![MenuAction::AddSelectionAsTemplate, MenuAction::UndoDeleteTemplate, MenuAction::OpenSettings]
        );
        assert!(matches!(&undo[1], MenuItem::Action { label, .. } if label.contains("A && B")));
        let long = "x".repeat(40);
        assert_eq!(template_label(&long), format!("{}…", "x".repeat(TEMPLATE_LABEL_CHARS)));
        assert_eq!(template_label("  short  "), "short");
    }

    #[test]
    fn checks_the_current_mode_and_toggles() {
        let state =
            TrayState { mode: InputMode::Kana, icon_visible: true, hide_on_fullscreen: false, ..TrayState::default() };
        let menu = tray_menu(&state);
        let checked: Vec<MenuAction> = menu
            .iter()
            .filter_map(|item| match item {
                MenuItem::Check { action, checked: true, .. } | MenuItem::Radio { action, checked: true, .. } => {
                    Some(*action)
                }
                _ => None,
            })
            .collect();
        assert_eq!(checked, vec![MenuAction::ToggleIconVisible, MenuAction::SetMode(InputMode::Kana)]);
        assert!(matches!(menu.last(), Some(MenuItem::Action { action: MenuAction::Quit, .. })));
    }

    #[test]
    fn the_record_item_leads_only_where_the_tray_has_no_click() {
        let state = TrayState::default();
        let first = |items: Vec<MenuItem>| items.into_iter().next();
        assert!(matches!(
            first(menu_items(&state, true)),
            Some(MenuItem::Action { action: MenuAction::ToggleRecording, .. })
        ));
        assert!(matches!(
            first(menu_items(&state, false)),
            Some(MenuItem::Check { action: MenuAction::ToggleIconVisible, .. })
        ));
    }
}
