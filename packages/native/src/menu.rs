//! The tray menu as data. Every OS builds its native menu from this list, so the items, their
//! order and their strings are the same everywhere.

use crate::i18n::t;
use crate::platform::{MenuAction, TrayState};
use crate::protocol::InputMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Check {
        action: MenuAction,
        label: String,
        checked: bool,
    },
    Radio {
        action: MenuAction,
        label: String,
        checked: bool,
    },
    Action {
        action: MenuAction,
        label: String,
    },
    Separator,
}

pub fn tooltip() -> String {
    t("native_trayTooltip")
}

pub fn tray_menu(state: &TrayState) -> Vec<MenuItem> {
    let mode = |m: InputMode, label: String| MenuItem::Radio {
        action: MenuAction::SetMode(m),
        label,
        checked: state.mode == m,
    };
    vec![
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
        MenuItem::Action {
            action: MenuAction::OpenSettings,
            label: t("native_trayOpenSettings"),
        },
        MenuItem::Action {
            action: MenuAction::ReportBug,
            label: t("native_trayReportBug"),
        },
        MenuItem::Action {
            action: MenuAction::CopyDiagnostics,
            label: t("native_trayCopyDiagnostics"),
        },
        MenuItem::Separator,
        MenuItem::Action {
            action: MenuAction::Quit,
            label: t("native_trayQuit"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_current_mode_and_toggles() {
        let state = TrayState {
            mode: InputMode::Kana,
            icon_visible: true,
            hide_on_fullscreen: false,
            ..TrayState::default()
        };
        let menu = tray_menu(&state);
        let checked: Vec<MenuAction> = menu
            .iter()
            .filter_map(|item| match item {
                MenuItem::Check {
                    action,
                    checked: true,
                    ..
                }
                | MenuItem::Radio {
                    action,
                    checked: true,
                    ..
                } => Some(*action),
                _ => None,
            })
            .collect();
        assert_eq!(
            checked,
            vec![
                MenuAction::ToggleIconVisible,
                MenuAction::SetMode(InputMode::Kana)
            ]
        );
        assert!(matches!(
            menu.last(),
            Some(MenuItem::Action {
                action: MenuAction::Quit,
                ..
            })
        ));
    }
}
