// SPDX-License-Identifier: MPL-2.0
//! Rules are local user configuration. Events are data, never executable code.
//! The desktop owns side effects; this crate owns validation, matching,
//! scheduling and the durable execution ledger, independent of UI and OS.
mod model;
mod script;
mod store;
pub use model::*;
pub use script::{evaluate, validate_script};
pub use store::*;
#[cfg(test)]
mod tests;
