//! Password fields through AT-SPI (child plan C7-C2). AT-SPI cannot be asked "what has the focus
//! now", so a thread listens to focus changes and remembers whether the newly focused element is
//! a password field. When AT-SPI is not there (no accessibility bus, or apps that do not expose
//! themselves), the answer is "unknown" and vtype types (parent plan D19, checklist Q3).
//!
//! It does not switch accessibility on for the session: that changes every app's behaviour and
//! is the user's call.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use atspi::events::object::StateChangedEvent;
use atspi::events::ObjectEvents;
use atspi::proxy::accessible::AccessibleProxy;
use atspi::{AccessibilityConnection, Event, Role, State};
use zbus::export::futures_core::Stream;

use crate::platform::FieldInfo;

#[derive(Default)]
struct Focus {
    status: String,
    is_password: Option<bool>,
    /// Whether it is an editable text field (what the clear button may empty).
    is_text_field: Option<bool>,
    /// Which element that answer is about (bus name and path).
    item: Option<(String, String)>,
}

#[derive(Default)]
pub struct Tracker {
    focus: Arc<Mutex<Focus>>,
    started: AtomicBool,
}

impl Tracker {
    /// Starts listening (once).
    pub fn start(&self) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let focus = self.focus.clone();
        set_status(&focus, "connecting");
        let spawned = std::thread::Builder::new().name("atspi".into()).spawn(move || {
            let result = zbus::block_on(listen(focus.clone()));
            let status = match result {
                Ok(()) => "stopped".to_string(),
                Err(e) => format!("unavailable: {e}"),
            };
            tracing::info!(%status, "AT-SPI");
            let mut f = focus.lock().unwrap_or_else(|e| e.into_inner());
            f.status = status;
            f.is_password = None;
            f.is_text_field = None;
        });
        if let Err(e) = spawned {
            set_status(&self.focus, &format!("unavailable: {e}"));
        }
    }

    pub fn field(&self) -> FieldInfo {
        let f = self.focus.lock().unwrap_or_else(|e| e.into_inner());
        FieldInfo { is_password: f.is_password, caret_rect: None, app_id: None, is_text_field: f.is_text_field }
    }

    pub fn status(&self) -> String {
        self.focus.lock().unwrap_or_else(|e| e.into_inner()).status.clone()
    }
}

fn set_status(focus: &Mutex<Focus>, status: &str) {
    focus.lock().unwrap_or_else(|e| e.into_inner()).status = status.to_string();
}

async fn role_of(conn: &AccessibilityConnection, event: &StateChangedEvent) -> Option<Role> {
    let name = event.item.name()?.clone();
    let proxy = AccessibleProxy::builder(conn.connection())
        .destination(name)
        .ok()?
        .path(event.item.path().clone())
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()?;
    proxy.get_role().await.ok()
}

async fn listen(focus: Arc<Mutex<Focus>>) -> Result<(), atspi::AtspiError> {
    let conn = AccessibilityConnection::new().await?;
    conn.register_event::<StateChangedEvent>().await?;
    set_status(&focus, "connected");
    let mut events = Box::pin(conn.event_stream());
    while let Some(event) = std::future::poll_fn(|cx| Pin::as_mut(&mut events).poll_next(cx)).await {
        let Ok(Event::Object(ObjectEvents::StateChanged(changed))) = event else { continue };
        if changed.state != State::Focused {
            continue;
        }
        let item = (changed.item.name_as_str().unwrap_or_default().to_string(), changed.item.path_as_str().to_string());
        if changed.enabled {
            let role = role_of(&conn, &changed).await;
            let mut f = focus.lock().unwrap_or_else(|e| e.into_inner());
            f.is_password = role.as_ref().map(|r| *r == Role::PasswordText);
            f.is_text_field = role.as_ref().map(|r| matches!(r, Role::Text | Role::Entry | Role::PasswordText));
            f.item = Some(item);
        } else {
            // Focus left an element. The next one may already have said it has it (the order
            // of the two events is not fixed), so only forget the answer about this one.
            let mut f = focus.lock().unwrap_or_else(|e| e.into_inner());
            if f.item.as_ref() == Some(&item) {
                f.is_password = None;
                f.is_text_field = None;
                f.item = None;
            }
        }
    }
    Ok(())
}
