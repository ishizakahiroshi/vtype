//! Command line. One binary plays every role (parent plan D5); the subcommand picks it.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::ipc;
use crate::protocol::{InputMode, Reply, Request};

#[derive(Parser, Debug)]
#[command(name = "vtype", version, about = "vtype desktop: voice input into any app")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Run the resident app (tray, shortcut, mic icon).
    Daemon,
    /// Start dictation, or stop it if it is running.
    Toggle {
        #[arg(long, value_enum)]
        mode: Option<InputMode>,
    },
    /// Start dictation.
    Start {
        #[arg(long, value_enum)]
        mode: Option<InputMode>,
    },
    /// Stop dictation.
    Stop,
    /// Switch the input mode (it stays until changed).
    Mode {
        #[arg(value_enum)]
        mode: InputMode,
    },
    /// Show whether the speech page is connected and dictation is running.
    Status,
    /// Open the settings page in Chrome.
    Settings,
    /// End the resident app.
    Quit,
    /// Start with the OS (and, on GNOME, register the global shortcut).
    Install,
    /// Undo `install`.
    Uninstall,
    /// Print diagnostic information (no transcripts).
    Diag,
    /// (Checks on a real desktop, Windows) type TEXT into whatever has the focus.
    #[command(hide = true)]
    DebugType {
        text: String,
        #[arg(long, default_value = "type")]
        method: String,
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
        /// Type only if the window in front has exactly this title (so a test never types elsewhere).
        #[arg(long)]
        expect_foreground: String,
    },
    /// (Checks on a real desktop, Windows) what UI Automation says about the focused field.
    #[command(hide = true)]
    DebugField {
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
        /// Look at this window instead of the focused element.
        #[arg(long)]
        hwnd: Option<isize>,
    },
    /// (Checks on a real desktop, Windows) click the floating mic and report the foreground window.
    #[command(hide = true)]
    DebugClickIcon,
}

fn endpoint() -> String {
    crate::paths::ipc_endpoint()
}

/// Sends a request, starting the daemon first if `autostart` and none answers.
fn send(req: Request, autostart: bool) -> Result<Reply> {
    let endpoint = endpoint();
    if autostart {
        ipc::ensure_daemon(&endpoint)?;
    }
    match ipc::request_at(&endpoint, &req) {
        Ok(reply) => Ok(reply),
        Err(_) if !autostart => bail!("vtype is not running"),
        Err(e) => Err(e.into()),
    }
}

fn expect_ok(reply: Reply) -> Result<()> {
    match reply {
        Reply::Ok => Ok(()),
        Reply::Error { code, message } => bail!("{message} ({code})"),
        other => bail!("unexpected reply: {other:?}"),
    }
}

pub fn run_client(command: Command) -> Result<()> {
    match command {
        Command::Toggle { mode } => expect_ok(send(Request::Toggle { mode }, true)?),
        Command::Start { mode } => expect_ok(send(Request::Start { mode }, true)?),
        Command::Stop => expect_ok(send(Request::Stop, false)?),
        Command::Mode { mode } => expect_ok(send(Request::SetMode { mode }, true)?),
        Command::Settings => expect_ok(send(Request::OpenSettings, true)?),
        Command::Quit => match send(Request::Quit, false) {
            Ok(reply) => expect_ok(reply),
            Err(_) => {
                println!("vtype is not running");
                Ok(())
            }
        },
        Command::Status => {
            match send(Request::Status, false) {
                Ok(Reply::Status { connected, recording, mode, version }) => {
                    println!("running: true");
                    println!("connected: {connected}");
                    println!("recording: {recording}");
                    println!("mode: {}", mode.as_str());
                    println!("version: {version}");
                }
                Ok(other) => bail!("unexpected reply: {other:?}"),
                Err(_) => {
                    println!("running: false");
                    println!("version: {}", env!("CARGO_PKG_VERSION"));
                }
            }
            Ok(())
        }
        Command::Diag => {
            let report = match send(Request::Diagnostics, false) {
                Ok(Reply::Diagnostics { report }) => report,
                _ => offline_diagnostics(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Command::DebugType { text, method, delay_ms, expect_foreground } => {
            let method = match method.as_str() {
                "paste" => crate::config::InjectMethod::Paste,
                "auto" => crate::config::InjectMethod::Auto,
                _ => crate::config::InjectMethod::Type,
            };
            println!("{}", debug::debug_type(&text, method, delay_ms, &expect_foreground));
            Ok(())
        }
        Command::DebugField { delay_ms, hwnd } => {
            println!("{}", debug::debug_field(delay_ms, hwnd));
            Ok(())
        }
        Command::DebugClickIcon => {
            println!("{}", debug::debug_click_icon());
            Ok(())
        }
        Command::Daemon | Command::Install | Command::Uninstall => {
            unreachable!("handled in main")
        }
    }
}

#[cfg(windows)]
use crate::platform::windows::debug;

/// The desktop checks exist on Windows only (the development machine).
#[cfg(not(windows))]
mod debug {
    const ONLY: &str = "this check exists on Windows only";
    pub fn debug_type(_: &str, _: crate::config::InjectMethod, _: u64, _: &str) -> String {
        ONLY.into()
    }
    pub fn debug_field(_: u64, _: Option<isize>) -> String {
        ONLY.into()
    }
    pub fn debug_click_icon() -> String {
        ONLY.into()
    }
}

/// What `vtype diag` can say without a running daemon.
fn offline_diagnostics() -> serde_json::Value {
    let platform = crate::platform::current();
    let (config, _) = crate::config::load(&crate::paths::config_file());
    let errors = crate::diag::ErrorLog::default();
    let mut report = crate::diag::report(&crate::diag::Snapshot {
        os: &platform.os_description(),
        config: &config,
        connected: false,
        extension_version: None,
        browser: None,
        errors: &errors,
        notes: &[],
    });
    report["running"] = serde_json::Value::Bool(false);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_subcommands() {
        let parse = |list: &[&str]| Cli::try_parse_from(list).map(|c| c.command);
        assert_eq!(parse(&["vtype", "toggle"]).unwrap(), Command::Toggle { mode: None });
        assert_eq!(
            parse(&["vtype", "start", "--mode", "kana"]).unwrap(),
            Command::Start { mode: Some(InputMode::Kana) }
        );
        assert_eq!(parse(&["vtype", "mode", "en"]).unwrap(), Command::Mode { mode: InputMode::En });
        assert_eq!(parse(&["vtype", "install"]).unwrap(), Command::Install);
        // The Native Messaging host is gone (standalone plan C4): Chrome no longer starts vtype.
        assert!(parse(&["vtype", "host"]).is_err());
        assert!(parse(&["vtype", "install", "--extension-id", "a"]).is_err());
        assert!(parse(&["vtype", "mode", "katakana"]).is_err());
    }
}
