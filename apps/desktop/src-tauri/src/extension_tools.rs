// SPDX-License-Identifier: MPL-2.0
//! API 2 model tools share the normal Agent journal and tool loop. A descriptor
//! is a frozen catalog entry, never authority to execute an installed package.
use crate::{ai, extension_chat_region::ChatRegionScope, journal_runtime, Host};
use mewu_core::{DispatchLease, NoResponseReason, NotSentReason, ToolBinding};
use mewu_extension::{ServiceKind, ServiceRegistration};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolDescriptor {
    pub plugin_id: String,
    pub revision: u64,
    pub version: String,
    pub sha256: String,
    pub service: ServiceRegistration,
}

pub(crate) enum ToolInvocationOutcome {
    NotSent(NotSentReason),
    Unknown {
        reason: NoResponseReason,
        safe_diagnostic: Option<crate::extension_host::SafeToolDiagnostic>,
    },
    Responded(Value),
}
impl ToolInvocationOutcome {
    pub(crate) fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Unknown {
                safe_diagnostic, ..
            } => Some(
                safe_diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.message().to_owned())
                    .unwrap_or_else(|| "扩展工具执行未确认 [TOOL_RESULT_UNKNOWN]".into()),
            ),
            _ => None,
        }
    }
}

impl ToolDescriptor {
    pub(crate) fn alias(&self) -> String {
        mewu_core::extension_tool_alias(&self.plugin_id, &self.service.id)
    }
    pub(crate) fn binding(&self) -> ToolBinding {
        ToolBinding::Extension {
            plugin_id: self.plugin_id.clone(),
            plugin_revision: self.revision,
            version: self.version.clone(),
            sha256: self.sha256.clone(),
            service_id: self.service.id.clone(),
            advertised_alias: self.alias(),
            name: self.service.title.clone(),
        }
    }
    pub(crate) fn requires_region(&self) -> Result<bool, String> {
        match self
            .service
            .metadata
            .as_ref()
            .and_then(|m| m.get("requiresRegion"))
        {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            _ => Err("扩展工具的选区声明无效".into()),
        }
    }
    fn schema(&self) -> Value {
        self.service.input_schema.clone().unwrap_or_else(|| {
            json!({
                "type":"object", "properties":{}, "additionalProperties":false
            })
        })
    }
    fn definition(&self) -> Result<Value, String> {
        if self.service.kind != ServiceKind::Tool || self.revision == 0 {
            return Err("扩展工具目录无效".into());
        }
        let schema = self.schema();
        fn bounded(value: &Value, depth: usize) -> bool {
            depth <= 32
                && match value {
                    Value::Array(values) => values.iter().all(|v| bounded(v, depth + 1)),
                    Value::Object(values) => values.values().all(|v| bounded(v, depth + 1)),
                    _ => true,
                }
        }
        if schema.get("type").and_then(Value::as_str) != Some("object")
            || !bounded(&schema, 0)
            || serde_json::to_vec(&schema)
                .map_err(|_| "扩展工具参数规范无效")?
                .len()
                > 16 * 1024
            || jsonschema::validator_for(&schema).is_err()
        {
            return Err("扩展工具参数规范无效".into());
        }
        Ok(json!({"type":"function", "function":{
            "name":self.alias(),
            "description":format!("{} / {}\n{}", self.plugin_id, self.service.title,
                self.service.description.as_deref().unwrap_or("")),
            "parameters":schema
        }}))
    }
    fn arguments(&self, wire: &str) -> Result<Value, NotSentReason> {
        if wire.len() > 16 * 1024 {
            return Err(NotSentReason::InvalidArguments);
        }
        let input: Value =
            serde_json::from_str(wire).map_err(|_| NotSentReason::InvalidArguments)?;
        if !input.is_object() {
            return Err(NotSentReason::InvalidArguments);
        }
        let validator =
            jsonschema::validator_for(&self.schema()).map_err(|_| NotSentReason::CatalogChanged)?;
        if !validator.is_valid(&input) {
            return Err(NotSentReason::InvalidArguments);
        }
        Ok(input)
    }
}

/// Deterministic construction is independent from installed-store locks and
/// runs. Both runtime and preflight use the same count/byte budget.
pub(crate) fn append_catalog(
    descriptors: Vec<ToolDescriptor>,
    has_region: bool,
    definitions: &mut Vec<Value>,
) -> Result<BTreeMap<String, ToolDescriptor>, String> {
    let mut bindings = BTreeMap::new();
    let mut additions = Vec::new();
    for descriptor in descriptors {
        if descriptor.requires_region()? && !has_region {
            continue;
        }
        if definitions.len() + additions.len() >= 64 {
            return Err("已启用的工具过多或定义过长，请减少所选工具".into());
        }
        let definition = descriptor.definition()?;
        if definitions.iter().any(|d| {
            d.pointer("/function/name").and_then(Value::as_str) == Some(&descriptor.alias())
        }) || bindings.insert(descriptor.alias(), descriptor).is_some()
        {
            return Err("扩展工具标识重复".into());
        }
        additions.push(definition);
    }
    let mut combined = definitions.clone();
    combined.extend(additions);
    if combined.len() > 64
        || serde_json::to_vec(&combined)
            .map_err(|_| "工具目录无效")?
            .len()
            > 128 * 1024
    {
        return Err("已启用的工具过多或定义过长，请减少所选工具".into());
    }
    *definitions = combined;
    Ok(bindings)
}

