//! vtype desktop. The Chrome extension recognizes speech; this program receives the text over
//! Native Messaging and types it into whatever app is in front.

// The platform layer is filled in OS by OS (Windows, then macOS, then Linux); until all three
// use the whole trait, parts of it are unused on some targets.
#![allow(dead_code)]

mod cli;
mod config;
mod daemon;
mod diag;
mod host;
mod i18n;
mod install;
mod ipc;
mod log;
mod menu;
mod nm_frame;
mod paths;
mod platform;
mod protocol;
mod report;
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
