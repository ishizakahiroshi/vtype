//! The daemon's own speech page server (standalone plan C3): a small HTTP and WebSocket server on
//! 127.0.0.1 that serves the speech page compiled in by build.rs (speech_assets.rs) to the Chrome
//! the daemon starts, and relays the page's WebSocket to `Core`: `HostHello`, then
//! `FromExtension` for every message, and `Reply::ToExtension` back.
//!
//! Safety (parent plan S6):
//! - it listens on 127.0.0.1 only, never on every interface;
//! - every path carries a token made fresh at each start (`/t/<token>/…`). A wrong token, a missing
//!   one and a missing file all get the same 404, so the token cannot be guessed piece by piece;
//! - the WebSocket is accepted only from the page itself: `Origin` must be exactly
//!   `http://127.0.0.1:<port>`, so a web site open in some browser cannot connect.
//!
//! The port is one of a fixed few, not a random one: Chrome remembers the microphone grant per
//! origin, and the origin includes the port.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::Role;
use tungstenite::{Message, WebSocket};

use crate::daemon::{Event, NEXT_CONN};
use crate::protocol::{Reply, Request};
use crate::speech_assets::asset;

/// Tried in order; the first free one is used.
pub const SPEECH_PORTS: [u16; 3] = [47213, 47214, 47215];

/// A request's line and headers may not be longer than this.
const HEAD_LIMIT: usize = 8 * 1024;
/// How long a client has to send its request.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the WebSocket loop looks for replies while it waits for the page.
const POLL: Duration = Duration::from_millis(100);

pub struct SpeechHost {
    port: u16,
    token: String,
}

impl SpeechHost {
    #[cfg(test)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The only origin the WebSocket accepts.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// What Chrome opens.
    pub fn page_url(&self) -> String {
        format!("{}/t/{}/speech", self.origin(), self.token)
    }
}

/// Listens on the first free port of `SPEECH_PORTS` and serves in the background.
pub fn start(tx: Sender<Event>) -> Result<SpeechHost> {
    start_on(&SPEECH_PORTS, tx)
}

fn start_on(ports: &[u16], tx: Sender<Event>) -> Result<SpeechHost> {
    let (listener, port) = bind(ports)?;
    let host = SpeechHost { port, token: new_token()? };
    let token = host.token.clone();
    let origin = host.origin();
    tracing::info!(port, "speech page server listening");
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let (tx, token, origin) = (tx.clone(), token.clone(), origin.clone());
                    thread::spawn(move || {
                        if let Err(e) = serve(stream, &token, &origin, tx) {
                            tracing::debug!(error = %e, "speech page connection ended");
                        }
                    });
                }
                Err(e) => tracing::warn!(error = %e, "speech page connection failed"),
            }
        }
    });
    Ok(host)
}

fn bind(ports: &[u16]) -> Result<(TcpListener, u16)> {
    let mut last = None;
    for &port in ports {
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(listener) => {
                let port = listener.local_addr()?.port();
                return Ok((listener, port));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow!("no free port for the speech page among {ports:?}: {last:?}"))
}

/// 32 hex digits: 128 bits from the OS's secure random source. The token is what keeps other
/// local programs and web pages away from the page, so it must not be guessable.
fn new_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow!("no secure random source for the speech page token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct Head {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    /// Bytes read past the end of the head.
    rest: Vec<u8>,
}

fn read_head(stream: &mut TcpStream) -> io::Result<Option<Head>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > HEAD_LIMIT {
            return Ok(None);
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let Ok(text) = std::str::from_utf8(&buf[..end]) else { return Ok(None) };
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target)) = (first.next(), first.next()) else { return Ok(None) };
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok(Some(Head {
        method: method.to_string(),
        path: target.split('?').next().unwrap_or_default().to_string(),
        headers,
        rest: buf[end + 4..].to_vec(),
    }))
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn not_found(stream: &mut TcpStream) -> io::Result<()> {
    respond(stream, "404 Not Found", "text/plain; charset=utf-8", b"not found")
}

