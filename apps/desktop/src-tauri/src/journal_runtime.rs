// SPDX-License-Identifier: MPL-2.0
//! Durable public execution observations. Protocol continuation and request
//! bodies remain transient; a receipt never grants permission to execute.
use crate::{ai::ModelToolCall, Host};
use mewu_core::{
    CompletePublicTurn, CoreError, DispatchLease, ModelDispatchInput, NoResponseReason,
    NotSentReason, ProposedTool, ToolBinding, TurnCommit,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Manager};

pub const MAX_TOOL_OUTPUT: usize = 128 * 1024;

pub struct CommittedToolOutput {
    pub model_json: String,
}

/// Call only after the real NotSent receipt is durable and the host has passed
/// its current live/exit/permission fences. This is model feedback, never an
/// execution result or an automatic retry. Unknown outcomes cannot enter here.
pub(crate) fn not_sent_feedback(reason: NotSentReason) -> Option<CommittedToolOutput> {
    let (code, message, correctable, correction) = match reason {
        NotSentReason::InvalidArguments => (
            "invalid_arguments",
            "工具参数不符合已提供的参数规范，本次未调用工具。",
            true,
            "请根据已提供的工具 schema 修正参数后再决定是否调用；不要原样重复无效参数。",
        ),
        NotSentReason::PreparationFailed => (
            "preparation_failed",
            "工具在调用前准备失败，本次未调用工具。",
            false,
            "可以在本轮既有预算内决定重试，或改用其他已提供的工具；持续失败时向用户说明，不得声称已执行成功。",
        ),
        NotSentReason::AuthorityChanged | NotSentReason::CatalogChanged => (
            "tool_unavailable",
            "该工具的授权或目录已变化，本次未执行。",
            false,
            "不要重复调用这个失效工具或尝试恢复授权。可以使用其他仍获授权的工具，或直接完成无需该工具的回答；确实依赖它时说明限制。",
        ),
        _ => return None,
    };
    Some(CommittedToolOutput {
        model_json: serde_json::json!({
            "isError":true,"error":message,"code":code,"executed":false,
            "correctable":correctable,"retryable":!matches!(reason, NotSentReason::AuthorityChanged | NotSentReason::CatalogChanged),"correction":correction
        })
        .to_string(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Changed<'a> {
    scene_id: &'a str,
    run_id: &'a str,
    revision: u64,
}

pub fn emit_changed(app: &AppHandle, scene: &str, run: &str, revision: u64) {
    // UI delivery failure cannot roll back an already committed receipt.
    let _ = app.emit_to(
        "space",
        "run-journal-changed",
        Changed {
            scene_id: scene,
            run_id: run,
            revision,
        },
    );
}

pub fn store_error(error: CoreError) -> String {
    match error {
        CoreError::Database(_) | CoreError::Json(_) => "无法保存或读取执行记录".into(),
        error => error.to_string(),
    }
}

pub struct RunJournal {
    app: AppHandle,
    scene_id: String,
    run_id: String,
    bindings: BTreeMap<String, ToolBinding>,
    plugin_origin: Option<(String, String, u64)>,
    visual_origin: Option<crate::visual_annotation_host::RunOrigin>,
}

impl RunJournal {
    pub fn new(
        app: AppHandle,
        scene_id: String,
        run_id: String,
        bindings: BTreeMap<String, ToolBinding>,
        plugin_origin: Option<(String, String, u64)>,
    ) -> Self {
        Self {
            app,
            scene_id,
            run_id,
            bindings,
            plugin_origin,
            visual_origin: None,
        }
    }

    pub fn with_visual(mut self, origin: Option<crate::visual_annotation_host::RunOrigin>) -> Self {
        self.visual_origin = origin;
        self
    }

    pub fn active(&self) -> Result<(), String> {
        let host = self.app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.require_agent()?;
        if let Some(origin) = &self.visual_origin {
            plugins.store.visual_annotations(&origin.grant)?;
        }
        let engine = host.lock()?;
        if self
            .visual_origin
            .as_ref()
            .is_some_and(|origin| engine.visual_runs.get(&self.run_id) != Some(origin))
        {
            return Err("原位作答已取消或插件已停用".into());
        }
        host.exit.ensure_background()?;
        if !crate::run_is_authorized(
            &engine,
            &self.scene_id,
            &self.run_id,
            self.plugin_origin.as_ref(),
        ) {
            return Err("运行已取消或结束".into());
        }
        Ok(())
    }

    /// Invoked after HTTP Request construction and immediately before execute.
    /// Hash identifies this request, not a durable copy of its private body.
    pub fn begin_model(&self, round: u32, request_sha256: String) -> Result<DispatchLease, String> {
        let host = self.app.state::<Host>();
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.require_agent()?;
        if let Some(origin) = &self.visual_origin {
            plugins.store.visual_annotations(&origin.grant)?;
        }
        let mut engine = host.lock()?;
        if self
            .visual_origin
            .as_ref()
            .is_some_and(|origin| engine.visual_runs.get(&self.run_id) != Some(origin))
        {
            return Err("原位作答已取消或插件已停用".into());
        }
        host.exit.ensure_background()?;
        if !crate::run_is_authorized(
            &engine,
            &self.scene_id,
            &self.run_id,
            self.plugin_origin.as_ref(),
        ) {
            return Err("运行已取消或结束".into());
        }
        let lease = engine
            .store
            .begin_model_request(
                &self.scene_id,
                &self.run_id,
                ModelDispatchInput {
                    round,
                    projected_request_sha256: request_sha256,
                },
            )
            .map_err(store_error)?;
        if let Some(summary) = engine
            .store
            .run_journal_summary(&self.scene_id, &self.run_id)
            .map_err(store_error)?
        {
            emit_changed(&self.app, &self.scene_id, &self.run_id, summary.revision);
        }
        Ok(lease)
    }

    pub fn complete(
        &self,
        lease: &DispatchLease,
        text: &str,
        calls: &[ModelToolCall],
    ) -> Result<TurnCommit, String> {
        let proposed = calls
            .iter()
            .map(|call| ProposedTool {
                call_id: call.id.clone(),
                binding: self.bindings.get(&call.name).cloned().unwrap_or_else(|| {
                    ToolBinding::Rejected {
                        advertised_name: call.name.clone(),
                    }
                }),
                arguments_wire_sha256: format!("{:x}", Sha256::digest(call.arguments.as_bytes())),
            })
            .collect();
        let host = self.app.state::<Host>();
        let mut engine = host.lock()?;
        // Exact late observations are auditable, even after native revocation.
        // The caller checks active() before any further tool/model request.
        let commit = engine
            .store
            .record_model_turn(
                lease,
                CompletePublicTurn {
                    text: text.to_owned(),
                    calls: proposed,
                },
            )
            .map_err(store_error)?;
        let arguments = commit
            .tools
            .iter()
            .zip(calls)
            .map(|(prepared, call)| (prepared.lease.clone(), call.arguments.clone()))
            .collect::<Vec<_>>();
        engine
            .store
            .retain_tool_arguments(&arguments)
            .map_err(store_error)?;
        emit_changed(
            &self.app,
            &self.scene_id,
            &self.run_id,
            commit.journal_revision,
        );
        if !commit.tools.is_empty() {
            crate::publish(&self.app, &engine.store.snapshot());
        }
        Ok(commit)
    }

    pub fn unknown(&self, lease: &DispatchLease, reason: NoResponseReason) -> Result<(), String> {
        let host = self.app.state::<Host>();
        let mut engine = host.lock()?;
        let commit = engine
            .store
            .record_request_unknown(lease, reason)
            .map_err(store_error)?;
        emit_changed(
            &self.app,
            &self.scene_id,
            &self.run_id,
            commit.journal_revision,
        );
        Ok(())
    }

    pub fn not_sent(&self, lease: &DispatchLease, reason: NotSentReason) -> Result<(), String> {
        let host = self.app.state::<Host>();
        let mut engine = host.lock()?;
        let commit = engine
            .store
            .record_tool_not_sent(lease, reason)
            .map_err(store_error)?;
        emit_changed(
            &self.app,
            &self.scene_id,
            &self.run_id,
            commit.journal_revision,
        );
        crate::publish(&self.app, &engine.store.snapshot());
        Ok(())
    }
}
