// SPDX-License-Identifier: MPL-2.0
//! Pure policies: no UI, network, credentials, memory writes or tool execution.
//! Inspired by Pi v1.4.0's projection/compaction boundaries; independently implemented.
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const DEFAULT_CONTEXT_TOKENS: usize = 65_536;
pub const MAX_SUMMARY_BYTES: usize = 12 * 1024;
pub const MAX_CONTEXT_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_CONTEXT_DOCUMENT_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ContextPolicy {
    pub window: usize,
    pub reserve: usize,
    pub recent: usize,
}
impl ContextPolicy {
    pub fn new(window: Option<u32>) -> Result<Self, String> {
        let window = window.map(|n| n as usize).unwrap_or(DEFAULT_CONTEXT_TOKENS);
        if !(8_192..=2_097_152).contains(&window) {
            return Err("上下文窗口须为 8192–2097152 tokens".into());
        }
        Ok(Self {
            window,
            reserve: (window / 4).min(16_384),
            recent: (window / 3).min(20_000),
        })
    }
    pub fn input_limit(self) -> usize {
        self.window - self.reserve
    }
    pub fn summary_limit(self) -> usize {
        (self.window / 8).min(MAX_SUMMARY_BYTES)
    }
}

/// A deliberately conservative heuristic, not provider billing/tokenizer output.
/// ASCII uses three bytes/token; CJK uses two tokens/character, supplementary
/// characters three. Unlike dividing UTF-8 length by four this budgets Chinese.
pub fn estimate_text(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut other = 0usize;
    for c in text.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += if c.len_utf8() == 4 { 3 } else { 2 };
        }
    }
    ascii.div_ceil(3) + other
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    /// Descriptors only. Summarization never claims to have inspected these pixels/files.
    pub attachments: Vec<String>,
    pub outcomes: Vec<String>,
    #[serde(default)]
    pub image_bytes: u64,
    #[serde(default)]
    pub document_bytes: u64,
}
impl SessionMessage {
    pub fn estimated_tokens(&self) -> usize {
        estimate_text(&self.text)
            + self.attachments.len() * 4096
            + self
                .outcomes
                .iter()
                .map(|s| estimate_text(s))
                .sum::<usize>()
            + 32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextCheckpoint {
    pub scope: String,
    pub prefix_hash: String,
    pub message_count: usize,
    pub summary: String,
}
impl ContextCheckpoint {
    pub fn matches(&self, scope: &str, messages: &[SessionMessage]) -> bool {
        self.scope == scope
            && self.message_count > 0
            && self.message_count < messages.len()
            && !self.summary.trim().is_empty()
            && self.summary.len() <= MAX_SUMMARY_BYTES
            && self.prefix_hash == fingerprint(&messages[..self.message_count])
            && messages[self.message_count].role == "user"
    }
}
pub fn fingerprint(value: &(impl Serialize + ?Sized)) -> String {
    // These types contain no floats/non-string map keys; serialization cannot fail.
    let bytes = serde_json::to_vec(value).expect("serializable context value");
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPlan {
    pub first_kept: usize,
    pub summarize_from: usize,
    pub previous_summary: Option<String>,
}

/// Cut only before a user turn. The current request is never summarized and a
/// completed user/assistant exchange is never split across the boundary.
pub fn plan(
    policy: ContextPolicy,
    scope: &str,
    messages: &[SessionMessage],
    fixed_tokens: usize,
    checkpoint: Option<&ContextCheckpoint>,
) -> Result<ContextPlan, String> {
    if messages.last().is_none_or(|m| m.role != "user") {
        return Err("上下文缺少当前用户消息".into());
    }
    let current = messages.last().unwrap();
    if current.image_bytes > MAX_CONTEXT_IMAGE_BYTES
        || current.document_bytes > MAX_CONTEXT_DOCUMENT_BYTES
    {
        return Err("本次引用文件不可用或超过输入大小限制，不能通过压缩历史省略本次输入".into());
    }
    let checkpoint = checkpoint.filter(|c| c.matches(scope, messages));
    let from = checkpoint.map_or(0, |c| c.message_count);
    let summary = checkpoint.map(|c| c.summary.clone());
    let cost = fixed_tokens
        .saturating_add(summary.as_deref().map_or(0, estimate_text))
        .saturating_add(
            messages[from..]
                .iter()
                .map(SessionMessage::estimated_tokens)
                .sum::<usize>(),
        );
    let (images, documents) = messages[from..].iter().fold((0u64, 0u64), |(i, d), m| {
        (
            i.saturating_add(m.image_bytes),
            d.saturating_add(m.document_bytes),
        )
    });
    if cost <= policy.input_limit()
        && images <= MAX_CONTEXT_IMAGE_BYTES
        && documents <= MAX_CONTEXT_DOCUMENT_BYTES
    {
        return Ok(ContextPlan {
            first_kept: from,
            summarize_from: from,
            previous_summary: summary,
        });
    }
    let available = policy
        .input_limit()
        .saturating_sub(fixed_tokens)
        .saturating_sub(policy.summary_limit());
    let current_cost = current.estimated_tokens();
    if current_cost > available {
        return Err("当前输入与必要上下文超过预算，请减少附件或提高连接的上下文窗口".into());
    }
    // The recent-history target is not a separate cap on the user's current
    // request. A large current message may consume the available window while
    // older turns are summarized; it must neither be dropped nor rejected just
    // because it exceeds our preferred history tail size.
    let keep_budget = policy.recent.max(current_cost).min(available);
    let mut kept_cost = 0usize;
    let mut images = 0u64;
    let mut documents = 0u64;
    let mut cut = messages.len() - 1;
    for (index, message) in messages.iter().enumerate().skip(from).rev() {
        kept_cost = kept_cost.saturating_add(message.estimated_tokens());
        images = images.saturating_add(message.image_bytes);
        documents = documents.saturating_add(message.document_bytes);
        if kept_cost > keep_budget
            || images > MAX_CONTEXT_IMAGE_BYTES
            || documents > MAX_CONTEXT_DOCUMENT_BYTES
        {
            break;
        }
        if message.role == "user" {
            cut = index;
        }
    }
    if cut <= from {
        return Err(
            "当前消息、系统提示或工具定义已超过上下文预算，请减少本次输入或提高连接的上下文窗口"
                .into(),
        );
    }
    Ok(ContextPlan {
        first_kept: cut,
        summarize_from: from,
        previous_summary: summary,
    })
}

/// A bounded, explicit projection of an *observed* tool result. It never says
/// execution failed just because its result did not fit, and never asks to replay
/// a side effect. The caller keeps the original receipt in the durable journal.
pub fn project_tool_output(content: String, limit: usize) -> String {
    if content.len() <= limit {
        return content;
    }
    json!({"type":"tool_result_omitted", "observed":true, "outputOmitted":true,
        "originalBytes":content.len(), "sha256":format!("{:x}",Sha256::digest(content.as_bytes())),
        "message":"The observed result is too large for model context. Receipt metadata is retained; full output may be unavailable. Do not repeat a side effect; use a narrower read/query if supported. Execution success is not inferred from this envelope."}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    fn m(i: usize, role: &str, n: usize) -> SessionMessage {
        SessionMessage {
            id: i.to_string(),
            role: role.into(),
            text: "界".repeat(n),
            attachments: vec![],
            outcomes: vec![],
            image_bytes: 0,
            document_bytes: 0,
        }
    }
    #[test]
    fn keeps_current_and_whole_recent_turns_without_mutating_history() {
        let history = vec![
            m(0, "user", 3000),
            m(1, "assistant", 1000),
            m(2, "user", 20),
            m(3, "assistant", 20),
            m(4, "user", 20),
        ];
        let original = history.clone();
        let p = plan(
            ContextPolicy::new(Some(8192)).unwrap(),
            "s",
            &history,
            200,
            None,
        )
        .unwrap();
        assert_eq!(p.first_kept, 2);
        assert_eq!(history, original);
    }
    #[test]
    fn checkpoint_requires_exact_prefix_and_scope_and_survives_append() {
        let mut h = vec![m(0, "user", 200), m(1, "assistant", 200), m(2, "user", 2)];
        let c = ContextCheckpoint {
            scope: "a/model/policy".into(),
            prefix_hash: fingerprint(&h[..2]),
            message_count: 2,
            summary: "goal".into(),
        };
        assert!(c.matches("a/model/policy", &h));
        h.push(m(3, "assistant", 20));
        h.push(m(4, "user", 20));
        assert!(c.matches("a/model/policy", &h));
        assert!(!c.matches("b/model/policy", &h));
        h[0].text = "edited".into();
        assert!(!c.matches("a/model/policy", &h));
    }
    #[test]
    fn summaries_are_not_accepted_at_assistant_boundaries() {
        let h = vec![m(0, "user", 2), m(1, "assistant", 2), m(2, "user", 2)];
        let c = ContextCheckpoint {
            scope: "s".into(),
            prefix_hash: fingerprint(&h[..1]),
            message_count: 1,
            summary: "summary".into(),
        };
        assert!(!c.matches("s", &h));
    }
    #[test]
    fn oversized_current_message_is_not_silently_discarded() {
        let h = vec![m(0, "user", 9000)];
        assert!(plan(ContextPolicy::new(Some(8192)).unwrap(), "s", &h, 100, None).is_err());
    }
    #[test]
    fn current_message_can_exceed_recent_history_target_without_exceeding_window() {
        let policy = ContextPolicy::new(Some(8192)).unwrap();
        let h = vec![
            m(0, "user", 2000),
            m(1, "assistant", 1000),
            m(2, "user", 1900),
        ];
        assert!(h[2].estimated_tokens() > policy.recent);
        let p = plan(policy, "s", &h, 200, None).unwrap();
        assert_eq!(p.first_kept, 2);
        assert_eq!(p.summarize_from, 0);
    }
    #[test]
    fn old_media_can_be_summarized_but_current_media_is_never_dropped() {
        let mut h = vec![m(0, "user", 20), m(1, "assistant", 20), m(2, "user", 20)];
        h[0].image_bytes = MAX_CONTEXT_IMAGE_BYTES;
        h[2].image_bytes = 2000;
        let p = plan(ContextPolicy::new(None).unwrap(), "s", &h, 200, None).unwrap();
        assert_eq!(p.first_kept, 2);
        h[2].image_bytes = u64::MAX;
        assert!(plan(ContextPolicy::new(None).unwrap(), "s", &h, 200, None).is_err());
        h[2].image_bytes = 0;
        h[0].document_bytes = MAX_CONTEXT_DOCUMENT_BYTES + 1;
        assert_eq!(
            plan(ContextPolicy::new(None).unwrap(), "s", &h, 200, None)
                .unwrap()
                .first_kept,
            2
        );
    }
    #[test]
    fn tool_projection_preserves_small_errors_and_never_requests_reexecution() {
        let small = json!({"isError":true,"retryable":true,"error":"bad arguments"}).to_string();
        assert_eq!(project_tool_output(small.clone(), 1024), small);
        let projected = project_tool_output("secret large result".repeat(500), 1024);
        let v: Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(v["observed"], true);
        assert_eq!(v["outputOmitted"], true);
        assert!(v.get("executed").is_none());
        assert!(v.get("retryable").is_none());
        assert!(!projected.contains("secret large result"));
    }
}
