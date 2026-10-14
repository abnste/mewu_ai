// SPDX-License-Identifier: MPL-2.0
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const HOST_API: u32 = 2;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_MODULE_BYTES: usize = 1024 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_MODULES: usize = 128;
pub const MAX_PROTOCOL_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
pub const MAX_SERVICES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    pub license: String,
    pub host_api: u32,
    pub runtime: String,
    pub entry: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceKind {
    Service,
    Command,
    Tool,
    Agent,
    Memory,
    Model,
    Channel,
    Renderer,
    Ui,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRegistration {
    pub kind: ServiceKind,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
// The transport queue has two entries; preserving the direct public manifest
// API costs < 1 KiB and avoids a separately allocated box per normal request.
#[allow(clippy::large_enum_variant)]
pub enum WorkerRequest {
    Initialize {
        id: String,
        manifest: ExtensionManifest,
        modules: BTreeMap<String, String>,
    },
    Invoke {
        id: String,
        #[serde(rename = "serviceId")]
        service_id: String,
        input: Value,
        #[serde(default)]
        context: Value,
    },
    #[serde(rename = "host.result")]
    HostResult {
        id: String,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<ProtocolError>,
    },
    Cancel {
        #[serde(default)]
        id: Option<String>,
    },
    Shutdown {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerResponse {
    Initialized {
        id: String,
        registrations: Vec<ServiceRegistration>,
    },
    Result {
        id: String,
        result: Value,
    },
    Error {
        id: String,
        error: ProtocolError,
    },
    #[serde(rename = "host.call")]
    HostCall {
        id: String,
        method: String,
        params: Value,
    },
    Stopped {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Invalid extension package: {0}")]
    InvalidPackage(&'static str),
    #[error("Invalid extension protocol: {0}")]
    Protocol(&'static str),
    #[error("Extension runtime failed: {0}")]
    Runtime(&'static str),
    #[error("Extension execution timed out")]
    Timeout,
    #[error("Extension execution was canceled")]
    Canceled,
    #[error("Extension transport failed")]
    Transport,
}

impl ExtensionError {
    pub fn protocol_error(&self) -> ProtocolError {
        let code = match self {
            Self::InvalidPackage(_) => "INVALID_PACKAGE",
            Self::Protocol(_) => "INVALID_PROTOCOL",
            Self::Runtime(_) => "RUNTIME_ERROR",
            Self::Timeout => "TIMEOUT",
            Self::Canceled => "CANCELED",
            Self::Transport => "TRANSPORT_ERROR",
        };
        ProtocolError {
            code: code.into(),
            message: self.to_string(),
        }
    }
}

pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub(crate) fn bounded_text(value: &str, max: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.trim().is_empty())
        && value.len() <= max
        && !value.chars().any(char::is_control)
}

pub(crate) fn validate_value(value: &Value) -> Result<(), ExtensionError> {
    fn depth(value: &Value, remaining: u8) -> bool {
        if remaining == 0 {
            return false;
        }
        match value {
            Value::Array(items) => items.iter().all(|v| depth(v, remaining - 1)),
            Value::Object(items) => items.values().all(|v| depth(v, remaining - 1)),
            _ => true,
        }
    }
    if !depth(value, 48)
        || serde_json::to_vec(value)
            .map_err(|_| ExtensionError::Protocol("invalid JSON value"))?
            .len()
            > MAX_VALUE_BYTES
    {
        return Err(ExtensionError::Protocol("JSON value exceeds limits"));
    }
    Ok(())
}
