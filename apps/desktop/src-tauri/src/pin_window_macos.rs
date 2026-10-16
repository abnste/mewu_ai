// SPDX-License-Identifier: MPL-2.0
//! Synchronous AppKit geometry and ordering. The shared physical geometry,
//! source leases and ready handshake remain authoritative; no native object
//! leaves the UI thread.
use super::Rect;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode};
use objc2_core_graphics::{CGDisplayPixelsHigh, CGMainDisplayID};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tauri::WebviewWindow;

fn native(window: &WebviewWindow) -> Result<Retained<NSWindow>, String> {
    MainThreadMarker::new().ok_or("贴图窗口只能在界面线程操作")?;
    let raw = window.ns_window().map_err(|_| "贴图窗口不可用")?;
    // SAFETY: Tauri owns this exact WebviewWindow's +0 NSWindow. Retain while
    // synchronously ordering it on the AppKit main thread; never store pointers.
    unsafe { Retained::<NSWindow>::retain(raw.cast()) }.ok_or("贴图窗口不可用".into())
}

/// Apply the complete outer frame before the caller's strict physical readback.
/// Tao 0.37.1 queues its separate position/size setters even on the UI thread.
pub(super) fn apply_rect(window: &WebviewWindow, rect: Rect) -> Result<(), String> {
    let pin = native(window)?;
    apply_native_rect(&pin, rect)
}

fn apply_native_rect(pin: &NSWindow, rect: Rect) -> Result<(), String> {
    MainThreadMarker::new().ok_or("贴图窗口只能在界面线程操作")?;
    let scale = pin.backingScaleFactor();
    if !scale.is_finite() || scale <= 0. || rect.width == 0 || rect.height == 0 {
        return Err("贴图尺寸无效".into());
    }
    let primary_height = CGDisplayPixelsHigh(CGMainDisplayID()) as f64;
    // Invert Tao's existing outer_position contract, including its main-display
    // reference. Keep the same physical coordinates and strict readback gate.
    let x = f64::from(rect.x) / scale;
    let width = f64::from(rect.width) / scale;
    let height = f64::from(rect.height) / scale;
    let y = primary_height - f64::from(rect.y) / scale - height;
    if primary_height <= 0.
        || ![x, y, width, height].into_iter().all(f64::is_finite)
        || width <= 0.
        || height <= 0.
    {
        return Err("贴图尺寸无效".into());
    }
    // Pin windows are borderless: content and outer frame have the same size.
    // One synchronous native call preserves the requested top edge when size
    // changes. It neither orders nor activates the hidden window.
    pin.setFrame_display(
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)),
        false,
    );
    Ok(())
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