fn serve(mut stream: TcpStream, token: &str, origin: &str, tx: Sender<Event>) -> io::Result<()> {
    stream.set_read_timeout(Some(HEAD_TIMEOUT))?;
    let Some(head) = read_head(&mut stream)? else { return Ok(()) };
    // Everything that is not this start's token gets the same answer.
    let Some(name) =
        head.path.strip_prefix("/t/").and_then(|p| p.strip_prefix(token)).and_then(|p| p.strip_prefix('/'))
    else {
        return not_found(&mut stream);
    };
    if head.method != "GET" || name.contains("..") {
        return not_found(&mut stream);
    }
    if name == "ws" {
        return serve_socket(stream, head, origin, tx);
    }
    let name = if name == "speech" { "speech.html" } else { name };
    match asset(name) {
        Some((content_type, body)) => respond(&mut stream, "200 OK", content_type, body),
        None => not_found(&mut stream),
    }
}

fn serve_socket(mut stream: TcpStream, head: Head, origin: &str, tx: Sender<Event>) -> io::Result<()> {
    let header = |name: &str| head.headers.get(name).map(String::as_str).unwrap_or_default();
    if header("origin") != origin {
        tracing::warn!(origin = header("origin"), "speech page socket refused: foreign origin");
        return respond(&mut stream, "403 Forbidden", "text/plain; charset=utf-8", b"forbidden");
    }
    let key = header("sec-websocket-key");
    if !header("upgrade").eq_ignore_ascii_case("websocket") || key.is_empty() || header("sec-websocket-version") != "13"
    {
        return respond(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", b"bad request");
    }
    let accept = derive_accept_key(key.as_bytes());
    stream.write_all(
        format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Accept: {accept}\r\n\r\n"
        )
        .as_bytes(),
    )?;
    stream.set_read_timeout(Some(POLL))?;
    let mut ws = WebSocket::from_partially_read(stream, head.rest, Role::Server, None);
    relay(&mut ws, origin, tx);
    Ok(())
}

