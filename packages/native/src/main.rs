//! vtype desktop. Chrome's speech recognition, in a Chrome that vtype starts itself, turns speech
//! into text; this program types it into whatever app is in front.

mod about;
mod beside_field;
mod chrome_launch;
mod cli;
mod config;
mod daemon;
mod diag;
mod field_check;
mod hotkey;
mod i18n;
mod icon_draw;
mod install;
mod ipc;
mod kept_bubble;
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
    let cli = Cli::parse();
    // Double-clicking the exe from the zip / tar.gz, and the Store package's StartupTask and Start
    // menu tile, start vtype without arguments.
    let command = cli::command_or_default(cli.command, || ipc::daemon_answers(&paths::ipc_endpoint()));
    let result = match command {
        Command::Daemon => daemon::run(),
        Command::Install => install::install(),
        Command::Uninstall => install::uninstall(),
        other => {
            crate::log::init_stderr();
            cli::run_client(other)
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
