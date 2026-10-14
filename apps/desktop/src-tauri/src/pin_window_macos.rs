// SPDX-License-Identifier: MPL-2.0
//! AppKit ordering only. The shared physical geometry, source leases and ready
//! handshake remain authoritative; no native object leaves the UI thread.
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode};
use tauri::WebviewWindow;

fn native(window: &WebviewWindow) -> Result<Retained<NSWindow>, String> {
    MainThreadMarker::new().ok_or("贴图窗口只能在界面线程操作")?;
    let raw = window.ns_window().map_err(|_| "贴图窗口不可用")?;
    // SAFETY: Tauri owns this exact WebviewWindow's +0 NSWindow. Retain while
    // synchronously ordering it on the AppKit main thread; never store pointers.
    unsafe { Retained::<NSWindow>::retain(raw.cast()) }.ok_or("贴图窗口不可用".into())
}

pub(super) fn show_without_activation(window: &WebviewWindow) -> Result<(), String> {
    let pin = native(window)?;
    pin.setHidesOnDeactivate(false);
    // Independent pins remain available on other desktop Spaces, including
    // alongside a fullscreen application. This is not a transient child sheet.
    pin.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    // Unlike makeKeyAndOrderFront, ordering does not steal the capture's key
    // window. A later explicit pointer/keyboard action may focus the pin.
    pin.orderFrontRegardless();
    if !pin.isVisible() {
        return Err("无法显示贴图".into());
    }
    Ok(())
}

pub(super) fn beneath_space(window: &WebviewWindow, space: &WebviewWindow) -> Result<(), String> {
    let pin = native(window)?;
    let parent = native(space)?;
    if !pin.isVisible() || !parent.isVisible() {
        return Ok(());
    }
    if !crate::pin_host::is_pin_label(window.label()) || space.label() != "space" {
        return Err("贴图窗口身份无效".into());
    }
    // Space renders these same independent objects. Native duplicates must sit
    // underneath it; hiding Space automatically exposes the unchanged pin.
    pin.orderWindow_relativeTo(NSWindowOrderingMode::Below, parent.windowNumber());
    Ok(())
}
