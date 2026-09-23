//! vtype desktop. Chrome's speech recognition, in a Chrome that vtype starts itself, turns speech
//! into text; this program types it into whatever app is in front.

mod about;
mod beside_field;
mod chrome_launch;
mod cli;
mod config;
mod daemon;
mod diag;
mod hotkey;
mod i18n;
mod icon_draw;
mod install;
mod ipc;
#[cfg(any(target_os = "macos", test))]
mod launch_agent;
#[cfg(any(target_os = "linux", test))]
mod linux_setup;
mod log;
mod menu;
mod overlay_logic;
mod paths;
mod platform;
mod protocol;
mod report;
mod ripple;
mod speech_assets;
mod speech_host;
#[cfg(any(target_os = "macos", test))]
mod text_chunks;
#[cfg(any(target_os = "linux", test))]
mod wayland_inject;
#[cfg(windows)]
mod win_registry;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = if args.len() == 1 && std::env::current_exe().is_ok_and(|exe| install::is_msix_install(&exe)) {
        // The Store package starts vtype without arguments (its StartupTask, its Start menu tile).
        daemon::run()
    } else {
        let cli = Cli::parse();
        match cli.command {
            Command::Daemon => daemon::run(),
            Command::Install => install::install(),
            Command::Uninstall => install::uninstall(),
            other => {
                crate::log::init_stderr();
                cli::run_client(other)
            }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vtype: {e:#}");
            tracing::error!(error = %format!("{e:#}"), "exit");
            ExitCode::FAILURE
        }
    }
}
