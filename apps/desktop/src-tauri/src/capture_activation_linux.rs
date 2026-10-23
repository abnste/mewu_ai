// SPDX-License-Identifier: MPL-2.0
//! Preserve a real tray activation across the asynchronous Portal request.
//! This observes only messages already delivered to our own GIO connection;
//! it neither monitors other applications nor synthesizes an input timestamp.
use gtk::{gio, glib};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(crate) struct Activation {
    timestamp: u32,
    received: Instant,
}
impl Activation {
    pub(crate) fn present(&self, window: &gtk::ApplicationWindow) {
        use gtk::prelude::*;
        if self.received.elapsed() <= Duration::from_secs(210) {
            // GTK requires the time of the original user request, not a new
            // server timestamp collected when asynchronous work finishes.
            window.present_with_time(self.timestamp);
        }
    }
}
fn pending() -> &'static Mutex<Option<Activation>> {
    static PENDING: OnceLock<Mutex<Option<Activation>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}
fn menu_timestamp(message: &gio::DBusMessage, incoming: bool) -> Option<u32> {
    if !incoming || message.message_type() != gio::DBusMessageType::MethodCall
        || message.interface().as_deref() != Some("com.canonical.dbusmenu")
        || message.member().as_deref() != Some("Event")
    { return None; }
    let (_, event, _, timestamp) = message.body()?.get::<(i32, String, glib::Variant, u32)>()?;
    (event == "clicked" && timestamp != 0).then_some(timestamp)
}
pub(crate) fn install() {
    // AppIndicator/libdbusmenu uses GIO's shared session connection. Its GTK
    // activate signal drops the DBusMenu Event's timestamp before Muda/Tauri
    // forwards the menu ID. Capture metadata here, leave the message untouched.
    gio::bus_get(gio::BusType::Session, gio::Cancellable::NONE, |result| {
        #[cfg(debug_assertions)]
        eprintln!("mewu-capture-activation observer-ready={}", result.is_ok());
        if let Ok(connection) = result {
            connection.add_filter(|_, message, incoming| {
                #[cfg(debug_assertions)]
                if incoming && message.interface().as_deref() == Some("com.canonical.dbusmenu") {
                    eprintln!("mewu-capture-activation menu-method={:?} signature={:?}",
                        message.member(), message.body().map(|body| body.type_().to_string()));
                }
                if let Some(timestamp) = menu_timestamp(message, incoming) {
                    if let Ok(mut slot) = pending().lock() {
                        *slot = Some(Activation { timestamp, received: Instant::now() });
                    }
                }
                Some(message.clone())
            });
        }
    });
}
pub(crate) fn take_menu() -> Option<Activation> {
    let activation = pending().lock().ok()?.take()
        .filter(|event| event.received.elapsed() <= Duration::from_secs(1));
    #[cfg(debug_assertions)]
    eprintln!("mewu-capture-activation menu-event-available={}", activation.is_some());
    activation
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::variant::ToVariant;
    fn event(name: &str, time: u32) -> gio::DBusMessage {
        let message = gio::DBusMessage::new_method_call(None, "/com/canonical/menu/1",
            Some("com.canonical.dbusmenu"), "Event");
        message.set_body(&(3_i32, name, 0_i32.to_variant(), time).to_variant());
        message
    }
    #[test]
    fn only_incoming_real_menu_clicks_supply_activation() {
        let message = event("clicked", 12345);
        assert_eq!(menu_timestamp(&message, true), Some(12345));
        assert_eq!(menu_timestamp(&message, false), None);
        assert_eq!(menu_timestamp(&event("opened", 12345), true), None);
        assert_eq!(menu_timestamp(&event("clicked", 0), true), None);
        message.set_interface(Some("org.example.Other"));
        assert_eq!(menu_timestamp(&message, true), None);
    }
    #[test]
    fn malformed_or_nonmethod_messages_do_not_supply_activation() {
        let message = event("clicked", 12345);
        message.set_body(&("not-an-event",).to_variant());
        assert_eq!(menu_timestamp(&message, true), None);
        message.set_body(&(3_i32, "clicked", 0_i32.to_variant(), 12345_u32).to_variant());
        message.set_message_type(gio::DBusMessageType::Signal);
        assert_eq!(menu_timestamp(&message, true), None);
    }
}
