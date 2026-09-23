//! The tray menu and the floating mic's templates list as data. Every OS builds its own UI from
//! these, so the items, their order and their strings are the same everywhere.

use crate::i18n::{t, t_with};
use crate::platform::{MenuAction, TrayState};
use crate::protocol::InputMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Check { action: MenuAction, label: String, checked: bool },
    Radio { action: MenuAction, label: String, checked: bool },
    Action { action: MenuAction, label: String },
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
        MenuItem::Action { action: MenuAction::About, label: t("native_trayAbout") },
        MenuItem::Separator,
        MenuItem::Action { action: MenuAction::Quit, label: t("native_trayQuit") },
    ]);
    items
}

/// How much of a template its row shows.
pub const TEMPLATE_LABEL_CHARS: usize = 30;

/// A template as a row: its first line, cut to `TEMPLATE_LABEL_CHARS`, with "…" when something
/// was left out. Every system draws the rows itself, so nothing is escaped.
pub fn template_label(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let cut: String = first.chars().take(TEMPLATE_LABEL_CHARS).collect();
    let more = first.chars().count() > TEMPLATE_LABEL_CHARS || text.trim().lines().count() > 1;
    if more {
        format!("{cut}…")
    } else {
        cut
    }
}

/// One row of the templates list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateRow {
    /// A template, `index` into the config's list. Its text puts it in; the edit and delete
    /// buttons at its right end show while the pointer is on the row.
    Template { index: usize, label: String },
    /// Where the template deleted last was: says so, and puts it back. `index` and `label` are
    /// what it had.
    Deleted { index: usize, label: String, message: String, undo: String },
}

fn deleted_row(index: usize, label: String) -> TemplateRow {
    let message = t_with("native_menuTemplateDeleted", &[("template", &label)]);
    TemplateRow::Deleted { index, label, message, undo: t("native_menuTemplateUndo") }
}

/// The floating mic's templates list (the same on every system; each draws it its own way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateList {
    pub rows: Vec<TemplateRow>,
    /// Below the rows: adding the selection, then the settings page.
    pub footer: Vec<(MenuAction, String)>,
    /// What the two buttons of a template row are called (tooltips, screen readers).
    pub edit_name: String,
    pub delete_name: String,
}

/// The templates, with the one `deleted` last (its old index and text) shown where it was until
/// the next deletion or until it is put back.
pub fn template_list(templates: &[String], deleted: Option<(usize, &str)>) -> TemplateList {
    let mut rows: Vec<TemplateRow> = templates
        .iter()
        .enumerate()
        .map(|(index, text)| TemplateRow::Template { index, label: template_label(text) })
        .collect();
    if let Some((index, text)) = deleted {
        rows.insert(index.min(rows.len()), deleted_row(index, template_label(text)));
    }
    TemplateList {
        rows,
        footer: vec![
            (MenuAction::AddSelectionAsTemplate, t("native_menuAddSelection")),
            (MenuAction::OpenSettings, t("native_menuEditTemplates")),
        ],
        edit_name: t("native_menuTemplateEdit"),
        delete_name: t("native_menuTemplateDelete"),
    }
}

/// Where on a row the pointer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowPart {
    Text,
    Edit,
    Delete,
}

/// What a click on `part` of `row` does. A deleted row is one button: anywhere on it puts back.
pub fn row_action(row: &TemplateRow, part: RowPart) -> MenuAction {
    match (row, part) {
        (TemplateRow::Template { index, .. }, RowPart::Text) => MenuAction::InsertTemplate(*index),
        (TemplateRow::Template { index, .. }, RowPart::Edit) => MenuAction::EditTemplate(*index),
        (TemplateRow::Template { index, .. }, RowPart::Delete) => MenuAction::DeleteTemplate(*index),
        (TemplateRow::Deleted { .. }, _) => MenuAction::UndoDeleteTemplate,
    }
}

/// Deleting and putting back keep the list open; everything else closes it.
pub fn keeps_list_open(action: MenuAction) -> bool {
    matches!(action, MenuAction::DeleteTemplate(_) | MenuAction::UndoDeleteTemplate)
}

