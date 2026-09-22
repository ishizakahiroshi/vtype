//! The Native Messaging host: the process Chrome starts for `connectNative`. It relays frames
//! between Chrome (stdin / stdout) and the daemon (IPC), starting the daemon if none is running.
//! It ends when Chrome closes stdin or when the daemon goes away.

use std::io::{self, BufReader};
use std::thread;

use anyhow::{Context, Result};
use interprocess::local_socket::prelude::*;
use serde_json::Value;

use crate::ipc;
use crate::nm_frame::{read_frame, write_frame};
use crate::protocol::{Reply, Request};

pub fn run(origin: String) -> Result<()> {
    crate::log::init_file("host");
    let endpoint = crate::paths::ipc_endpoint();
    ipc::ensure_daemon(&endpoint)?;
    let stream = ipc::connect_at(&endpoint).context("could not reach the vtype daemon")?;
    let (recv, mut send) = stream.split();
    ipc::write_line(&mut send, &Request::HostHello { origin })?;

    thread::spawn(move || {
        let mut reader = BufReader::new(recv);
        while let Ok(Some(reply)) = ipc::read_line::<_, Reply>(&mut reader) {
            if let Reply::ToExtension { message } = reply {
                let body = serde_json::to_vec(&message).unwrap_or_default();
                // Locked per frame, never across the wait for the daemon: exiting flushes stdout.
                if let Err(e) = write_frame(&mut io::stdout().lock(), &body) {
                    tracing::warn!(error = %e, "could not write to Chrome");
                    break;
                }
            }
        }
        tracing::info!("daemon connection closed");
        std::process::exit(0);
    });

    let mut stdin = io::stdin().lock();
    while let Some(frame) = read_frame(&mut stdin)? {
        match serde_json::from_slice::<Value>(&frame) {
            Ok(message) => ipc::write_line(&mut send, &Request::FromExtension { message })?,
            Err(e) => tracing::warn!(error = %e, "unreadable frame from Chrome"),
        }
    }
    tracing::info!("Chrome closed the port");
    // The relay thread is still waiting on the daemon; it must not keep this process alive.
    std::process::exit(0);
}
