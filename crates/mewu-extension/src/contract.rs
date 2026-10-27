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

/// Small vector artwork, rendered as fixed SVG path nodes, never HTML/URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionIcon {
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub filled: bool,
}
pub fn valid_extension_icon(icon: &ExtensionIcon) -> bool {
    !icon.paths.is_empty() && icon.paths.len() <= 16
        && icon.paths.iter().map(String::len).sum::<usize>() <= 8192
        && icon.paths.iter().all(|path| !path.trim().is_empty() && path.len() <= 4096
            && path.bytes().all(|b| b.is_ascii_digit() || b"MmZzLlHhVvCcSsQqTtAaEe.,+- \t\n".contains(&b)))
        && icon.color.as_ref().is_none_or(|color| color.len()==7 && color.starts_with('#') && color[1..].bytes().all(|b|b.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ExtensionIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    pub license: String,
    pub host_api: u32,
    pub runtime: String,
    pub entry: String,
    /// Omitted for portable JavaScript packages. This restricts the package,
    /// not which native capabilities the host grants it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

pub const PLUGIN_PLATFORMS: [&str; 3] = ["windows-x86_64", "darwin-aarch64", "linux-x86_64"];
pub fn current_plugin_platform() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) { "windows-x86_64" }
    else if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "darwin-aarch64" }
    else if cfg!(all(target_os = "linux", target_arch = "x86_64")) { "linux-x86_64" }
    else { "unsupported" }
}
pub fn valid_plugin_platforms(platforms: &[String]) -> bool {
    !platforms.is_empty() && platforms.len() <= PLUGIN_PLATFORMS.len()
        && platforms.iter().all(|p| p == "any" || PLUGIN_PLATFORMS.contains(&p.as_str()))
        && platforms.iter().collect::<std::collections::BTreeSet<_>>().len() == platforms.len()
        && (!platforms.iter().any(|p| p == "any") || platforms.len() == 1)
}
pub fn plugin_platform_supported(platforms: Option<&[String]>, platform: &str) -> bool {
    PLUGIN_PLATFORMS.contains(&platform) && platforms.is_none_or(|values|
        valid_plugin_platforms(values) && values.iter().any(|p| p == "any" || p == platform))
}
/// Existing service metadata can narrow one native feature in a portable package.
pub fn service_platforms(service: &ServiceRegistration) -> Result<Option<Vec<String>>, ExtensionError> {
    if service.metadata.as_ref().and_then(|m| m.get("requiresRegion")).is_some_and(|v| !v.is_boolean()) {
        return Err(ExtensionError::InvalidPackage("invalid service region requirement"));
    }
    let Some(value) = service.metadata.as_ref().and_then(|m| m.get("platforms")) else { return Ok(None) };
    let platforms: Vec<String> = serde_json::from_value(value.clone())
        .map_err(|_| ExtensionError::InvalidPackage("invalid service platforms"))?;
    if !valid_plugin_platforms(&platforms) { return Err(ExtensionError::InvalidPackage("invalid service platforms")); }
    Ok(Some(platforms))
}

/// Hold triggers admit bare modifier keys only. Letters, digits and every other
/// key are rejected at registration, so a trigger can never forward typed
/// characters and become a keylogger. `code` names the key that starts the hold
/// (KeyboardEvent names); `with` names the extra modifiers that must be held
/// together with it. Both map onto the same {ctrl, shift, alt} set, so the
/// combination must not repeat the primary key.
pub fn validate_trigger(service: &ServiceRegistration) -> Result<(), ExtensionError> {
    if service.kind != ServiceKind::Trigger { return Ok(()); }
    let invalid = ExtensionError::InvalidPackage;
    let Some(trigger) = service.metadata.as_ref().and_then(|m| m.get("trigger")) else {
        return Err(invalid("触发器缺少按键描述"));
    };
    if !trigger.is_object() { return Err(invalid("触发器按键描述无效")); }
    let code = trigger.get("code").and_then(|v| v.as_str()).unwrap_or_default();
    let modifier = match code {
        "Alt" => "alt",
        "Control" => "ctrl",
        "Shift" => "shift",
        _ => return Err(invalid("触发器按键仅支持 Alt、Control 或 Shift")),
    };
    let mut combination = std::collections::BTreeSet::from([modifier]);
    if let Some(extra) = trigger.get("with") {
        let extra: Vec<String> = serde_json::from_value(extra.clone())
            .map_err(|_| invalid("触发器组合无效"))?;
        for key in extra {
            let mapped = match key.as_str() {
                "Ctrl" => "ctrl",
                "Shift" => "shift",
                "Alt" => "alt",
                _ => return Err(invalid("触发器组合仅支持 Ctrl、Shift 或 Alt")),
            };
            if !combination.insert(mapped) {
                return Err(invalid("触发器组合不能重复修饰键"));
            }
        }
    }
    Ok(())
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
    Trigger,
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

/// Fixed categories only: no JavaScript message, stack, or custom error name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerFailureKind {
    Syntax,
    Type,
    Range,
    Reference,
    Uri,
    Eval,
    Error,
    Unknown,
}
impl HandlerFailureKind {
    pub(crate) fn from_outcome(value: Option<&str>) -> Self {
        match value {
            Some("syntax") => Self::Syntax,
            Some("type") => Self::Type,
            Some("range") => Self::Range,
            Some("reference") => Self::Reference,
            Some("uri") => Self::Uri,
            Some("eval") => Self::Eval,
            Some("error") => Self::Error,
            _ => Self::Unknown,
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Syntax => "JS_SYNTAX_ERROR",
            Self::Type => "JS_TYPE_ERROR",
            Self::Range => "JS_RANGE_ERROR",
            Self::Reference => "JS_REFERENCE_ERROR",
            Self::Uri => "JS_URI_ERROR",
            Self::Eval => "JS_EVAL_ERROR",
            Self::Error => "JS_ERROR",
            Self::Unknown => "JS_UNKNOWN_ERROR",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("Invalid extension package: {0}")]
    InvalidPackage(&'static str),
    #[error("Invalid extension protocol: {0}")]
    Protocol(&'static str),
    #[error("Extension runtime failed: {0}")]
    Runtime(&'static str),
    #[error("Extension service handler failed")]
    Handler(HandlerFailureKind),
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
            Self::Handler(kind) => kind.code(),
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
