// SPDX-License-Identifier: MPL-2.0
//! Code extensions run in a separate, bounded QuickJS interpreter. Only the
//! explicit host bridge can perform I/O; the host must authorize every call.
mod contract;
mod package;
mod runtime;
mod worker;

pub use contract::*;
pub use package::{load_directory_package, validate_package, ExtensionPackage};
pub use runtime::{ExtensionRuntime, RuntimeLimits, SDK_SOURCE};
pub use worker::{run_stdio_worker, run_worker};
