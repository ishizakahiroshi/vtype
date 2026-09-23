//! The Store version starts with Windows through its package's StartupTask
//! (packaging/msix/AppxManifest.xml), not the Run key (plan C12). The app may switch it on and
//! off, except after the user switched it off in Windows (Task Manager, Settings > Apps >
//! Startup) or a policy decided: then only Windows can ("this method will not override their
//! choice and the user must re-enable the task manually", Microsoft Learn, StartupTask).

use windows::core::HSTRING;
use windows::ApplicationModel::{StartupTask, StartupTaskState};

use crate::platform::{Autostart, PlatformError};

/// The `TaskId` in AppxManifest.xml.
const TASK_ID: &str = "vtypeDaemon";

fn task() -> Result<StartupTask, PlatformError> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID)).and_then(|op| op.join()).map_err(PlatformError::failed)
}

/// What a task's state means for the settings page.
fn from_state(state: StartupTaskState) -> Autostart {
    match state {
        StartupTaskState::Enabled => Autostart { enabled: true, can_change: true },
        StartupTaskState::Disabled => Autostart { enabled: false, can_change: true },
        StartupTaskState::EnabledByPolicy => Autostart { enabled: true, can_change: false },
        // DisabledByUser, DisabledByPolicy, and whatever Windows adds later.
        _ => Autostart { enabled: false, can_change: false },
    }
}

pub fn autostart() -> Result<Autostart, PlatformError> {
    task()?.State().map(from_state).map_err(PlatformError::failed)
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let task = task()?;
    if !enabled {
        return task.Disable().map_err(PlatformError::failed);
    }
    // A packaged desktop app gets no consent dialog: the answer is the new state.
    let state = task.RequestEnableAsync().and_then(|op| op.join()).map_err(PlatformError::failed)?;
    if from_state(state).enabled {
        Ok(())
    } else {
        Err(PlatformError::Failed(format!("Windows keeps it off ({state:?})")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_apps_own_choices_can_be_changed_here() {
        assert_eq!(from_state(StartupTaskState::Enabled), Autostart { enabled: true, can_change: true });
        assert_eq!(from_state(StartupTaskState::Disabled), Autostart { enabled: false, can_change: true });
        for locked in [StartupTaskState::DisabledByUser, StartupTaskState::DisabledByPolicy] {
            assert_eq!(from_state(locked), Autostart { enabled: false, can_change: false });
        }
        assert_eq!(from_state(StartupTaskState::EnabledByPolicy), Autostart { enabled: true, can_change: false });
    }

    #[test]
    fn asks_for_the_task_the_package_declares() {
        let manifest = include_str!("../../../packaging/msix/AppxManifest.xml");
        assert!(manifest.contains(&format!("TaskId=\"{TASK_ID}\"")));
    }
}