/// One page connection: `HostHello` first,
/// then the page's messages in and `ToExtension` replies out, until either side goes away.
fn relay(ws: &mut WebSocket<TcpStream>, origin: &str, tx: Sender<Event>) {
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (out_tx, out_rx) = mpsc::channel::<Reply>();
    let hello = Request::HostHello { origin: origin.to_string() };
    if tx.send(Event::Request { conn, req: hello, out: out_tx.clone() }).is_ok() {
        tracing::info!(conn, "speech page connected");
        'relay: loop {
            match ws.read() {
                Ok(Message::Text(text)) => match serde_json::from_str::<Value>(text.as_str()) {
                    Ok(message) => {
                        let req = Request::FromExtension { message };
                        if tx.send(Event::Request { conn, req, out: out_tx.clone() }).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "unreadable message from the speech page"),
                },
                Ok(_) => {} // pings are answered by tungstenite; a close is answered on the next write
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
            loop {
                match out_rx.try_recv() {
                    Ok(Reply::ToExtension { message }) => {
                        if ws.send(Message::text(message.to_string())).is_err() {
                            break 'relay;
                        }
                    }
                    Ok(_) => {} // replies meant for the command line
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            match ws.flush() {
                Ok(()) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
    }
    tracing::info!(conn, "speech page disconnected");
    let _ = tx.send(Event::Closed { conn });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::mpsc::Receiver;
    use tungstenite::client::IntoClientRequest;

    fn host() -> (SpeechHost, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (start_on(&[0], tx).unwrap(), rx)
    }

    fn addr(h: &SpeechHost) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, h.port()))
    }

    fn get(h: &SpeechHost, path: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(addr(h)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        let end = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&all[..end]).into_owned(), all[end + 4..].to_vec())
    }

    fn socket(h: &SpeechHost, path: &str, origin: &str) -> tungstenite::Result<WebSocket<TcpStream>> {
        let mut req = format!("ws://127.0.0.1:{}{path}", h.port()).into_client_request().unwrap();
        req.headers_mut().insert("Origin", origin.parse().unwrap());
        let stream = TcpStream::connect(addr(h)).unwrap();
        tungstenite::client(req, stream).map(|(ws, _)| ws).map_err(|e| match e {
            tungstenite::HandshakeError::Failure(e) => e,
            tungstenite::HandshakeError::Interrupted(_) => unreachable!(),
        })
    }

    fn next(rx: &Receiver<Event>) -> Event {
        rx.recv_timeout(Duration::from_secs(5)).expect("an event")
    }

    #[test]
    fn listens_on_loopback_only() {
        let (h, _rx) = host();
        assert!(h.page_url().starts_with("http://127.0.0.1:"));
        let (listener, _) = bind(&[0]).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn the_token_is_128_bits_of_hex_and_new_each_start() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn serves_the_page_with_the_right_token() {
        let (h, _rx) = host();
        let (head, body) = get(&h, &format!("/t/{}/speech", h.token));
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("text/html; charset=utf-8"));
        assert!(head.contains("Cache-Control: no-store"));
        assert!(head.contains("X-Content-Type-Options: nosniff"));
        assert!(String::from_utf8_lossy(&body).contains("speech.js"));
        let (head, body) = get(&h, &format!("/t/{}/dict/base.dat.gz?x=1", h.token));
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(!body.is_empty());
    }

    #[test]
    fn everything_else_is_the_same_404() {
        let (h, _rx) = host();
        let wrong = "0".repeat(32);
        for path in [
            format!("/t/{wrong}/speech"),
            "/speech".to_string(),
            "/".to_string(),
            format!("/t/{}", h.token),
            format!("/t/{}/../x", h.token),
            format!("/t/{}/nothing.js", h.token),
            format!("/t/{}x/speech", h.token),
        ] {
            let (head, body) = get(&h, &path);
            assert!(head.starts_with("HTTP/1.1 404"), "{path}: {head}");
            assert_eq!(body, b"not found", "{path}");
        }
    }

    #[test]
    fn a_taken_port_moves_to_the_next() {
        let (busy, taken) = bind(&[0]).unwrap();
        let (tx, _rx) = mpsc::channel();
        let h = start_on(&[taken, 0], tx).unwrap();
        assert_ne!(h.port(), taken);
        drop(busy);
    }

    #[test]
    fn the_page_socket_relays_both_ways() {
        let (h, rx) = host();
        let mut ws = socket(&h, &format!("/t/{}/ws", h.token), &h.origin()).unwrap();
        let (conn, out) = match next(&rx) {
            Event::Request { conn, req: Request::HostHello { origin }, out } => {
                assert_eq!(origin, h.origin());
                (conn, out)
            }
            _ => panic!("expected host-hello"),
        };
        ws.send(Message::text(json!({"type":"hello","extensionVersion":"desktop-page"}).to_string())).unwrap();
        match next(&rx) {
            Event::Request { conn: c, req: Request::FromExtension { message }, .. } => {
                assert_eq!(c, conn);
                assert_eq!(message["type"], "hello");
            }
            _ => panic!("expected from-extension"),
        }
        out.send(Reply::Ok).unwrap(); // not for the page: dropped
        out.send(Reply::ToExtension { message: json!({"type":"start"}) }).unwrap();
        let got = loop {
            match ws.read().unwrap() {
                Message::Text(t) => break t,
                _ => continue,
            }
        };
        assert_eq!(serde_json::from_str::<Value>(got.as_str()).unwrap(), json!({"type":"start"}));
        ws.close(None).unwrap();
        loop {
            match next(&rx) {
                Event::Closed { conn: c } => {
                    assert_eq!(c, conn);
                    break;
                }
                _ => continue,
            }
        }
    }

    #[test]
    fn a_foreign_origin_is_refused() {
        let (h, rx) = host();
        let err = socket(&h, &format!("/t/{}/ws", h.token), "https://example.com").unwrap_err();
        assert!(matches!(err, tungstenite::Error::Http(ref r) if r.status() == 403), "{err:?}");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn a_wrong_token_socket_is_refused() {
        let (h, rx) = host();
        let err = socket(&h, &format!("/t/{}/ws", "0".repeat(32)), &h.origin()).unwrap_err();
        assert!(matches!(err, tungstenite::Error::Http(ref r) if r.status() == 404), "{err:?}");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
