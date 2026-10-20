// SPDX-License-Identifier: MPL-2.0
//! Local control transport. This crate never opens business storage or starts a GUI.
//!
//! The host must still check its exact plugin revision and operation authority at
//! dispatch and commit. A transport grant is not a business-operation grant.
mod endpoint;
#[cfg(target_os = "linux")]
mod linux_appimage;
mod protocol;
mod server;
mod transport;

pub use endpoint::Endpoint;
pub use protocol::{
    ControlError, ErrorCode, ExecutionOutcome, Request, Response, ResponseBody, MAX_FRAME_BYTES,
    PROTOCOL_VERSION,
};
pub use server::{Dispatcher, Server, ServerHandle, ServerOptions};
pub use transport::Client;

#[cfg(test)]
mod tests;
