// SPDX-License-Identifier: MPL-2.0
//! Keep WebKit's widgets and process-global cursor cache on one GdkDisplay.
//! The compositor session remains independent: XWayland windows still use
//! the Wayland desktop's screenshot, screencast and shortcut portals.
use gtk::prelude::*;
fn wayland_session(display: Option<&str>, session: Option<&str>) -> bool {
    display.is_some_and(|s| !s.trim().is_empty())
        || session.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
}

pub(crate) fn uses_portal() -> bool {
    wayland_session(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
    ) || gtk::gdk::Display::default()
        .is_some_and(|display| display.type_().name() == "GdkWaylandDisplay")
}

pub(crate) fn initialize() {
    // Before GTK/Tao creates any window. Never migrate a live WebKit tree to
    // another display, and never switch the global display around callbacks.
    // X11 permits precise positioning of independent desktop objects. On a
    // compositor without XWayland, retain Wayland and report that limitation.
    gtk::gdk::set_allowed_backends("x11,wayland");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xwayland_rendering_never_selects_x11_screen_capture() {
        assert!(wayland_session(Some("wayland-0"), Some("wayland")));
        assert!(wayland_session(None, Some("wayland")));
        assert!(wayland_session(Some("wayland-1"), Some("x11")));
        assert!(!wayland_session(None, Some("x11")));
        assert!(!wayland_session(Some(" "), None));
    }
}
