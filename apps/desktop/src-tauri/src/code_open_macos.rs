// SPDX-License-Identifier: MPL-2.0
//! Native dispatch for an address already validated by the code_open caller.
#![cfg(target_os = "macos")]

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};

/// The caller must supply a canonical HTTP(S), recipient-only mailto, or global
/// tel address, or the canonical local .app file URL from open_application produced by code_open's existing validation. This is not an IPC
/// command and must not receive an arbitrary user-supplied scheme or arguments.
/// The caller retains its action permit until this returns.
pub(crate) fn dispatch(
    address: String,
    allowed: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    // Apple documents openURL as safe from any thread since macOS 10.6.
    // https://developer.apple.com/documentation/appkit/nsworkspace/open(_:)
    autoreleasepool(|_| {
        let address = NSString::from_str(&address);
        let url = NSURL::URLWithString(&address).ok_or("无法构造打开地址")?;
        let workspace = NSWorkspace::sharedWorkspace();
        // Keep the final source/exit fence adjacent to the irreversible dispatch.
        allowed()?;
        if workspace.openURL(&url) {
            Ok(())
        } else {
            Err("无法打开应用，可复制内容后重试".into())
        }
    })
}
