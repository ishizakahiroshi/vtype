//! Messages on the local IPC channel: one JSON object per line (parent plan D14).
//!
//! The command line talks to the daemon over this endpoint (`vtype toggle` and friends, one request
//! and one reply). The speech page's WebSocket (speech_host.rs) is turned into the same
//! `host-hello` / `from-extension` requests, so the daemon has one way to hear a recognizer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Input mode, the same three values as `InputMode` in vtype-core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    #[default]
    Normal,
    En,
    Kana,
}

impl InputMode {
    pub fn as_str(self) -> &'static str {
        match self {
            InputMode::Normal => "normal",
            InputMode::En => "en",
            InputMode::Kana => "kana",
        }
    }
}

/// Client (the command line, or speech_host.rs for the page) to daemon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Request {
    Toggle {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<InputMode>,
    },
    Start {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<InputMode>,
    },
    Stop,
    SetMode {
        mode: InputMode,
    },
    Status,
    OpenSettings,
    Diagnostics,
    Quit,
    /// A recognizer connected: the speech page, with `origin` `http://127.0.0.1:<port>`.
    HostHello {
        origin: String,
    },
    /// A message the speech page sent, relayed unchanged.
    FromExtension {
        message: Value,
    },
}

/// Daemon to client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Reply {
    Ok,
    Status {
        connected: bool,
        recording: bool,
        mode: InputMode,
        version: String,
    },
    Error {
        code: String,
        message: String,
    },
    Diagnostics {
        report: Value,
    },
    /// A message for the speech page; speech_host.rs sends it unchanged.
    ToExtension {
        message: Value,
    },
}

impl Reply {
    pub fn error(code: &str, message: impl Into<String>) -> Reply {
        Reply::Error { code: code.to_string(), message: message.into() }
    }
}

// ---------------------------------------------------------------------------
// What the desktop app and the speech page say to each other (the vocabulary of the extension's
// former Native Messaging link, kept so both sides stay small). The TypeScript
// side is packages/extension/src/shared/native-messages.ts; both are tested against
// tests/fixtures/nm-messages.json.
// ---------------------------------------------------------------------------

/// Desktop app to extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum ToExtension {
    Hello {
        native_version: String,
        os: String,
    },
    Start {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<InputMode>,
    },
    Stop,
    SetMode {
        mode: InputMode,
    },
    GetState,
    /// The desktop app's current settings, for the extension's options page.
    NativeConfig {
        config: crate::config::NativeConfig,
    },
    OpenOptions,
}

/// Extension to desktop app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum FromExtension {
    Hello {
        extension_version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser: Option<String>,
    },
    State {
        mode: InputMode,
        recording: bool,
    },
    Session {
        event: SessionEvent,
    },
    /// The options page changed the desktop app's settings.
    SetNativeConfig {
        config: crate::config::NativeConfig,
    },
    GetNativeConfig,
    Error {
        code: String,
    },
    /// The daemon's speech page (standalone plan C2): the user pressed "agree and start".
    Consent,
    /// Where the speech page's first-run setup stands.
    PageState {
        consented: bool,
        mic_granted: bool,
    },
}

/// A recognition session the desktop app started. `interim` is shown, only `final` is typed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SessionEvent {
    Started,
    Interim {
        text: String,
    },
    Final {
        text: String,
    },
    Ended {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip_request(req: Request, expected: Value) {
        let text = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
    }

    #[test]
    fn requests_round_trip() {
        round_trip_request(Request::Toggle { mode: None }, json!({"type":"toggle"}));
        round_trip_request(Request::Start { mode: Some(InputMode::Kana) }, json!({"type":"start","mode":"kana"}));
        round_trip_request(Request::Stop, json!({"type":"stop"}));
        round_trip_request(Request::SetMode { mode: InputMode::En }, json!({"type":"set-mode","mode":"en"}));
        round_trip_request(Request::Status, json!({"type":"status"}));
        round_trip_request(Request::OpenSettings, json!({"type":"open-settings"}));
        round_trip_request(Request::Quit, json!({"type":"quit"}));
        round_trip_request(
            Request::HostHello { origin: "chrome-extension://abc/".into() },
            json!({"type":"host-hello","origin":"chrome-extension://abc/"}),
        );
        round_trip_request(
            Request::FromExtension { message: json!({"type":"hello"}) },
            json!({"type":"from-extension","message":{"type":"hello"}}),
        );
    }

    #[test]
    fn replies_round_trip() {
        let cases = [
            (Reply::Ok, json!({"type":"ok"})),
            (
                Reply::Status { connected: false, recording: false, mode: InputMode::Normal, version: "0.1.0".into() },
                json!({"type":"status","connected":false,"recording":false,"mode":"normal","version":"0.1.0"}),
            ),
            (Reply::error("not_connected", "x"), json!({"type":"error","code":"not_connected","message":"x"})),
            (
                Reply::ToExtension { message: json!({"type":"stop"}) },
                json!({"type":"to-extension","message":{"type":"stop"}}),
            ),
        ];
        for (reply, expected) in cases {
            let text = serde_json::to_string(&reply).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
            assert_eq!(serde_json::from_str::<Reply>(&text).unwrap(), reply);
        }
    }

    /// Shared with packages/extension/tests/native-messages.test.ts.
    #[test]
    fn the_shared_fixture_round_trips() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/nm-messages.json")).unwrap();
        for m in fixture["toExtension"].as_array().unwrap() {
            let parsed: ToExtension = serde_json::from_value(m.clone()).unwrap_or_else(|e| panic!("{m}: {e}"));
            assert_eq!(&serde_json::to_value(&parsed).unwrap(), m);
        }
        for m in fixture["fromExtension"].as_array().unwrap() {
            let parsed: FromExtension = serde_json::from_value(m.clone()).unwrap_or_else(|e| panic!("{m}: {e}"));
            assert_eq!(&serde_json::to_value(&parsed).unwrap(), m);
        }
        for m in fixture["invalidToExtension"].as_array().unwrap() {
            assert!(serde_json::from_value::<ToExtension>(m.clone()).is_err(), "{m}");
        }
        for m in fixture["invalidFromExtension"].as_array().unwrap() {
            assert!(serde_json::from_value::<FromExtension>(m.clone()).is_err(), "{m}");
        }
    }

    #[test]
    fn a_missing_mode_reads_as_none() {
        let req: Request = serde_json::from_str(r#"{"type":"toggle"}"#).unwrap();
        assert_eq!(req, Request::Toggle { mode: None });
    }
}
