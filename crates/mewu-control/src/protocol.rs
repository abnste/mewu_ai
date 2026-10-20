// SPDX-License-Identifier: MPL-2.0
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Value,
}
impl Request {
    pub fn new(id: impl Into<String>, method: impl Into<String>, params: Value) -> Self {
        Self {
            id: id.into(),
            method: method.into(),
            params,
        }
    }
    pub fn validate(&self) -> Result<(), ControlError> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':'))
        };
        if !valid(&self.id) || !valid(&self.method) || !self.params.is_object() {
            return Err(ControlError::not_sent(
                ErrorCode::InvalidRequest,
                "Expected bounded id/method and object params",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOutcome {
    NotSent,
    Executed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    InvalidResponse,
    FrameTooLarge,
    Unavailable,
    PermissionDenied,
    Disabled,
    ShuttingDown,
    Timeout,
    Disconnected,
    Internal,
    ResultTooLarge,
    Unsupported,
    Busy,
    Conflict,
    Canceled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlError {
    pub code: ErrorCode,
    pub message: String,
    pub outcome: ExecutionOutcome,
    /// A fact about the effect is never an instruction to replay it.
    pub retryable: bool,
}
impl ControlError {
    pub fn new(code: ErrorCode, message: impl Into<String>, outcome: ExecutionOutcome) -> Self {
        // Dispatcher diagnostics must be short, intentional public messages.
        let mut message = message.into();
        if message.len() > 512 {
            let mut end = 512;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        Self {
            code,
            message,
            outcome,
            retryable: false,
        }
    }
    pub fn not_sent(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(code, message, ExecutionOutcome::NotSent)
    }
    pub fn unknown(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(code, message, ExecutionOutcome::Unknown)
    }
}
impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ControlError {}

/// The flattened untagged body serializes exactly one of `result` or `error`.
#[derive(Clone, Debug, Serialize)]
pub struct Response {
    pub id: String,
    #[serde(flatten)]
    pub body: ResponseBody,
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Result { result: Value },
    Error { error: ControlError },
}
impl Response {
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            body: ResponseBody::Result { result },
        }
    }
    pub fn error(id: impl Into<String>, error: ControlError) -> Self {
        Self {
            id: id.into(),
            body: ResponseBody::Error { error },
        }
    }
    pub fn result(&self) -> Option<&Value> {
        match &self.body {
            ResponseBody::Result { result } => Some(result),
            _ => None,
        }
    }
    pub fn error_ref(&self) -> Option<&ControlError> {
        match &self.body {
            ResponseBody::Error { error } => Some(error),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // An enum keeps JSON null as a present result and rejects both/neither,
        // unknown fields and duplicate fields without ad-hoc string parsing.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OkWire {
            id: String,
            result: Value,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ErrWire {
            id: String,
            error: ControlError,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Ok(OkWire),
            Err(ErrWire),
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Ok(value) => Self::success(value.id, value.result),
            Wire::Err(value) => Self::error(value.id, value.error),
        })
    }
}
