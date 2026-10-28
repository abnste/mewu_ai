// SPDX-License-Identifier: MPL-2.0
//! General local tools shared by the native Agent catalog and tool dispatcher.
//! No feature-specific OS query or plugin-authoring workaround is registered.
use crate::{
    ai::ModelToolCall,
    journal_runtime::{self, CommittedToolOutput},
    Host,
};
use mewu_core::{DispatchLease, JournalContent, McpCommand, ObservedToolResult, ToolResponseKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub fn handles(name: &str) -> bool {
    mewu_agent_tools::handles(name)
}
fn authorized(app: &AppHandle, scene: &str, run: &str) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.ensure_background()?;
    let engine = host.lock()?;
    if !crate::run_is_authorized(&engine, scene, run, None) {
        return Err("运行已结束或授权已变化".into());
    }
    Ok(())
}
pub fn directory(root: &std::path::Path, scene: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(scene).map_err(|_| "会话标识无效")?;
    let path = root.join("workspaces").join(scene);
    std::fs::create_dir_all(&path).map_err(|_| "无法创建会话工作目录")?;
    path.canonicalize().map_err(|_| "会话工作目录不可用".into())
}
pub async fn execute(
    app: AppHandle,
    scene: String,
    run: String,
    call: ModelToolCall,
    lease: DispatchLease,
) -> Result<CommittedToolOutput, String> {
    authorized(&app, &scene, &run)?;
    let root = app.state::<Host>().root.clone();
    let cwd = directory(&root, &scene)?;
    let receipt = app
        .state::<Host>()
        .lock()?
        .store
        .mark_tool_dispatched(&lease)
        .map_err(journal_runtime::store_error)?;
    journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
    let canceled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(canceled.clone());
    let mut value = if matches!(call.name.as_str(), "bash" | "powershell") {
        let prepared = (|| {
            let args: mewu_agent_tools::ShellCall = mewu_agent_tools::parse(&call.arguments)?;
            let selected = match &args.cwd {
                Some(path) => mewu_agent_tools::resolve_path(&cwd, path)?,
                None => cwd,
            };
            let (spec, input) = mewu_agent_tools::shell::prepare(&call.name, &args)?;
            Ok::<_, String>((
                McpCommand {
                    executable: spec.executable.to_string_lossy().into_owned(),
                    args: spec.args,
                    cwd: Some(selected.to_string_lossy().into_owned()),
                },
                input,
                args.timeout.map(Duration::from_secs_f64),
            ))
        })();
        match prepared {
            Err(error) => json!({"isError":true,"executed":false,"error":error}),
            Ok((config, input, timeout)) => {
                let gate_app = app.clone();
                let gate_scene = scene.clone();
                let gate_run = run.clone();
                let log = root
                    .join("agent-output")
                    .join(&run)
                    .join(format!("{}.log", uuid::Uuid::new_v4()));
                let tool = call.name.clone();
                let id = call.id.clone();
                crate::mcp_runtime::run_agent_command(config,input,timeout,log,Arc::new(move||authorized(&gate_app,&gate_scene,&gate_run)),|stream,text|{
                    if !canceled.load(Ordering::Acquire){let _=crate::emit_run_event(&app,json!({"sceneId":scene,"runId":run,"status":"running","toolOutput":{"callId":id,"tool":tool,"stream":stream,"text":text}}));}
                }).await?
            }
        }
    } else {
        let a = app.clone();
        let s = scene.clone();
        let r = run.clone();
        let name = call.name.clone();
        let args = call.arguments.clone();
        let mutates = mewu_agent_tools::mutates(&name);
        let mut worker = tauri::async_runtime::spawn_blocking(move || {
            mewu_agent_tools::execute(&name, &args, &cwd, &|| {
                if canceled.load(Ordering::Acquire) {
                    return Err("运行已停止".into());
                }
                if mutates {
                    authorized(&a, &s, &r)?;
                }
                Ok(())
            })
        });
        let result = loop {
            tokio::select! {
                value=&mut worker=>break value.map_err(|_|"文件工具执行中断；请核对实际文件状态")?,
                _=tokio::time::sleep(Duration::from_millis(40))=>authorized(&app,&scene,&run)?,
            }
        };
        match result {
            Ok(value) => value,
            Err(error) => json!({"isError":true,"error":error,"executed":false}),
        }
    };
    let failed = value.get("isError") == Some(&Value::Bool(true));
    if failed {
        value["correction"]=json!("Read the actual error/output and correct the next action. Do not create a plugin for ordinary local tasks. A timed-out command may have partial effects; inspect state before retrying.");
    }
    let body = serde_json::to_string(&value).map_err(|_| "工具结果无法序列化")?;
    if body.len() > journal_runtime::MAX_TOOL_OUTPUT {
        return Err("工具结果超出持久回执预算；请核对实际文件或命令日志，禁止重放写入".into());
    }
    let host = app.state::<Host>();
    let mut engine = host.lock()?;
    let receipt = engine
        .store
        .record_tool_response(
            &lease,
            ObservedToolResult {
                response_kind: if mewu_agent_tools::mutates(&call.name) {
                    ToolResponseKind::LocalCommit
                } else {
                    ToolResponseKind::LocalRead
                },
                returned_error: Some(failed),
                observed_encoding: "mewu-local-json-v1".into(),
                observed_bytes: body.len() as u64,
                observed_sha256: format!("{:x}", Sha256::digest(body.as_bytes())),
                retained_content: JournalContent::Json { value },
                references: vec![],
                model_visible_json: Some(body.clone()),
            },
        )
        .map_err(journal_runtime::store_error)?;
    journal_runtime::emit_changed(&app, &scene, &run, receipt.journal_revision);
    crate::publish(&app, &engine.store.snapshot());
    if !crate::run_is_authorized(&engine, &scene, &run, None) {
        return Err("运行已结束；已观察到的操作回执已保留".into());
    }
    Ok(CommittedToolOutput { model_json: body })
}
