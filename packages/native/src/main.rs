//! vtype desktop. The Chrome extension recognizes speech; this program receives the text over
//! Native Messaging and types it into whatever app is in front.

mod beside_field;
mod cli;
mod config;
mod daemon;
mod diag;
mod host;
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
mod nm_frame;
mod overlay_logic;
mod paths;
mod platform;
mod protocol;
mod report;
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
    let result = if let Some(origin) = cli::host_origin(&args) {
        host::run(origin)
    } else {
        let cli = Cli::parse();
        match cli.command {
            Command::Daemon => daemon::run(),
            Command::Host { origin } => host::run(origin.unwrap_or_default()),
            Command::Install { extension_ids } => install::install(&extension_ids),
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