/// The list as the daemon will show it once `action` is done, for a system whose open menu cannot
/// wait for the daemon (macOS runs no other main-thread work while its menu tracks). The daemon's
/// own list (`template_list`) replaces it as soon as it arrives. (Windows reopens its menu with the
/// daemon's list instead.)
#[cfg_attr(windows, allow(dead_code))]
pub fn list_after(list: &TemplateList, action: MenuAction) -> TemplateList {
    let mut next = list.clone();
    match action {
        MenuAction::DeleteTemplate(gone) => {
            let Some(label) = list.rows.iter().find_map(|row| match row {
                TemplateRow::Template { index, label } if *index == gone => Some(label.clone()),
                _ => None,
            }) else {
                return next;
            };
            // Only one deleted row: the one before is forgotten.
            next.rows = list
                .rows
                .iter()
                .filter_map(|row| match row {
                    TemplateRow::Template { index, label } if *index != gone => Some(TemplateRow::Template {
                        index: if *index > gone { index - 1 } else { *index },
                        label: label.clone(),
                    }),
                    _ => None,
                })
                .collect();
            next.rows.insert(gone.min(next.rows.len()), deleted_row(gone, label));
        }
        MenuAction::UndoDeleteTemplate => {
            let Some((back, label)) = list.rows.iter().find_map(|row| match row {
                TemplateRow::Deleted { index, label, .. } => Some((*index, label.clone())),
                _ => None,
            }) else {
                return next;
            };
            let mut rows: Vec<TemplateRow> = list
                .rows
                .iter()
                .filter_map(|row| match row {
                    TemplateRow::Template { index, label } => Some(TemplateRow::Template {
                        index: if *index >= back { index + 1 } else { *index },
                        label: label.clone(),
                    }),
                    TemplateRow::Deleted { .. } => None,
                })
                .collect();
            let at = back.min(rows.len());
            rows.insert(at, TemplateRow::Template { index: at, label });
            next.rows = rows;
        }
        _ => {}
    }
    next
}

/// The two buttons of a template row, in the row's own units (pixels or points), measured from
/// its right end: each is `ROW_BUTTON` wide with `ROW_BUTTON_GAP` between, `ROW_PAD_RIGHT` from
/// the end. Only the systems that draw the rows themselves (Windows, macOS) place the buttons by
/// these; GTK lays its row out and asks the widgets where they are.
pub const ROW_BUTTON: f64 = 24.0;
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const ROW_BUTTON_GAP: f64 = 2.0;
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const ROW_PAD_RIGHT: f64 = 6.0;
/// Room the buttons take at the right end of a row, with a gap before the text.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const ROW_BUTTONS_WIDTH: f64 = ROW_PAD_RIGHT + 2.0 * ROW_BUTTON + ROW_BUTTON_GAP + 8.0;

/// The left and right edge of the edit and the delete button in a row `width` wide, with
/// `scale` units per point.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn row_buttons(width: f64, scale: f64) -> [(RowPart, f64, f64); 2] {
    let right = width - ROW_PAD_RIGHT * scale;
    let delete = (right - ROW_BUTTON * scale, right);
    let edit_right = delete.0 - ROW_BUTTON_GAP * scale;
    [(RowPart::Edit, edit_right - ROW_BUTTON * scale, edit_right), (RowPart::Delete, delete.0, delete.1)]
}

