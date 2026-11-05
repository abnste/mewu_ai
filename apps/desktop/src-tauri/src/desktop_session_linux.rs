// SPDX-License-Identifier: MPL-2.0
//! Keep WebKit's widgets and process-global cursor cache on one GdkDisplay.
//! The compositor session remains independent: XWayland windows still use
//! the Wayland desktop's screenshot, screencast and shortcut portals.
use gtk::prelude::*;
use std::sync::OnceLock;

// GTK objects stay on the UI thread. Worker tasks may only read this value.
static WAYLAND_DISPLAY: OnceLock<bool> = OnceLock::new();
fn wayland_session(display: Option<&str>, session: Option<&str>) -> bool {
    display.is_some_and(|s| !s.trim().is_empty())
        || session.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
}

pub(crate) fn uses_portal() -> bool {
    wayland_session(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
    ) || WAYLAND_DISPLAY.get().copied().unwrap_or(false)
}

/// PAM may give graphical sessions CAP_WAKE_ALARM. A less privileged portal
/// cannot inspect /proc/<pid>/root for that client. Re-exec our current inode
/// without inherited privileges before opening any GUI, IPC or user storage.
pub(crate) fn shed_ambient_capabilities() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let status = std::fs::read_to_string("/proc/thread-self/status")?;
    let privileged = status.lines().find_map(|s| s.strip_prefix("CapAmb:"))
        .is_some_and(|bits| bits.trim().bytes().any(|b| b != b'0'));
    if !privileged { return Ok(()); }
    rustix::thread::clear_ambient_capability_set()?;
    let mut arguments = std::env::args_os();
    let mut command = std::process::Command::new("/proc/self/exe");
    if let Some(name) = arguments.next() { command.arg0(name); }
    Err(command.args(arguments).exec())
}

pub(crate) fn observe_display() {
    assert!(gtk::is_initialized_main_thread());
    WAYLAND_DISPLAY.get_or_init(|| {
        gtk::gdk::Display::default()
            .is_some_and(|display| display.type_().name() == "GdkWaylandDisplay")
    });
}

pub(crate) fn initialize() {
    // Before GTK/Tao creates any window. Never migrate a live WebKit tree to
    // another display, and never switch the global display around callbacks.
    // X11 permits precise positioning of independent desktop objects. On a
    // compositor without XWayland, retain Wayland and report that limitation.
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let display = std::env::var("DISPLAY").ok();
    gtk::gdk::set_allowed_backends(backend_order(
        wayland_session(wayland.as_deref(), session.as_deref()),
        display.as_deref(),
        crate::capture_wayland_host::is_xwayland_server,
    ));
}

fn backend_order(portal: bool, display: Option<&str>, is_xwayland: impl FnOnce(&str) -> bool) -> &'static str {
    if portal && !display.filter(|s| !s.is_empty()).is_some_and(is_xwayland) {
        "wayland,x11"
    } else {
        "x11,wayland"
    }
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

    #[test]
    fn desktop_detection_is_safe_on_background_workers_without_gtk() {
        std::thread::spawn(uses_portal).join().unwrap();
    }

    #[test]
    fn a_leftover_xorg_display_does_not_take_the_wayland_capture_window() {
        assert_eq!(backend_order(true, Some(":9"), |_| false), "wayland,x11");
        assert_eq!(backend_order(true, Some(":0"), |_| true), "x11,wayland");
        assert_eq!(backend_order(true, None, |_| unreachable!()), "wayland,x11");
        assert_eq!(backend_order(false, Some(":9"), |_| unreachable!()), "x11,wayland");
    }
}