pub(crate) async fn execute_recorded(
    app: &AppHandle,
    scene_id: &str,
    run_id: &str,
    descriptor: &ToolDescriptor,
    canceled: Arc<AtomicBool>,
    region: Option<Arc<ChatRegionScope>>,
    call: ai::ModelToolCall,
    lease: DispatchLease,
) -> Result<journal_runtime::CommittedToolOutput, String> {
    let outcome = if canceled.load(Ordering::SeqCst) {
        ToolInvocationOutcome::NotSent(NotSentReason::RunEnded)
    } else {
        match descriptor.arguments(&call.arguments) {
            Ok(input) => {
                crate::extension_host::invoke_tool_for_run(
                    app.clone(),
                    descriptor.clone(),
                    scene_id.into(),
                    run_id.into(),
                    canceled.clone(),
                    region,
                    input,
                    lease.clone(),
                )
                .await
            }
            Err(reason) => ToolInvocationOutcome::NotSent(reason),
        }
    };
    let observation = match outcome {
        ToolInvocationOutcome::Responded(value) => {
            let body = serde_json::to_string(&value).map_err(|_| "扩展工具结果无效")?;
            if body.len() > journal_runtime::MAX_TOOL_OUTPUT {
                Err(ToolInvocationOutcome::Unknown {
                    reason: NoResponseReason::ProtocolInvalid,
                    safe_diagnostic: None,
                })
            } else {
                Ok(mewu_core::ObservedToolResult {
                    response_kind: mewu_core::ToolResponseKind::Complete,
                    returned_error: Some(
                        value
                            .get("isError")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    ),
                    observed_encoding: "mewu-extension-json-v1".into(),
                    observed_bytes: body.len() as u64,
                    observed_sha256: format!("{:x}", Sha256::digest(body.as_bytes())),
                    retained_content: mewu_core::JournalContent::Json { value },
                    references: vec![],
                    model_visible_json: Some(body),
                })
            }
        }
        other => Err(other),
    };
    let host = app.state::<Host>();
    let mut engine = host.lock()?;
    // Commit exact late observations before checking whether another operation
    // is allowed. Canceling does not erase a completed external side effect.
    // A public diagnostic is not a confirmed tool reply. Commit the real
    // Unknown receipt before surfacing it and never feed it back for a retry.
    let stop_diagnostic = observation
        .as_ref()
        .err()
        .and_then(ToolInvocationOutcome::diagnostic);
    let (commit, content) = match observation {
        Ok(result) => {
            let content = result.model_visible_json.clone();
            (
                engine
                    .store
                    .record_tool_response(&lease, result)
                    .map_err(journal_runtime::store_error)?,
                content,
            )
        }
        Err(ToolInvocationOutcome::NotSent(reason)) => (
            engine
                .store
                .record_tool_not_sent(&lease, reason)
                .map_err(journal_runtime::store_error)?,
            None,
        ),
        Err(ToolInvocationOutcome::Unknown { reason, .. }) => (
            engine
                .store
                .record_request_unknown(&lease, reason)
                .map_err(journal_runtime::store_error)?,
            None,
        ),
        Err(ToolInvocationOutcome::Responded(_)) => unreachable!(),
    };
    journal_runtime::emit_changed(app, scene_id, run_id, commit.journal_revision);
    crate::publish(app, &engine.store.snapshot());
    host.exit.ensure_background()?;
    if canceled.load(Ordering::SeqCst) || !crate::run_is_authorized(&engine, scene_id, run_id, None)
    {
        return Err("运行已取消或结束".into());
    }
    content
        .map(|model_json| journal_runtime::CommittedToolOutput { model_json })
        .ok_or_else(|| stop_diagnostic.unwrap_or_else(|| "扩展工具处理已停止".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_worker_diagnostic_keeps_an_explicit_unknown_stage_without_a_fake_reply() {
        let outcome = ToolInvocationOutcome::Unknown {
            reason: NoResponseReason::IncompleteResponse,
            safe_diagnostic: None,
        };
        assert_eq!(
            outcome.diagnostic().as_deref(),
            Some("扩展工具执行未确认 [TOOL_RESULT_UNKNOWN]")
        );
        assert!(matches!(
            outcome,
            ToolInvocationOutcome::Unknown {
                reason: NoResponseReason::IncompleteResponse,
                ..
            }
        ));
        assert!(
            ToolInvocationOutcome::Responded(json!({"content":"own result"}))
                .diagnostic()
                .is_none()
        );
    }

    fn descriptor(id: &str, requires_region: bool) -> ToolDescriptor {
        ToolDescriptor {
            plugin_id: "independent.author".into(),
            revision: 4,
            version: "0.1.1".into(),
            sha256: "a".repeat(64),
            service: ServiceRegistration {
                kind: ServiceKind::Tool,
                id: id.into(),
                title: format!("Tool {id}"),
                description: Some("An independently registered tool".into()),
                input_schema: Some(
                    json!({"type":"object", "properties":{"task":{"type":"string","minLength":1}}, "required":["task"], "additionalProperties":false}),
                ),
                metadata: Some(json!({"requiresRegion":requires_region})),
            },
        }
    }

    #[test]
    fn dynamic_extension_catalog_preserves_other_tools_and_filters_only_explicit_region_tools() {
        let base = json!({"type":"function","function":{"name":"existing_memory","parameters":{"type":"object"}}});
        let mut definitions = vec![base.clone()];
        let plain = descriptor("inspect.document", false);
        let region = descriptor("operate.region", true);
        let bindings =
            append_catalog(vec![plain.clone(), region.clone()], false, &mut definitions).unwrap();
        assert_eq!(definitions[0], base);
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            definitions[1]["function"]["description"],
            "independent.author / Tool inspect.document\nAn independently registered tool"
        );
        assert_eq!(
            definitions[1]["function"]["parameters"],
            plain.service.input_schema.clone().unwrap()
        );
        assert!(matches!(
            bindings[&plain.alias()].binding(),
            ToolBinding::Extension {
                plugin_revision: 4,
                ..
            }
        ));
        let mut authorized = vec![];
        let bindings =
            append_catalog(vec![region.clone(), plain.clone()], true, &mut authorized).unwrap();
        assert_eq!(bindings.len(), 2);
        assert!(bindings.contains_key(&region.alias()));
        let mut changed = plain.clone();
        changed.revision += 1;
        changed.version = "0.1.2".into();
        changed.sha256 = "b".repeat(64);
        assert_eq!(plain.alias(), changed.alias());
        assert_ne!(plain.binding(), changed.binding());
        assert_eq!(plain.alias().len(), 64);
        assert!(plain
            .alias()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        assert_ne!(
            descriptor("first", false).alias(),
            descriptor("second", false).alias()
        );
    }

    #[test]
    fn extension_catalog_limits_and_collisions_do_not_mutate_existing_catalog() {
        let base = json!({"type":"function","function":{"name":"existing","parameters":{"type":"object"}}});
        for scenario in [
            "count",
            "bytes",
            "duplicate",
            "alias",
            "schema",
            "remote-schema",
            "region-flag",
        ] {
            let mut descriptors: Vec<_> = (0..3)
                .map(|n| descriptor(&format!("tool-{n}"), false))
                .collect();
            let mut definitions = vec![base.clone()];
            match scenario {
                "count" => descriptors = (0..64).map(|n| descriptor(&format!("tool-{n}"), false)).collect(),
                "bytes" => { descriptors = (0..40).map(|n| descriptor(&format!("tool-{n}"), false)).collect(); for tool in &mut descriptors { tool.service.description=Some("x".repeat(4000)); } }
                "duplicate" => descriptors.push(descriptors[0].clone()),
                "alias" => definitions.push(json!({"type":"function","function":{"name":descriptors[0].alias(),"parameters":{"type":"object"}}})),
                "schema" => descriptors[0].service.input_schema=Some(json!({"type":"array"})),
                "remote-schema" => descriptors[0].service.input_schema=Some(json!({"type":"object","properties":{"value":{"$ref":"https://example.invalid/tool-schema.json"}}})),
                _ => descriptors[0].service.metadata=Some(json!({"requiresRegion":"yes"})),
            }
            let original = definitions.clone();
            assert!(
                append_catalog(descriptors, false, &mut definitions).is_err(),
                "{scenario}"
            );
            assert_eq!(definitions, original, "{scenario}");
        }
    }

    #[test]
    fn extension_tool_arguments_follow_the_registered_schema_and_wire_budget() {
        let tool = descriptor("inspect", false);
        assert_eq!(
            tool.arguments(r#"{"task":"draw"}"#).unwrap(),
            json!({"task":"draw"})
        );
        for wire in [
            "[1]",
            "{",
            "{}",
            r#"{"task":""}"#,
            r#"{"task":"draw","extra":true}"#,
        ] {
            assert!(matches!(
                tool.arguments(wire),
                Err(NotSentReason::InvalidArguments)
            ));
        }
        let oversized = serde_json::to_string(&json!({"task":"x".repeat(16*1024)})).unwrap();
        assert!(matches!(
            tool.arguments(&oversized),
            Err(NotSentReason::InvalidArguments)
        ));
        let mut no_arguments = tool;
        no_arguments.service.input_schema = None;
        assert!(no_arguments.definition().is_ok());
        assert_eq!(no_arguments.arguments("{}").unwrap(), json!({}));
        assert!(no_arguments.arguments(r#"{"unexpected":true}"#).is_err());
    }
}
