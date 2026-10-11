// SPDX-License-Identifier: MPL-2.0
//! Native screen-capture permission checks. Never captures or uploads pixels.
#![cfg(target_os = "macos")]

// Public CoreGraphics APIs, available since macOS 10.15 (our minimum is 12.3).
// https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess()
// https://developer.apple.com/documentation/coregraphics/cgrequestscreencaptureaccess()
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

const REQUIRED: &str = "MewuAI 尚未获得屏幕录制权限。请在系统设置的隐私与安全设置中允许 MewuAI 录制屏幕，授权后重新打开应用。";

/// Checks permission immediately before capture without showing a prompt.
pub(crate) fn preflight() -> Result<(), String> {
    // SAFETY: This public API takes no pointers and returns a C bool.
    if unsafe { CGPreflightScreenCaptureAccess() } {
        Ok(())
    } else {
        Err(REQUIRED.into())
    }
}

/// Requests permission only in response to an explicit user capture action.
/// Call before hiding presentation windows, without holding host/store locks.
pub(crate) fn request() -> Result<(), String> {
    // SAFETY: This public API takes no pointers and returns a C bool.
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    // SAFETY: This public API takes no pointers. The OS owns its permission UI.
    unsafe { CGRequestScreenCaptureAccess() };
    // Recheck actual access: dismissing the prompt is not an authorization.
    preflight()
}
