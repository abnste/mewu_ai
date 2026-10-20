// SPDX-License-Identifier: MPL-2.0
//! Only measured placement failures are correctable. Renderer, live-fence and
//! storage failures stay hard; no error message is inspected to classify them.
use crate::journal_runtime::{self, CommittedToolOutput};
use mewu_core::{DispatchLease, NotSentReason, ReceiptCommit, Store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    ImageBoundsTooSmall,
    VideoTextOutsideFrame,
}

#[derive(Debug)]
pub(crate) enum Failure {
    Correctable(Rejection),
    Hard(String),
}
impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self::Hard(error)
    }
}
impl From<&str> for Failure {
    fn from(error: &str) -> Self {
        Self::Hard(error.into())
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Hard(error) => error,
            Self::Correctable(Rejection::ImageBoundsTooSmall) => {
                "可用范围太小，无法完整显示表格或公式"
            }
            Self::Correctable(Rejection::VideoTextOutsideFrame) => "视频文字布局超出原视频",
        })
    }
}

/// The caller must fence before this operation and again before delivering the
/// feedback. A failed durable receipt never yields a model-visible rejection.
pub(crate) fn reject(
    store: &mut Store,
    lease: &DispatchLease,
    rejection: Rejection,
) -> Result<(ReceiptCommit, CommittedToolOutput), String> {
    let receipt = store
        .record_tool_not_sent(lease, NotSentReason::InvalidArguments)
        .map_err(journal_runtime::store_error)?;
    let base = journal_runtime::not_sent_feedback(NotSentReason::InvalidArguments)
        .ok_or("批注参数反馈不可用")?;
    let mut body: serde_json::Value =
        serde_json::from_str(&base.model_json).map_err(|_| "批注参数反馈无效")?;
    let (code, message, correction) = match rejection {
        Rejection::ImageBoundsTooSmall => (
            "annotation_bounds_too_small",
            "可用范围太小，无法完整显示表格或公式，本次未提交任何批注。",
            "请扩大该表格或公式的两个 points 所定义的可用范围，或精简内容后重新调用；保持原 target 与坐标单位，不得声称已经绘制。",
        ),
        Rejection::VideoTextOutsideFrame => (
            "video_text_outside_frame",
            "实测文字布局超出原视频边界，本次未提交任何批注。",
            "请将文字 topLeft 向左或向上移动，或精简文字后重新调用，确保完整文字位于视频内；保持原 targetHandle 与单位，不得声称已经绘制。",
        ),
    };
    body["code"] = code.into();
    body["error"] = message.into();
    body["correction"] = correction.into();
    Ok((
        receipt,
        CommittedToolOutput {
            model_json: body.to_string(),
        },
    ))
}