/// Which part of a template row `x` (from the row's left edge) is on.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn row_part(x: f64, width: f64, scale: f64) -> RowPart {
    row_buttons(width, scale)
        .into_iter()
        .find(|&(_, left, right)| x >= left && x < right)
        .map_or(RowPart::Text, |(part, _, _)| part)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_templates_list_has_a_row_each_then_adding_and_the_settings_page() {
        let templates = ["お世話になっております。".to_string(), "A & B\nsecond line".to_string()];
        let list = template_list(&templates, None);
        assert_eq!(
            list.rows,
            vec![
                TemplateRow::Template { index: 0, label: "お世話になっております。".into() },
                TemplateRow::Template { index: 1, label: "A & B…".into() },
            ]
        );
        let footer: Vec<MenuAction> = list.footer.iter().map(|(a, _)| *a).collect();
        assert_eq!(footer, vec![MenuAction::AddSelectionAsTemplate, MenuAction::OpenSettings]);
        // Nothing saved yet: only the footer.
        assert!(template_list(&[], None).rows.is_empty());
        let long = "x".repeat(40);
        assert_eq!(template_label(&long), format!("{}…", "x".repeat(TEMPLATE_LABEL_CHARS)));
        assert_eq!(template_label("  short  "), "short");
    }

    #[test]
    fn the_template_deleted_last_shows_where_it_was() {
        let templates = ["a".to_string(), "c".to_string()];
        let list = template_list(&templates, Some((1, "b")));
        assert!(matches!(&list.rows[1], TemplateRow::Deleted { message, .. } if message.contains('b')));
        assert!(matches!(&list.rows[2], TemplateRow::Template { index: 1, .. }));
        // Past the end when the list got shorter meanwhile; alone when it was the only one.
        assert!(matches!(template_list(&templates, Some((9, "z"))).rows[2], TemplateRow::Deleted { .. }));
        assert_eq!(template_list(&[], Some((0, "z"))).rows.len(), 1);
    }

    #[test]
    fn the_list_after_a_deletion_is_the_one_the_daemon_will_show() {
        let texts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let abc = template_list(&texts(&["a", "b", "c"]), None);
        // Deleting b: a, (b deleted), c — with c now at index 1.
        let without_b = list_after(&abc, MenuAction::DeleteTemplate(1));
        assert_eq!(without_b, template_list(&texts(&["a", "c"]), Some((1, "b"))));
        // Deleting another forgets the first deletion.
        let without_a = list_after(&without_b, MenuAction::DeleteTemplate(0));
        assert_eq!(without_a, template_list(&texts(&["c"]), Some((0, "a"))));
        // Putting back undoes it exactly.
        assert_eq!(list_after(&without_b, MenuAction::UndoDeleteTemplate), abc);
        assert_eq!(list_after(&without_a, MenuAction::UndoDeleteTemplate), template_list(&texts(&["a", "c"]), None));
        // The last one, and the only one.
        let without_c = list_after(&abc, MenuAction::DeleteTemplate(2));
        assert_eq!(without_c, template_list(&texts(&["a", "b"]), Some((2, "c"))));
        assert_eq!(list_after(&without_c, MenuAction::UndoDeleteTemplate), abc);
        let only = template_list(&texts(&["x"]), None);
        assert_eq!(list_after(&only, MenuAction::DeleteTemplate(0)), template_list(&[], Some((0, "x"))));
        // Nothing to delete or put back: unchanged.
        assert_eq!(list_after(&abc, MenuAction::DeleteTemplate(7)), abc);
        assert_eq!(list_after(&abc, MenuAction::UndoDeleteTemplate), abc);
        assert_eq!(list_after(&abc, MenuAction::InsertTemplate(0)), abc);
    }

    #[test]
    fn a_row_tells_the_text_from_its_two_buttons() {
        let row = TemplateRow::Template { index: 3, label: "x".into() };
        // A row 200 wide at scale 1: delete is 170..194, edit 144..168.
        assert_eq!(row_part(10.0, 200.0, 1.0), RowPart::Text);
        assert_eq!(row_part(150.0, 200.0, 1.0), RowPart::Edit);
        assert_eq!(row_part(180.0, 200.0, 1.0), RowPart::Delete);
        assert_eq!(row_part(169.0, 200.0, 1.0), RowPart::Text, "the gap between is text");
        assert_eq!(row_part(360.0, 400.0, 2.0), RowPart::Delete, "twice the size at scale 2");
        assert_eq!(row_action(&row, RowPart::Text), MenuAction::InsertTemplate(3));
        assert_eq!(row_action(&row, RowPart::Edit), MenuAction::EditTemplate(3));
        assert_eq!(row_action(&row, RowPart::Delete), MenuAction::DeleteTemplate(3));
        let deleted = TemplateRow::Deleted { index: 0, label: "l".into(), message: "m".into(), undo: "u".into() };
        assert_eq!(row_action(&deleted, RowPart::Text), MenuAction::UndoDeleteTemplate);
        assert!(keeps_list_open(MenuAction::DeleteTemplate(0)));
        assert!(keeps_list_open(MenuAction::UndoDeleteTemplate));
        assert!(!keeps_list_open(MenuAction::EditTemplate(0)));
        assert!(!keeps_list_open(MenuAction::InsertTemplate(0)));
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
        // "About vtype" closes the group above Quit.
        assert!(matches!(menu[menu.len() - 3], MenuItem::Action { action: MenuAction::About, .. }));
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
