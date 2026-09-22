//! The local endpoint the daemon listens on: a named pipe on Windows, a Unix socket elsewhere.
//! Every message is one JSON object on one line.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use interprocess::local_socket::{prelude::*, GenericFilePath, Listener, ListenerOptions, Stream};
use serde::{de::DeserializeOwned, Serialize};

use crate::protocol::{Reply, Request};

/// How long the command line waits for a daemon it just started (child plan C3).
pub const DAEMON_START_WAIT: Duration = Duration::from_secs(3);

pub fn listen_at(endpoint: &str) -> io::Result<Listener> {
    let name = endpoint.to_fs_name::<GenericFilePath>()?;
    #[cfg(unix)]
    {
        if let Some(dir) = std::path::Path::new(endpoint).parent() {
            std::fs::create_dir_all(dir)?;
        }
        // The caller has already made sure no daemon answers on this socket, so a file that is
        // still there is a leftover from a crash and may be replaced.
        ListenerOptions::new()
            .name(name)
            .try_overwrite(true)
            .create_sync()
    }
    #[cfg(not(unix))]
    {
        ListenerOptions::new().name(name).create_sync()
    }
}

pub fn connect_at(endpoint: &str) -> io::Result<Stream> {
    Stream::connect(endpoint.to_fs_name::<GenericFilePath>()?)
}

/// Writes `value` as one line.
pub fn write_line<W: Write, T: Serialize>(w: &mut W, value: &T) -> io::Result<()> {
    let mut text = serde_json::to_vec(value).map_err(io::Error::other)?;
    text.push(b'\n');
    w.write_all(&text)?;
    w.flush()
}

/// Reads one line and parses it. `Ok(None)` at end of input.
pub fn read_line<R: BufRead, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Sends one request and waits for its reply.
pub fn request_at(endpoint: &str, req: &Request) -> io::Result<Reply> {
    let stream = connect_at(endpoint)?;
    let mut reader = BufReader::new(stream);
    write_line(reader.get_mut(), req)?;
    read_line(&mut reader)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "daemon closed the connection"))
}

/// True if a daemon answers `status` on the endpoint.
pub fn daemon_answers(endpoint: &str) -> bool {
    matches!(
        request_at(endpoint, &Request::Status),
        Ok(Reply::Status { .. })
    )
}

/// Starts `vtype daemon` detached from this process, unless one already answers, and waits up to
/// three seconds for it.
pub fn ensure_daemon(endpoint: &str) -> Result<()> {
    if daemon_answers(endpoint) {
        return Ok(());
    }
    spawn_daemon().context("could not start the vtype daemon")?;
    let deadline = Instant::now() + DAEMON_START_WAIT;
    while Instant::now() < deadline {
        if daemon_answers(endpoint) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "the vtype daemon did not answer within {} seconds",
        DAEMON_START_WAIT.as_secs()
    )
}

pub fn spawn_daemon() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    match cmd.spawn() {
        Ok(_) => Ok(()),
        #[cfg(windows)]
        Err(_) => {
            // The job this process runs in (Chrome's, for the host) may forbid breaking away.
            // Start it inside the job instead; it then lives as long as the job does.
            use std::os::windows::process::CommandExt;
            let mut cmd = Command::new(std::env::current_exe()?);
            cmd.arg("daemon")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
            cmd.spawn().map(|_| ())
        }
        #[cfg(not(windows))]
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB,
    );
}

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A new process group, so the terminal's Ctrl+C or Chrome closing the host does not reach it.
    cmd.process_group(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InputMode;

    pub(crate) fn test_endpoint(tag: &str) -> String {
        let unique = format!("vtype-test-{}-{}", tag, std::process::id());
        if cfg!(windows) {
            format!(r"\\.\pipe\{unique}")
        } else {
            std::env::temp_dir()
                .join(format!("{unique}.sock"))
                .to_string_lossy()
                .into_owned()
        }
    }

    #[test]
    fn a_status_request_round_trips_over_the_endpoint() {
        let endpoint = test_endpoint("status");
        let listener = listen_at(&endpoint).unwrap();
        let server = thread::spawn(move || {
            let conn = listener.incoming().next().unwrap().unwrap();
            let mut reader = BufReader::new(conn);
            let req: Request = read_line(&mut reader).unwrap().unwrap();
            assert_eq!(req, Request::Status);
            write_line(
                reader.get_mut(),
                &Reply::Status {
                    connected: false,
                    recording: false,
                    mode: InputMode::Normal,
                    version: "0.1.0".into(),
                },
            )
            .unwrap();
        });
        let reply = request_at(&endpoint, &Request::Status).unwrap();
        server.join().unwrap();
        assert_eq!(
            reply,
            Reply::Status {
                connected: false,
                recording: false,
                mode: InputMode::Normal,
                version: "0.1.0".into()
            }
        );
    }

    #[test]
    fn nobody_listening_is_an_error_not_a_hang() {
        let endpoint = test_endpoint("nobody");
        assert!(request_at(&endpoint, &Request::Status).is_err());
        assert!(!daemon_answers(&endpoint));
    }

    #[test]
    fn blank_lines_are_skipped() {
        let mut r = io::Cursor::new(b"\n\n{\"type\":\"stop\"}\n".to_vec());
        assert_eq!(
            read_line::<_, Request>(&mut r).unwrap(),
            Some(Request::Stop)
        );
        assert_eq!(read_line::<_, Request>(&mut r).unwrap(), None);
    }
}
