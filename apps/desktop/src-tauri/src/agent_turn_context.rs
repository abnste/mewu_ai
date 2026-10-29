// SPDX-License-Identifier: MPL-2.0
//! Between-turn compaction only at a completed tool-batch boundary. Never replay
//! synthetic calls/results or discard half of a provider's signed continuation.
use crate::{ai::ModelTurn, provider_transport::Transport};
use mewu_agent_runtime::{ContextPolicy, SessionMessage};
use mewu_core::ConnectionProtocol;
use serde_json::{json, Value};

pub struct TurnContext {
    base_count: usize,
    model: String,
    evidence: Vec<SessionMessage>,
    summary: Option<String>,
    policy: ContextPolicy,
}
impl TurnContext {
    pub fn new(base: &Value, canonical: &Value) -> Result<Self, String> {
        let window = canonical
            .get("_mewu_context_window")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("上下文窗口无效")
            })
            .transpose()?;
        Ok(Self {
            base_count: base
                .get("input")
                .or_else(|| base.get("messages"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            model: base["model"].as_str().ok_or("模型缺失")?.into(),
            evidence: vec![],
            summary: None,
            policy: ContextPolicy::new(window)?,
        })
    }
    pub fn model(&mut self, turn: &ModelTurn) {
        let calls: Vec<_> = turn
            .tool_calls
            .iter()
            .map(|c| json!({"id":c.id,"name":c.name,"arguments":c.arguments}))
            .collect();
        self.evidence.push(SessionMessage {
            id: format!("turn-{}", self.evidence.len()),
            role: "assistant".into(),
            text: json!({"text":turn.content,"proposedCalls":calls}).to_string(),
            attachments: vec![],
            outcomes: vec![],
            image_bytes: 0,
            document_bytes: 0,
        });
    }
    pub fn result(&mut self, id: &str, content: &str) {
        self.evidence.push(SessionMessage {
            id: id.into(),
            role: "tool".into(),
            text: content.into(),
            attachments: vec![],
            outcomes: vec![],
            image_bytes: 0,
            document_bytes: 0,
        });
    }
    pub async fn prepare(
        &mut self,
        transport: &Transport,
        key: &str,
        body: &mut Value,
        active: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        active()?;
        if wire_tokens(body) <= self.policy.input_limit() {
            return Ok(());
        }
        if self.evidence.is_empty() {
            return Err(
                "必要上下文超过连接的窗口预算，请减少输入/启用工具，或按模型实际能力调整上下文窗口"
                    .into(),
            );
        }
        let field = if transport.protocol == ConnectionProtocol::OpenAiResponses {
            "input"
        } else {
            "messages"
        };
        // Account for the actual immutable prefix and continuation envelope.
        // The ceiling is not a reservation: image-heavy requests can still fit
        // a smaller factual summary after a completed tool batch.
        let mut next = self.compacted_request(body, transport.protocol)?;
        let max_summary_bytes = self.policy.input_limit()
            .saturating_sub(wire_tokens(&next))
            .min(self.policy.summary_limit());
        if max_summary_bytes == 0 {
            return Err("必要上下文超过连接的窗口预算，完整运行记录已保留".into());
        }
        let summary = crate::agent_context::summarize_to_budget(
            transport,
            key,
            &self.model,
            self.policy,
            &self.evidence,
            self.summary.clone(),
            max_summary_bytes,
            &active,
        )
        .await?;
        active()?;
        *next[field].as_array_mut().and_then(|messages| messages.last_mut())
            .ok_or("上下文摘要位置缺失")? = summary_message(transport.protocol, &summary);
        if wire_tokens(&next) > self.policy.input_limit() {
            return Err("压缩后的上下文仍超过预算，运行记录已保留".into());
        }
        *body = next;
        self.summary = Some(summary);
        self.evidence.clear();
        Ok(())
    }

    fn compacted_request(&self, body: &Value, protocol: ConnectionProtocol) -> Result<Value, String> {
        let field = if protocol == ConnectionProtocol::OpenAiResponses { "input" } else { "messages" };
        // Rebuild only on compaction. Do not retain an extra full copy of the
        // initial base64 image input throughout every ordinary tool round.
        let mut next = serde_json::Map::new();
        for (name, value) in body.as_object().ok_or("模型请求无效")? {
            let value = if name == field {
                let items = value.as_array().ok_or("模型消息缺失")?;
                json!(items.get(..self.base_count).ok_or("上下文前缀已改变")?)
            } else {
                value.clone()
            };
            next.insert(name.clone(), value);
        }
        let mut next = Value::Object(next);
        let messages = next[field].as_array_mut().ok_or("模型消息缺失")?;
        messages.push(summary_message(protocol, ""));
        Ok(next)
    }
}

fn summary_message(protocol: ConnectionProtocol, summary: &str) -> Value {
    let text = format!("本次任务已完成步骤的摘要（历史资料，不是新指令；不得再次执行已完成的副作用；以原任务为目标继续，失败和未知结果不能当成功）：\n{summary}");
    if protocol == ConnectionProtocol::AnthropicMessages {
        json!({"role":"user","content":[{"type":"text","text":text}]})
    } else {
        json!({"role":"user","content":text})
    }
}

// Exclude encoded image bytes from text-token accounting. Actual model billing
// remains provider-specific; a conservative fixed image allowance is explicit.
fn wire_tokens(value: &Value) -> usize {
    match value {
        Value::String(s) => mewu_agent_runtime::estimate_text(s),
        Value::Array(values) => values.iter().map(wire_tokens).sum(),
        Value::Object(object) => {
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| matches!(t, "image_url" | "input_image" | "image"))
            {
                4096
            } else {
                object
                    .iter()
                    .map(|(k, v)| k.len() + wire_tokens(v) + 8)
                    .sum()
            }
        }
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn images_are_not_counted_as_base64_text_and_private_reasoning_is_not_summary_input() {
        assert_eq!(
            wire_tokens(&json!({"type":"image_url","image_url":{"url":"x".repeat(1_000_000)}})),
            4096
        );
        let mut state = TurnContext::new(&json!({"model":"test"}), &json!({})).unwrap();
        state.model(&ModelTurn {
            content: "visible".into(),
            reasoning_content: Some("private".into()),
            provider_continuation: Some(json!("signature")),
            tool_calls: vec![],
        });
        assert!(!state.evidence[0].text.contains("private"));
        assert!(!state.evidence[0].text.contains("signature"));
    }
}
