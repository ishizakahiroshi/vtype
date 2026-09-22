//! Command line. One binary plays every role (parent plan D5); the subcommand picks it.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::ipc;
use crate::protocol::{InputMode, Reply, Request};

#[derive(Parser, Debug)]
#[command(
    name = "vtype",
    version,
    about = "vtype desktop: voice input into any app"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Run the resident app (tray, shortcut, mic icon).
    Daemon,
    /// Native Messaging host (Chrome starts this itself).
    Host {
        /// chrome-extension://<id>/ of the calling extension.
        origin: Option<String>,
    },
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
    /// Show whether the extension is connected and dictation is running.
    Status,
    /// Open the settings page in Chrome.
    Settings,
    /// End the resident app.
    Quit,
    /// Register the Native Messaging host with Chrome and start with the OS.
    Install {
        /// Also allow this extension ID (a development build). Repeatable.
        #[arg(long = "extension-id")]
        extension_ids: Vec<String>,
    },
    /// Undo `install`.
    Uninstall,
    /// Print diagnostic information (no transcripts).
    Diag,
}

/// Chrome starts the host with `chrome-extension://<id>/` as the first argument (and, on
/// Windows, `--parent-window=<n>` after it), not with a subcommand.
pub fn host_origin(args: &[String]) -> Option<String> {
    args.get(1)
        .filter(|a| a.starts_with("chrome-extension://"))
        .cloned()
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
                Ok(Reply::Status {
                    connected,
                    recording,
                    mode,
                    version,
                }) => {
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
        Command::Daemon | Command::Host { .. } | Command::Install { .. } | Command::Uninstall => {
            unreachable!("handled in main")
        }
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn chrome_style_arguments_mean_host() {
        assert_eq!(
            host_origin(&args(&[
                "vtype.exe",
                "chrome-extension://abc/",
                "--parent-window=123"
            ]))
            .as_deref(),
            Some("chrome-extension://abc/")
        );
        assert_eq!(host_origin(&args(&["vtype", "status"])), None);
        assert_eq!(host_origin(&args(&["vtype"])), None);
    }

    #[test]
    fn parses_the_subcommands() {
        let parse = |list: &[&str]| Cli::try_parse_from(list).map(|c| c.command);
        assert_eq!(
            parse(&["vtype", "toggle"]).unwrap(),
            Command::Toggle { mode: None }
        );
        assert_eq!(
            parse(&["vtype", "start", "--mode", "kana"]).unwrap(),
            Command::Start {
                mode: Some(InputMode::Kana)
            }
        );
        assert_eq!(
            parse(&["vtype", "mode", "en"]).unwrap(),
            Command::Mode {
                mode: InputMode::En
            }
        );
        assert_eq!(
            parse(&[
                "vtype",
                "install",
                "--extension-id",
                "a",
                "--extension-id",
                "b"
            ])
            .unwrap(),
            Command::Install {
                extension_ids: vec!["a".into(), "b".into()]
            }
        );
        assert!(parse(&["vtype", "mode", "katakana"]).is_err());
    }
}
