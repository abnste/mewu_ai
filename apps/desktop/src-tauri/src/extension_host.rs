// SPDX-License-Identifier: MPL-2.0
//! Executable extension lifecycle and capability bridge. Plugins receive no
//! Tauri handle or ambient OS API. Explicit process.rpc permission admits local
//! programs under the user's normal OS identity; it is not an OS sandbox.
use crate::{
    extension_market as market, extension_region as region,
    extension_store::{service_platform_error, Binding, Record, Source, Store},
    Host,
};
use base64::Engine as _;
use mewu_extension::{
    ExtensionPackage, ProtocolError, ServiceRegistration, WorkerRequest, WorkerResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

#[path = "extension_secrets.rs"]
mod secrets;
#[path = "extension_memory_archive.rs"]
mod memory_archive;

pub struct ExtensionHost {
    pub(crate) surfaces: crate::extension_surfaces::Surfaces,
    store: Mutex<Store>,
    pending: Mutex<HashMap<String, Pending>>,
    runs: Mutex<HashMap<String, ActiveRun>>,
    #[cfg(windows)]
    pub(crate) region_focus_owner: Mutex<Option<String>>,
    // SQLite is dropped before this final lease owner releases the root lock.
    _storage_lock: Arc<crate::storage_root::RuntimeLock>,
}
struct Pending {
    package: ExtensionPackage,
    bytes: Vec<u8>,
    source: Source,
    expected: Option<u64>,
    created: Instant,
}
struct ActiveRun {
    plugin: String,
    scope: String,
    canceled: Arc<AtomicBool>,
    surface: Option<(String,crate::extension_surfaces::Owner)>,
    surface_window: Option<crate::extension_surfaces::WindowTicket>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    id: String,
    manifest: mewu_extension::ExtensionManifest,
    sha256: String,
    source: Source,
    expected_revision: Option<u64>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Invocation {
    plugin_id: String,
    revision: u64,
    service_id: String,
    #[serde(default)]
    input: Value,
    scene_id: Option<String>,
    region_id: Option<String>,
    #[serde(default)]
    object_source: Option<crate::extension_objects_host::Source>,
    #[serde(default)]
    surface: Option<SurfaceInvocation>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag="action", rename_all="camelCase", deny_unknown_fields)]
enum SurfaceInvocation { Mount, Call { id:String }, Close { id:String } }
pub(crate) struct ExternalAgentProvider {
    pub agent_id: String,
    pub record: Record,
    pub service_id: String,
    pub authority: crate::extension_agent_authority::ExternalAgentAuthority,
}
pub(crate) fn external_agent_for_scene(
    app: &AppHandle,
    scene_id: &str,
) -> Result<Option<ExternalAgentProvider>, String> {
    let agent_id = {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        let scene = engine.store.scene(scene_id).map_err(|e| e.to_string())?;
        if scene.closed {
            return Err("会话已关闭".into());
        }
        scene.agent_id.clone()
    };
    let extensions = app.state::<ExtensionHost>();
    let store = extensions.store()?;
    let Some(binding) = store.binding(&agent_id, "agent")? else {
        return Ok(None);
    };
    let (Some(plugin), Some(service)) = (binding.plugin_id.as_ref(), binding.service_id.as_ref())
    else {
        return Ok(None);
    };
    let record = store.get(plugin)?;
    record.ensure_supported()?;
    if record.state != "enabled"
        || !record
            .services
            .iter()
            .any(|s| &s.id == service && s.kind == mewu_extension::ServiceKind::Agent)
    {
        return Err("已选的 Agent 扩展不可用".into());
    }
    store.validate_dependencies(&record.manifest)?;
    let authority = crate::extension_agent_authority::ExternalAgentAuthority {
        agent_id: agent_id.clone(),
        plugin_id: plugin.clone(),
        service_id: service.clone(),
        plugin_revision: record.revision,
        binding_revision: binding.revision,
        sha256: record.sha256.clone(),
        canceled: Arc::new(AtomicBool::new(false)),
    };
    crate::extension_agent_authority::register(
        &authority,
        record.manifest.dependencies.keys().cloned().collect(),
    )?;
    Ok(Some(ExternalAgentProvider {
        agent_id,
        service_id: service.clone(),
        record,
        authority,
    }))
}
impl ExtensionHost {
    pub(crate) fn window_destroyed(&self,label:&str){
        self.surfaces.revoke_window(label);
        if let Ok(runs)=self.runs.lock(){for run in runs.values().filter(|run|run.surface_window.as_ref().is_some_and(|ticket|ticket.label==label)){
            run.canceled.store(true,Ordering::SeqCst);
        }}
    }
    pub fn open(
        root: &Path,
        storage_lock: Arc<crate::storage_root::RuntimeLock>,
    ) -> Result<Self, String> {
        Ok(Self {
            surfaces: Default::default(),
            store: Mutex::new(Store::open(root)?),
            pending: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
            #[cfg(windows)]
            region_focus_owner: Mutex::new(None),
            _storage_lock: storage_lock,
        })
    }
    pub(crate) fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, String> {
        self.store.lock().map_err(|_| "扩展存储不可用".into())
    }
    pub fn cancel_all(&self) {
        self.surfaces.revoke(None);
        crate::extension_agent_authority::cancel_all();
        if let Ok(runs) = self.runs.lock() {
            for run in runs.values() {
                run.canceled.store(true, Ordering::SeqCst);
            }
        }
    }
    pub(crate) fn is_idle(&self) -> bool {
        self.runs.lock().is_ok_and(|runs| runs.is_empty())
    }
    pub async fn drain(&self, timeout: Duration) -> Result<(), String> {
        self.cancel_all();
        let deadline = Instant::now() + timeout;
        loop {
            if self.is_idle() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("扩展运行时尚未退出".into());
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
    }
    fn cancel_plugin(&self, id: &str) {
        self.surfaces.revoke(Some(id));
        crate::extension_agent_authority::cancel_plugin(id);
        if let Ok(runs) = self.runs.lock() {
            for run in runs.values().filter(|r| r.plugin == id) {
                run.canceled.store(true, Ordering::SeqCst);
            }
        }
    }
    pub(crate) fn close_surface(&self,id:&str,owner:&crate::extension_surfaces::Owner)->Result<(),String> {
        if let Ok(runs)=self.runs.lock() {
            for run in runs.values().filter(|run|run.surface.as_ref().is_some_and(|(key,value)|key==id&&value==owner)) {
                run.canceled.store(true,Ordering::SeqCst);
            }
        }
        self.surfaces.close(id,owner)
    }
    fn cancel_agent(&self, id: &str) {
        crate::extension_agent_authority::cancel_agent(id);
        if let Ok(runs) = self.runs.lock() {
            for run in runs.values().filter(|r| r.scope == id) {
                run.canceled.store(true, Ordering::SeqCst);
            }
        }
    }
}
fn trusted(window: &tauri::WebviewWindow) -> Result<(), String> {
    if !["space", "settings"].contains(&window.label()) {
        return Err("此窗口不能管理扩展".into());
    }
    Ok(())
}
fn publish(app: &AppHandle) {
    if let Ok(records) = app.state::<ExtensionHost>().store().and_then(|s| s.list()) {
        let _ = app.emit("extension-state", records);
    }
    crate::extension_surface_window::reconcile(app);
}
#[tauri::command]
pub fn get_extensions(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ExtensionHost>,
) -> Result<Vec<Record>, String> {
    trusted(&window)?;
    state.store()?.list()
}
#[tauri::command]
pub async fn get_extension_catalog(
    window: tauri::WebviewWindow,
) -> Result<market::Catalog, String> {
    trusted(&window)?;
    market::get_catalog().await
}
#[tauri::command]
pub async fn get_extension_project(
    window: tauri::WebviewWindow,
    entry: market::Entry,
) -> Result<Option<crate::extension_project::Project>, String> {
    trusted(&window)?;
    crate::extension_project_host::get_project(&entry).await
}
#[tauri::command]
pub async fn get_extension_project_image(
    window: tauri::WebviewWindow,
    entry: market::Entry,
    index: usize,
    expected_source_commit: String,
    expected_image_sha256: String,
) -> Result<String, String> {
    trusted(&window)?;
    crate::extension_project_host::get_image(
        &entry,
        index,
        &expected_source_commit,
        &expected_image_sha256,
    )
    .await
}
#[tauri::command]
pub async fn prepare_extension(
    app: AppHandle,
    window: tauri::WebviewWindow,
    entry: Option<market::Entry>,
) -> Result<Option<Proposal>, String> {
    trusted(&window)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let (package, bytes, source) = if let Some(entry) = entry {
        if !market::supports_platform(&entry) {
            return Err("此插件不支持当前平台".into());
        }
        let (package, bytes) = market::get_package(&entry).await?;
        (package, bytes, Source::Github { entry })
    } else {
        let picked = tauri::async_runtime::spawn_blocking(move || {
            rfd::FileDialog::new()
                .set_parent(&window)
                .add_filter("Mewu 插件", &["zip"])
                .pick_file()
        })
        .await
        .map_err(|_| "选择插件中断")?;
        let Some(path) = picked else { return Ok(None) };
        let text = path.to_str().ok_or("插件路径不是 UTF-8")?.to_owned();
        let bytes = tauri::async_runtime::spawn_blocking(move || {
            let file = std::fs::File::open(path).map_err(|_| "无法读取插件")?;
            let mut bytes = Vec::new();
            file.take(market::MAX_ARCHIVE as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "无法读取插件")?;
            Ok::<_, String>(bytes)
        })
        .await
        .map_err(|_| "读取插件中断")??;
        let package = market::decode_package(&bytes)?;
        (package, bytes, Source::Local { path: text })
    };
    let state = app.state::<ExtensionHost>();
    if !mewu_extension::plugin_platform_supported(package.manifest.platforms.as_deref(), mewu_extension::current_plugin_platform()) {
        return Err("此插件不支持当前系统".into());
    }
    let expected = state
        .store()?
        .get_optional(&package.manifest.id)?
        .map(|p| p.revision);
    let proposal = Proposal {
        id: uuid::Uuid::new_v4().to_string(),
        manifest: package.manifest.clone(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        source: source.clone(),
        expected_revision: expected,
    };
    let mut pending = state.pending.lock().map_err(|_| "扩展预览不可用")?;
    pending.retain(|_, p| p.created.elapsed() < Duration::from_secs(600));
    if pending.len() >= 8 {
        return Err("待安装扩展过多".into());
    }
    pending.insert(
        proposal.id.clone(),
        Pending {
            package,
            bytes,
            source,
            expected,
            created: Instant::now(),
        },
    );
    Ok(Some(proposal))
}
#[tauri::command]
pub async fn install_extension(
    app: AppHandle,
    window: tauri::WebviewWindow,
    proposal_id: String,
) -> Result<Vec<Record>, String> {
    trusted(&window)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let record = {
        let state = app.state::<ExtensionHost>();
        let pending = state
            .pending
            .lock()
            .map_err(|_| "扩展预览不可用")?
            .remove(&proposal_id)
            .ok_or("安装预览已失效")?;
        if pending.created.elapsed() > Duration::from_secs(600) {
            return Err("安装预览已失效".into());
        }
        state.cancel_plugin(&pending.package.manifest.id);
        let mut store = state.store()?;
        let result = store.install(
            &pending.package,
            &pending.bytes,
            &pending.source,
            pending.expected,
        )?;
        state.cancel_plugin(&pending.package.manifest.id);
        result
    };
    finish_activation(&app, &record).await?;
    let result = app.state::<ExtensionHost>().store()?.list();
    result
}
#[tauri::command]
pub async fn control_extension(
    app: AppHandle,
    window: tauri::WebviewWindow,
    plugin_id: String,
    revision: u64,
    action: String,
) -> Result<Vec<Record>, String> {
    trusted(&window)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let record = {
        let state = app.state::<ExtensionHost>();
        state.cancel_plugin(&plugin_id);
        let mut store = state.store()?;
        let result = if action == "rollback" {
            store.rollback(&plugin_id, revision)?
        } else {
            store.set_state(
                &plugin_id,
                revision,
                match action.as_str() {
                    "enable" => "enabled",
                    "disable" => "disabled",
                    "remove" => "removed",
                    _ => return Err("扩展操作无效".into()),
                },
            )?
        };
        state.cancel_plugin(&plugin_id);
        result
    };
    finish_activation(&app, &record).await?;
    let result = app.state::<ExtensionHost>().store()?.list();
    result
}

async fn finish_activation(app: &AppHandle, record: &Record) -> Result<(), String> {
    if record.state == "enabled" {
        let app2 = app.clone();
        let record2 = record.clone();
        match tauri::async_runtime::spawn_blocking(move || activate(&app2, &record2)).await {
            Ok(result) => result?,
            // A worker in the unwinding blocking task has already been dropped
            // (shutdown/kill/wait). Join failure still needs registry compensation.
            Err(_) => return Err(activation_failed(app, record, "扩展激活中断".into())),
        }
    }
    publish(app);
    Ok(())
}

fn activation_failed(app: &AppHandle, record: &Record, cause: String) -> String {
    let compensated = (|| {
        let state = app.state::<ExtensionHost>();
        let mut store = state.store()?;
        if store
            .fail_activation(&record.manifest.id, record.revision)?
            .is_some()
        {
            // Keep this post-write cancellation inside the Store guard. A new
            // provider authority cannot be issued between the CAS and revocation.
            state.cancel_plugin(&record.manifest.id);
        }
        Ok::<_, String>(())
    })();
    // Publish the actual registry even when compensation fails or a newer
    // revision wins. Never report a failed activation as successful installation.
    publish(app);
    match compensated {
        Ok(()) => cause,
        Err(error) => format!("{cause}；{error}"),
    }
}

/// Only host-owned static diagnostics cross the error boundary. Arbitrary JS
/// errors, provider bodies, OS paths and arguments never become diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegionDiagnostic {
    code: &'static str,
    reason: &'static str,
}
fn safe_region_reason(message: &str) -> &'static str {
    const REASONS: &[&str] = &[
        "无法确认物理屏幕坐标",
        "目标窗口已改变或失去焦点，区域操作已停止",
        "请选择应用内容区域",
        "操作区域被其他窗口遮挡",
        "选区必须完整位于同一个应用的内容区域",
        "选区必须完整位于同一个应用窗口内",
        "请先松开键盘修饰键",
        "请先松开鼠标按钮",
        "无法确认选区内的输入焦点",
        "请将输入框完整包含在操作选区内",
        "系统拒绝了区域输入",
        "区域操作已停止",
        "鼠标位置受到系统限制",
        "桌面坐标范围已改变",
        "请选择原始截图中的有效区域",
        "操作区域不能超过 4 Mi 像素",
        "当前平台尚不支持区域自动操作",
        "请在系统设置中允许 MewuAI 使用辅助功能",
        "输入焦点不在授权区域内",
        "不能向密码框输入",
        "请松开修饰键或停止键后重试",
        "请松开鼠标后重试",
        "操作坐标超出授权区域",
        "截图文件已不可用",
        "截图文件大小无效",
        "无法读取截图文件",
        "截图文件已改变",
        "已有区域操作正在运行",
        "显示器设置已改变，请重新选择区域",
        "无法编码区域截图",
        "请先观察当前区域，再执行下一步操作",
        "拖动路径需要 2 至 256 个坐标",
        "单次拖动路径过长",
        "请输入不含控制字符的短文本",
        "滚动量超出单次范围",
        "显示器配置不可用",
        "显示器标识不唯一",
        "截图与当前显示器坐标不一致，请重新截图",
        "区域运行已停止",
        "无法注册区域停止键，请先结束其他使用 Esc 的操作",
        "插件授权已撤销",
        "会话已关闭",
        "截图来源已变化",
        "选区已删除",
        "选区已变化",
        "覆盖层状态无效",
        "截图覆盖层操作已撤销",
        "无法隐藏截图覆盖层",
        "隐藏截图覆盖层超时",
        "插件不能访问未授权的选区",
        "缺少区域动作",
        "区域动作无效",
        "没有当前会话授权",
        "没有当前选区授权",
        "会话不存在",
        "没有截图背景",
        "选区不存在",
        "截图路径无效",
        "无法读取截图",
        "截图源文件过大",
        "素材不存在或路径无效",
        "素材目录不可用",
        "素材路径超出范围",
    ];
    REASONS
        .iter()
        .copied()
        .find(|known| *known == message)
        .unwrap_or("区域操作失败")
}
fn record_region_error(
    diagnostic: &mut Option<RegionDiagnostic>,
    error: region::RegionError,
) -> String {
    let code = match error.code {
        code @ ("region_authority_revoked"
        | "region_accessibility_permission"
        | "region_focus_outside"
        | "region_secure_input"
        | "region_coordinates_unavailable"
        | "region_cursor_clipped"
        | "region_display_changed"
        | "region_input_busy"
        | "region_input_failed"
        | "region_keyboard_focus_denied"
        | "region_system_target_denied"
        | "region_target_changed"
        | "region_target_obscured"
        | "region_window_boundary"
        | "overlay_hide_failed"
        | "region_busy"
        | "region_capture_failed"
        | "region_observation_required"
        | "region_path_invalid"
        | "region_platform_unsupported"
        | "region_point_outside"
        | "region_scroll_invalid"
        | "region_source_changed"
        | "region_source_invalid"
        | "region_source_missing"
        | "region_stopped"
        | "region_stop_unavailable"
        | "region_text_invalid"
        | "region_too_large") => code,
        _ => "region_operation_failed",
    };
    *diagnostic = Some(RegionDiagnostic {
        code,
        reason: safe_region_reason(&error.message),
    });
    error.to_string()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostCallFailure {
    method: &'static str,
    code: &'static str,
    reason: &'static str,
}
impl HostCallFailure {
    fn from_error(method: &str, message: &str, region: Option<RegionDiagnostic>) -> Self {
        let (method, code, reason) = match method {
            "region.observe" => (
                "region.observe",
                "REGION_OPERATION_FAILED",
                safe_region_reason(message),
            ),
            "region.action" => (
                "region.action",
                "REGION_OPERATION_FAILED",
                safe_region_reason(message),
            ),
            "model.invoke" => ("model.invoke", "MODEL_REQUEST_FAILED", "模型请求失败"),
            "network.fetch" => ("network.fetch", "NETWORK_REQUEST_FAILED", "网络请求失败"),
            "process.resolve" => ("process.resolve", "PROCESS_RESOLVE_FAILED", "无法找到已安装的运行程序"),
            "process.open" => ("process.open", "PROCESS_OPEN_FAILED", "无法启动外部程序"),
            "process.write" => ("process.write", "PROCESS_WRITE_FAILED", "外部程序输入失败"),
            "process.read" => ("process.read", "PROCESS_READ_FAILED", "外部程序输出失败或连接已关闭"),
            "process.close" => ("process.close", "PROCESS_CLOSE_FAILED", "外部程序关闭失败"),
            "storage.get" => (
                "storage.get",
                "STORAGE_OPERATION_FAILED",
                "插件存储操作失败",
            ),
            "storage.set" => (
                "storage.set",
                "STORAGE_OPERATION_FAILED",
                "插件存储操作失败",
            ),
            "storage.delete" => (
                "storage.delete",
                "STORAGE_OPERATION_FAILED",
                "插件存储操作失败",
            ),
            "storage.compareAndSet" => (
                "storage.compareAndSet",
                "STORAGE_OPERATION_FAILED",
                "插件存储操作失败",
            ),
            "storage.list" => (
                "storage.list",
                "STORAGE_OPERATION_FAILED",
                "插件存储操作失败",
            ),
            "services.list" => (
                "services.list",
                "SERVICE_OPERATION_FAILED",
                "插件服务调用失败",
            ),
            "services.call" => (
                "services.call",
                "SERVICE_OPERATION_FAILED",
                "插件服务调用失败",
            ),
            "ui.render" => ("ui.render", "RENDER_OPERATION_FAILED", "界面生成失败"),
            "objects.library.get" => ("objects.library.get", "COLLECTION_OPERATION_FAILED", "对象库读取失败"),
            "objects.library.preview" => ("objects.library.preview", "COLLECTION_OPERATION_FAILED", "对象预览失败"),
            "objects.library.describe" => ("objects.library.describe", "COLLECTION_OPERATION_FAILED", "宿主不支持对象库"),
            "objects.library.commit" => ("objects.library.commit", "COLLECTION_OPERATION_FAILED", "对象收藏失败"),
            "objects.library.restore" => ("objects.library.restore", "COLLECTION_OPERATION_FAILED", "收藏对象取出失败"),
            "events.emit" => ("events.emit", "EVENT_OPERATION_FAILED", "插件事件发送失败"),
            _ => ("unsupported", "HOST_UNSUPPORTED", "宿主不支持此扩展接口"),
        };
        if matches!(method, "region.observe" | "region.action") {
            if let Some(region) = region {
                return Self {
                    method,
                    code: region.code,
                    reason: region.reason,
                };
            }
        }
        if message.starts_with("插件未声明权限：") {
            return Self {
                method,
                code: "HOST_PERMISSION_DENIED",
                reason: "插件未声明所需权限",
            };
        }
        if matches!(
            message,
            "扩展运行已停止"
                | "扩展已停用或版本已变化"
                | "调用方扩展授权已撤销"
                | "扩展绑定已撤销"
                | "扩展绑定已变化"
                | "扩展所属会话已变化"
        ) {
            return Self {
                method,
                code: "HOST_AUTHORITY_REVOKED",
                reason: "扩展授权已变化或操作已停止",
            };
        }
        Self {
            method,
            code,
            reason,
        }
    }
    fn public_message(self) -> String {
        format!("{}：{} [{}]", self.method, self.reason, self.code)
    }
    fn protocol_error(self) -> ProtocolError {
        ProtocolError {
            code: self.code.into(),
            message: self.public_message(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HostCallProgress {
    last_success: Option<&'static str>,
    completed: u32,
    model_shape: Option<(&'static str, usize)>,
}
impl HostCallProgress {
    fn succeeded(&mut self, method: &str, result: &Value) {
        self.last_success = Some(match method {
            "region.observe" => "region.observe",
            "region.action" => "region.action",
            "model.invoke" => "model.invoke",
            "process.resolve" | "process.open" | "process.write" | "process.read" | "process.close" => "process.rpc",
        "network.fetch" => "network.fetch",
            "storage.get" => "storage.get",
            "storage.set" => "storage.set",
            "storage.delete" => "storage.delete",
            "storage.compareAndSet" => "storage.compareAndSet",
            "storage.list" => "storage.list",
            "services.list" => "services.list",
            "services.call" => "services.call",
            "ui.render" => "ui.render",
            "objects.library.get" => "objects.library.get",
            "objects.library.preview" => "objects.library.preview",
            "objects.library.describe" => "objects.library.describe",
            "objects.library.commit" => "objects.library.commit",
            "objects.library.restore" => "objects.library.restore",
            "events.emit" => "events.emit",
            _ => "unsupported",
        });
        self.completed = self.completed.saturating_add(1);
        if method == "model.invoke" {
            let content = match result.get("content").and_then(Value::as_str) {
                Some(value) if value.trim().is_empty() => "empty",
                Some(_) => "text",
                None => "invalid",
            };
            self.model_shape = Some((
                content,
                result
                    .get("toolCalls")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
            ));
        }
    }
}
fn worker_failure(
    error: &ProtocolError,
    last_host_failure: Option<HostCallFailure>,
    progress: HostCallProgress,
) -> String {
    let code = match error.code.as_str() {
        code @ ("INVALID_PACKAGE" | "INVALID_PROTOCOL" | "RUNTIME_ERROR" | "TIMEOUT"
        | "CANCELED" | "TRANSPORT_ERROR" | "JS_SYNTAX_ERROR" | "JS_TYPE_ERROR"
        | "JS_RANGE_ERROR" | "JS_REFERENCE_ERROR" | "JS_URI_ERROR" | "JS_EVAL_ERROR"
        | "JS_ERROR" | "JS_UNKNOWN_ERROR") => code,
        _ => "RUNTIME_ERROR",
    };
    let mut message = format!("扩展执行失败 [{code}]");
    if let Some(failure) = last_host_failure {
        message.push_str("；最近宿主调用失败：");
        message.push_str(&failure.public_message());
    }
    message.push_str(&format!(
        "；已完成宿主调用：{}；最近成功：{}",
        progress.completed,
        progress.last_success.unwrap_or("none")
    ));
    if let Some((content, tool_calls)) = progress.model_shape {
        message.push_str(&format!("；模型正文：{content}；工具调用数：{tool_calls}"));
    }
    message
}

/// Only WorkerResponse::Error's existing sanitizer can create this value.
/// It is transient UI diagnostics, never a tool response or persisted payload.
#[derive(Clone)]
pub(crate) struct SafeToolDiagnostic(String);
impl SafeToolDiagnostic {
    fn from_worker(
        error: &ProtocolError,
        last_host_failure: Option<HostCallFailure>,
        progress: HostCallProgress,
    ) -> Self {
        Self(worker_failure(error, last_host_failure, progress))
    }
    pub(crate) fn message(&self) -> &str {
        &self.0
    }
}

struct Worker {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<WorkerResponse, String>>,
    safe_failure: Option<SafeToolDiagnostic>,
}
impl Worker {
    fn spawn() -> Result<Self, String> {
        let mut command = Command::new(std::env::current_exe().map_err(|_| "无法定位扩展运行时")?);
        command
            .arg("--extension-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| "无法启动扩展运行时")?;
        let input = child.stdin.take().ok_or("扩展输入不可用")?;
        let output = child.stdout.take().ok_or("扩展输出不可用")?;
        let (send, receive) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let read = reader
                    .by_ref()
                    .take(mewu_extension::MAX_PROTOCOL_BYTES as u64 + 1)
                    .read_until(b'\n', &mut bytes);
                if matches!(read, Ok(0)) {
                    break;
                }
                if read.is_err()
                    || bytes.len() > mewu_extension::MAX_PROTOCOL_BYTES
                    || bytes.last() != Some(&b'\n')
                {
                    let _ = send.send(Err("扩展协议过大或读取失败".into()));
                    break;
                }
                let result = serde_json::from_slice(&bytes).map_err(|_| "扩展返回无效协议".into());
                if send.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            output: receive,
            safe_failure: None,
        })
    }
    fn send(&mut self, value: &WorkerRequest) -> Result<(), String> {
        serde_json::to_writer(&mut self.input, value).map_err(|_| "无法向扩展发送请求")?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| "扩展输入已关闭".into())
    }
    fn receive(
        &mut self,
        app: &AppHandle,
        context: &mut Context,
        id: &str,
        initialize: bool,
    ) -> Result<Value, String> {
        let deadline = Instant::now()
            + if initialize {
                Duration::from_secs(20)
            } else {
                Duration::from_secs(600)
            };
        let mut last_host_failure = None;
        let mut progress = HostCallProgress::default();
        loop {
            context.check(app)?;
            if Instant::now() > deadline {
                return Err("扩展执行超时".into());
            }
            match self.output.recv_timeout(Duration::from_millis(80)) {
                Ok(Ok(WorkerResponse::HostCall { id, method, params })) => {
                    let result = bridge(app, context, &method, params);
                    let request = match result {
                        Ok(result) => {
                            progress.succeeded(&method, &result);
                            WorkerRequest::HostResult {
                                id,
                                result: Some(result),
                                error: None,
                            }
                        }
                        Err(failure) => {
                            last_host_failure = Some(failure);
                            WorkerRequest::HostResult {
                                id,
                                result: None,
                                error: Some(failure.protocol_error()),
                            }
                        }
                    };
                    self.send(&request)?;
                }
                Ok(Ok(WorkerResponse::Initialized {
                    id: reply,
                    registrations,
                })) if initialize && reply == id => {
                    return serde_json::to_value(registrations).map_err(|_| "服务目录无效".into())
                }
                Ok(Ok(WorkerResponse::Result { id: reply, result }))
                    if !initialize && reply == id =>
                {
                    return Ok(result)
                }
                Ok(Ok(WorkerResponse::Error { id: reply, error })) if reply == id => {
                    let diagnostic =
                        SafeToolDiagnostic::from_worker(&error, last_host_failure, progress);
                    let message = diagnostic.message().to_owned();
                    self.safe_failure = Some(diagnostic);
                    return Err(message);
                }
                Ok(Ok(_)) => return Err("扩展返回意外的协议顺序".into()),
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("扩展运行时已退出".into()),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.send(&WorkerRequest::Shutdown {
            id: "shutdown".into(),
        });
        let start = Instant::now();
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            if start.elapsed() > Duration::from_millis(300) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
struct Context {
    record: Record,
    run_id: String,
    canceled: Arc<AtomicBool>,
    scene_id: Option<String>,
    region_id: Option<String>,
    scope: String,
    region: Option<Arc<region::RegionSession>>,
    chat_region: Option<Arc<crate::extension_chat_region::ChatRegionScope>>,
    native_run: Option<(String, String)>,
    model_connection: Option<mewu_core::ConnectionProfile>,
    model_lease: Option<Arc<crate::extension_model_lease::ModelLease>>,
    tool_preparing: bool,
    #[cfg(windows)]
    escape: crate::extension_escape::RunEscapeLease,
    #[cfg(windows)]
    focus_return: Option<Arc<crate::extension_focus::FocusTicket>>,
    frames: HashMap<String, region::RegionFrame>,
    default_messages: Vec<Value>,
    history: Option<Value>,
    default_images: Vec<String>,
    binding: Option<(String, u64)>,
    delegated_permissions: Option<Vec<String>>,
    authority_chain: Vec<(String, u64)>,
    public_reasoning: Option<Arc<Mutex<String>>>,
    memory_enabled_required: bool,
    depth: u8,
    object_access: Option<crate::extension_objects_host::Access>,
    processes: crate::extension_process::Scope,
    surface_owner: Option<(String,crate::extension_surfaces::Owner)>,
    surface_window: Option<crate::extension_surfaces::WindowTicket>,
}
impl Context {
    fn check(&self, app: &AppHandle) -> Result<(), String> {
        if self.canceled.load(Ordering::SeqCst) {
            return Err("扩展运行已停止".into());
        }
        let host = app.state::<Host>();
        host.exit.ensure_background()?;
        {
            let extensions = app.state::<ExtensionHost>();
            if self.surface_window.as_ref().is_some_and(|ticket|!extensions.surfaces.window_current(ticket)){return Err("插件宿主窗口已关闭".into());}
            let store = extensions.store()?;
            if let Some((id,owner))=&self.surface_owner {extensions.surfaces.check(id,owner)?;}
            let record = store.get(&self.record.manifest.id)?;
            record.ensure_supported()?;
            if record.state != "enabled" || record.revision != self.record.revision {
                return Err("扩展已停用或版本已变化".into());
            }
            store.validate_dependencies(&record.manifest)?;
            for (id, revision) in &self.authority_chain {
                let ancestor = store.get(id)?;
                ancestor.ensure_supported()?;
                if ancestor.state != "enabled" || ancestor.revision != *revision {
                    return Err("调用方扩展授权已撤销".into());
                }
                store.validate_dependencies(&ancestor.manifest)?;
            }
            if let Some((kind, revision)) = &self.binding {
                let live = store.binding(&self.scope, kind)?.ok_or("扩展绑定已撤销")?;
                if live.revision != *revision
                    || live.plugin_id.as_deref() != Some(&self.record.manifest.id)
                {
                    return Err("扩展绑定已变化".into());
                }
            }
        }
        if let Some(scope) = &self.chat_region {
            scope.check(app)?;
        }
        if let Some(scene_id) = &self.scene_id {
            let engine = host.lock()?;
            guard_model_connection(
                &engine.store,
                scene_id,
                self.model_connection.as_ref(),
                &self.canceled,
            )?;
            if let Some(lease) = &self.model_lease {
                let connection = self.model_connection.as_ref().ok_or("本轮模型授权缺失")?;
                lease.key(scene_id, self.native_run.as_ref(), &self.canceled, connection)?;
            }
            if self
                .native_run
                .as_ref()
                .is_some_and(|(scene, run)| !crate::run_is_authorized(&engine, scene, run, None))
            {
                return Err("Agent 运行已结束或授权已变化".into());
            }
            let scene = engine
                .store
                .scenes()
                .iter()
                .find(|s| &s.id == scene_id)
                .ok_or("会话已关闭")?;
            if scene.closed || scene.agent_id != self.scope {
                return Err("扩展所属会话已变化".into());
            }
            if self.surface_owner.is_some() && engine.store.active_scene().map_err(|e|e.to_string())?.id != *scene_id {
                return Err("插件界面所属会话已变化".into());
            }
            if self.memory_enabled_required
                && !engine
                    .store
                    .agent_profile(&self.scope)
                    .map_err(|e| e.to_string())?
                    .memory_enabled
            {
                return Err("长期记忆已关闭".into());
            }
        }
        Ok(())
    }
    fn permission(&self, name: &str) -> Result<(), String> {
        if !self.record.manifest.permissions.iter().any(|p| p == name)
            || self
                .delegated_permissions
                .as_ref()
                .is_some_and(|permissions| !permissions.iter().any(|p| p == name))
        {
            return Err(format!("插件未声明权限：{name}"));
        }
        Ok(())
    }
}
fn start_worker(
    app: &AppHandle,
    context: &mut Context,
) -> Result<(Worker, Vec<ServiceRegistration>), String> {
    context.check(app)?;
    let package = {
        let state = app.state::<ExtensionHost>();
        let store = state.store()?;
        store.validate_dependencies(&context.record.manifest)?;
        store.package(&context.record.manifest.id, context.record.revision)?
    };
    let mut worker = Worker::spawn()?;
    let id = uuid::Uuid::new_v4().to_string();
    worker.send(&WorkerRequest::Initialize {
        id: id.clone(),
        manifest: package.manifest,
        modules: package.modules,
    })?;
    let registrations: Vec<ServiceRegistration> =
        serde_json::from_value(worker.receive(app, context, &id, true)?)
            .map_err(|_| "扩展服务目录无效")?;
    Ok((worker, registrations))
}
fn activate(app: &AppHandle, record: &Record) -> Result<(), String> {
    let mut context = Context {
        record: record.clone(),
        run_id: uuid::Uuid::new_v4().to_string(),
        canceled: Arc::new(AtomicBool::new(false)),
        scene_id: None,
        region_id: None,
        scope: "default".into(),
        region: None,
        chat_region: None,
        native_run: None,
        model_connection: None,
        model_lease: None,
        tool_preparing: false,
        #[cfg(windows)]
        focus_return: None,
        #[cfg(windows)]
        escape: crate::extension_escape::RunEscapeLease::default(),
        frames: HashMap::new(),
        default_messages: vec![],
        history: None,
        default_images: vec![],
        binding: None,
        delegated_permissions: None,
        authority_chain: vec![],
        public_reasoning: None,
        memory_enabled_required: false,
        depth: 0,
                object_access: None,
                processes: Default::default(),
        surface_owner: None,
        surface_window: None,
    };
    let result = (|| {
        let mut registration = register_run(app, &context)?;
        let result = (|| {
            let (worker, registrations) = start_worker(app, &mut context)?;
            let result = {
                let state = app.state::<ExtensionHost>();
                let mut store = state.store()?;
                store.set_services(&record.manifest.id, record.revision, &registrations)
            };
            // Do not hold a SQLite guard across the process shutdown/wait.
            drop(worker);
            result
        })();
        registration.success = result.is_ok();
        result
    })();
    result.map_err(|cause| activation_failed(app, record, cause))
}
#[tauri::command]
pub async fn invoke_extension(
    app: AppHandle,
    window: tauri::WebviewWindow,
    invocation: Invocation,
) -> Result<Value, String> {
    let detached=crate::extension_surface_window::id(window.label()).is_some();
    if detached {
        let (id,owner)=crate::extension_surface_window::bound(&app,&window)?;
        if invocation.plugin_id!=owner.plugin||invocation.revision!=owner.revision||invocation.service_id!=owner.service
            || invocation.scene_id.as_deref()!=Some(&owner.scene)||invocation.region_id.is_some()||invocation.object_source.is_some()
            || !matches!(&invocation.surface,Some(SurfaceInvocation::Call{id:key}) if key==&id){
            return Err("插件对象不能调用其他界面".into());
        }
    } else {trusted(&window)?;}
    let surface_window=if invocation.surface.is_some(){app.state::<ExtensionHost>().surfaces.ticket(window.label())}else{None};
    // Releasing a previous generation must remain possible after scene change,
    // disable or shutdown admission closes. This never executes plugin code.
    if let Some(SurfaceInvocation::Close{id})=&invocation.surface {
        if window.label()!="space" && !(window.label()=="settings" && invocation.scene_id.is_none()) {return Err("此窗口不能关闭插件界面".into());}
        let owner=crate::extension_surfaces::Owner {
            plugin:invocation.plugin_id.clone(),revision:invocation.revision,
            service:invocation.service_id.clone(),scene:invocation.scene_id.clone().unwrap_or_default(),
        };
        app.state::<ExtensionHost>().close_surface(id,&owner)?;
        return Ok(json!({"closed":true}));
    }
    if invocation.region_id.is_some() && window.label() != "space" {
        return Err("请从截图选区启动区域操作".into());
    }
    app.state::<Host>().exit.ensure_new_operation()?;
    let record = app
        .state::<ExtensionHost>()
        .store()?
        .get(&invocation.plugin_id)?;
    if record.revision != invocation.revision || record.state != "enabled" {
        return Err("扩展已停用或版本已变化".into());
    }
    let scope = if let Some(scene_id) = &invocation.scene_id {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        let scene = engine
            .store
            .scenes()
            .iter()
            .find(|s| &s.id == scene_id)
            .ok_or("会话不存在")?;
        if !detached && engine.store.snapshot().active_scene_id != *scene_id {
            return Err("请使用当前会话".into());
        }
        scene.agent_id.clone()
    } else {
        "default".into()
    };
    let run_id = uuid::Uuid::new_v4().to_string();
    #[cfg(windows)]
    let focus_return = invocation.region_id.as_ref().and_then(|_| {
        crate::extension_focus::FocusTicket::new(
            &app,
            run_id.clone(),
            invocation.scene_id.as_deref()?,
        )
    });
    let canceled=Arc::new(AtomicBool::new(false));
    let service=record.services.iter().find(|s|s.id==invocation.service_id).ok_or("插件服务不存在")?;
    let owner=crate::extension_surfaces::Owner {
        plugin: record.manifest.id.clone(), revision: record.revision,
        service: invocation.service_id.clone(), scene: invocation.scene_id.clone().unwrap_or_default(),
    };
    if invocation.surface.is_some() {
        let settings = crate::extension_objects_host::is_settings_surface(service)
            && matches!(window.label(),"settings"|"space") && invocation.scene_id.is_none()
            && invocation.region_id.is_none() && invocation.object_source.is_none();
        let space = (detached||window.label()=="space") && !owner.scene.is_empty()
            && crate::extension_objects_host::is_surface(service);
        if !(settings||space)
            || !record.manifest.permissions.iter().any(|p|p=="ui.render") { return Err("插件界面未授权".into()); }
        let state=app.state::<ExtensionHost>();
        match &invocation.surface {
            Some(SurfaceInvocation::Mount)=>{},
            Some(SurfaceInvocation::Call{id})=>state.surfaces.check(id,&owner)?,
            Some(SurfaceInvocation::Close{id})=>{ state.close_surface(id,&owner)?;return Ok(json!({"closed":true})); },
            None=>{}
        }
    }
    let object_access=if matches!(invocation.surface,Some(SurfaceInvocation::Call{..})) && crate::extension_objects_host::is_surface(service)
        && record.manifest.permissions.iter().any(|p|p=="objects.library") {
        if (!detached&&window.label()!="space") || !matches!(invocation.surface,Some(SurfaceInvocation::Call{..})) {
            return Err("此窗口不能授权对象库".into());
        }
        Some(crate::extension_objects_host::Access {
            plugin_id:record.manifest.id.clone(), revision:record.revision,
            scene_id:invocation.scene_id.clone().ok_or("对象库缺少当前会话")?,
            canceled:canceled.clone(),source:invocation.object_source,
            surface_id: match &invocation.surface {Some(SurfaceInvocation::Call{id})=>id.clone(),_=>unreachable!()},
            service_id:invocation.service_id.clone(),
        })
    } else {
        if invocation.object_source.is_some(){return Err("此服务不能接收对象拖放".into());}
        None
    };
    let context = Context {
        record,
        run_id,
        canceled,
        scene_id: if detached {None}else{invocation.scene_id},
        region_id: invocation.region_id,
        scope,
        region: None,
        chat_region: None,
        native_run: None,
        model_connection: None,
        model_lease: None,
        tool_preparing: false,
        #[cfg(windows)]
        focus_return,
        #[cfg(windows)]
        escape: crate::extension_escape::RunEscapeLease::default(),
        frames: HashMap::new(),
        default_messages: vec![],
        history: None,
        default_images: vec![],
        binding: None,
        delegated_permissions: None,
        authority_chain: vec![],
        public_reasoning: None,
        memory_enabled_required: false,
        depth: 0,
                object_access,
        processes: Default::default(),
        surface_owner: match &invocation.surface {Some(SurfaceInvocation::Call{id})=>Some((id.clone(),owner.clone())),_=>None},
        surface_window: surface_window.clone(),
    };
    let mount=matches!(invocation.surface,Some(SurfaceInvocation::Mount));
    let result=run_context(app.clone(), context, invocation.service_id, invocation.input).await?;
    if mount {
        let document:crate::extension_surfaces::DocumentInput=serde_json::from_value(result).map_err(|_|"插件界面返回格式无效")?;
        let state=app.state::<ExtensionHost>();
        let registry=state.store()?;
        let record=registry.get(&owner.plugin)?;
        if record.revision!=owner.revision || record.state!="enabled" {return Err("插件界面已撤权".into());}
        let host=app.state::<Host>();
        let engine=host.lock()?;
        if !owner.scene.is_empty() && engine.store.active_scene().map_err(|e|e.to_string())?.id!=owner.scene {return Err("会话已变化".into());}
        serde_json::to_value(state.surfaces.mount_scoped(owner,document,&host.content_origin,surface_window.as_ref())?).map_err(|_|"插件界面无效".into())
    } else { Ok(result) }
}
#[tauri::command]
pub fn stop_extension(
    app: AppHandle,
    window: tauri::WebviewWindow,
    plugin_id: Option<String>,
) -> Result<(), String> {
    trusted(&window)?;
    let state = app.state::<ExtensionHost>();
    if let Some(id) = plugin_id {
        state.cancel_plugin(&id)
    } else {
        state.cancel_all()
    }
    crate::extension_surface_window::reconcile(&app);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingChange {
    agent_id: String,
    kind: String,
    plugin_id: Option<String>,
    service_id: Option<String>,
    expected_revision: Option<u64>,
}
#[tauri::command]
pub fn get_extension_bindings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ExtensionHost>,
) -> Result<Vec<Binding>, String> {
    trusted(&window)?;
    state.store()?.bindings()
}
#[tauri::command]
pub fn set_extension_binding(
    app: AppHandle,
    window: tauri::WebviewWindow,
    binding: BindingChange,
) -> Result<Vec<Binding>, String> {
    trusted(&window)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    if !["agent", "memory"].contains(&binding.kind.as_str())
        || binding.plugin_id.is_some() != binding.service_id.is_some()
    {
        return Err("扩展绑定无效".into());
    }
    // Same extension -> Engine order as UI persistence. No I/O awaits in locks.
    let extensions = app.state::<ExtensionHost>();
    let mut store = extensions.store()?;
    {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        if !engine
            .store
            .snapshot()
            .agents
            .iter()
            .any(|a| a.id == binding.agent_id && !a.archived)
        {
            return Err("Agent 不存在".into());
        }
    }
    if let (Some(plugin), Some(service)) = (&binding.plugin_id, &binding.service_id) {
        let record = store.get(plugin)?;
        let expected_kind = if binding.kind == "agent" {
            mewu_extension::ServiceKind::Agent
        } else {
            mewu_extension::ServiceKind::Memory
        };
        if record.state != "enabled"
            || !record
                .services
                .iter()
                .any(|s| &s.id == service && s.kind == expected_kind)
        {
            return Err("扩展未启用或没有对应服务".into());
        }
        record.ensure_supported()?;
        let selected_service = record.services.iter().find(|s| &s.id == service).ok_or("扩展没有对应服务")?;
        if let Some(reason) = service_platform_error(selected_service, mewu_extension::current_plugin_platform()) { return Err(reason.into()); }
        store.validate_dependencies(&record.manifest)?;
    }
    store.set_binding(
        &binding.agent_id,
        &binding.kind,
        binding.plugin_id.as_deref(),
        binding.service_id.as_deref(),
        binding.expected_revision,
    )?;
    let bindings = store.bindings()?;
    extensions.cancel_agent(&binding.agent_id);
    drop(store);
    let _ = app.emit("extension-bindings", &bindings);
    Ok(bindings)
}
pub(crate) fn has_binding(app: &AppHandle, agent_id: &str, kind: &str) -> Result<bool, String> {
    let binding = app
        .state::<ExtensionHost>()
        .store()?
        .binding(agent_id, kind)?;
    Ok(binding.is_some_and(|b| b.plugin_id.is_some() && b.service_id.is_some()))
}

// Dropping the calling future revokes authority immediately. The blocking task
// retains the registry entry until the child process has actually drained.
struct FutureCancellation {
    signal: Arc<AtomicBool>,
    armed: bool,
}
impl Drop for FutureCancellation {
    fn drop(&mut self) {
        if self.armed {
            self.signal.store(true, Ordering::SeqCst);
        }
    }
}
struct RunRegistration {
    app: AppHandle,
    id: String,
    success: bool,
}
impl Drop for RunRegistration {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.app.state::<ExtensionHost>().runs.lock() {
            runs.remove(&self.id);
        }
        let _ = self.app.emit(
            "extension-run-ended",
            json!({"runId":self.id,"success":self.success}),
        );
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        if let Some(region) = self.region.take() {
            // Chat tools borrow the run's session. Ending one worker must not
            // revoke its target or release Escape while the next tool runs.
            if self.chat_region.is_none() {
                region.revoke();
            }
            drop(region);
        }
        #[cfg(windows)]
        self.escape.release();
        #[cfg(windows)]
        if let Some(ticket) = self.focus_return.take() {
            ticket.finalize();
        }
    }
}
async fn run_context(
    app: AppHandle,
    mut context: Context,
    service: String,
    input: Value,
) -> Result<Value, String> {
    context.check(&app)?;
    let registration = register_run(&app, &context)?;
    let mut cancellation = FutureCancellation {
        signal: context.canceled.clone(),
        armed: true,
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut registration = registration;
        let result = execute(&app, &mut context, &service, input);
        registration.success = result.is_ok();
        result
    })
    .await
    .map_err(|_| "扩展执行中断".to_string());
    cancellation.armed = false;
    result?
}
fn register_run(app: &AppHandle, context: &Context) -> Result<RunRegistration, String> {
    let extensions = app.state::<ExtensionHost>();
    let mut runs = extensions.runs.lock().map_err(|_| "扩展运行状态不可用")?;
    if runs.len() >= 8 {
        return Err("扩展运行过多".into());
    }
    runs.insert(
        context.run_id.clone(),
        ActiveRun {
            plugin: context.record.manifest.id.clone(),
            scope: context.scope.clone(),
            canceled: context.canceled.clone(),
            surface: context.surface_owner.clone(),
            surface_window: context.surface_window.clone(),
        },
    );
    Ok(RunRegistration {
        app: app.clone(),
        id: context.run_id.clone(),
        success: false,
    })
}

pub async fn run_bound_service(
    app: AppHandle,
    agent_id: String,
    kind: &str,
    scene_id: String,
    input: Value,
) -> Result<Option<Value>, String> {
    if !["agent", "memory"].contains(&kind) {
        return Err("扩展提供方类型无效".into());
    }
    let (binding, record, service) = {
        let extensions = app.state::<ExtensionHost>();
        let store = extensions.store()?;
        let Some(binding) = store.binding(&agent_id, kind)? else {
            return Ok(None);
        };
        let (Some(plugin), Some(service)) =
            (binding.plugin_id.as_ref(), binding.service_id.as_ref())
        else {
            return Ok(None);
        };
        let record = store.get(plugin)?;
        let expected_kind = if kind == "agent" {
            mewu_extension::ServiceKind::Agent
        } else {
            mewu_extension::ServiceKind::Memory
        };
        if record.state != "enabled"
            || !record
                .services
                .iter()
                .any(|s| &s.id == service && s.kind == expected_kind)
        {
            return Err("已选的扩展提供方不可用".into());
        }
        (binding.clone(), record, service.clone())
    };
    let prepared = crate::extension_input::prepare(input)?;
    let images = prepared
        .input
        .get("images")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let context = Context {
        record,
        run_id: uuid::Uuid::new_v4().to_string(),
        canceled: Arc::new(AtomicBool::new(false)),
        scene_id: Some(scene_id),
        region_id: None,
        scope: agent_id,
        region: None,
        chat_region: None,
        native_run: None,
        model_connection: None,
        model_lease: None,
        tool_preparing: false,
        #[cfg(windows)]
        focus_return: None,
        #[cfg(windows)]
        escape: crate::extension_escape::RunEscapeLease::default(),
        frames: prepared.frames,
        default_messages: prepared.messages,
        history: prepared.history,
        default_images: images,
        binding: Some((kind.into(), binding.revision)),
        delegated_permissions: None,
        authority_chain: vec![],
        public_reasoning: None,
        memory_enabled_required: kind == "memory",
        depth: 0,
                object_access: None,
                processes: Default::default(),
        surface_owner: None,
        surface_window: None,
    };
    run_context(app, context, service, prepared.input)
        .await
        .map(Some)
}
pub(crate) struct ExternalAgentResult {
    pub output: Value,
    pub reasoning: Option<String>,
}

fn validate_tool_record(
    store: &Store,
    descriptor: &crate::extension_tools::ToolDescriptor,
) -> Result<Record, String> {
    let record = store.get(&descriptor.plugin_id)?;
    record.ensure_supported()?;
    if let Some(reason) = service_platform_error(&descriptor.service, mewu_extension::current_plugin_platform()) { return Err(reason.into()); }
    if record.state != "enabled"
        || record.revision != descriptor.revision
        || record.manifest.version != descriptor.version
        || record.sha256 != descriptor.sha256
        || descriptor.service.kind != mewu_extension::ServiceKind::Tool
        || !record.services.contains(&descriptor.service)
    {
        return Err("扩展工具目录或授权已变化".into());
    }
    store.validate_dependencies(&record.manifest)?;
    Ok(record)
}

pub(crate) fn tool_catalog(
    app: &AppHandle,
    scene_id:&str,
) -> Result<Vec<crate::extension_tools::ToolDescriptor>, String> {
    let (agent_id,memory_enabled)={let host=app.state::<Host>();let engine=host.lock()?;
        let scene=engine.store.scene(scene_id).map_err(|e|e.to_string())?;
        let agent=engine.store.agent_profile(&scene.agent_id).map_err(|e|e.to_string())?;
        (agent.id.clone(),agent.memory_enabled)};
    let extensions = app.state::<ExtensionHost>();
    let store = extensions.store()?;
    let mut tools = vec![];
    for record in store.list()? {
        if record.state != "enabled" || record.ensure_supported().is_err() {
            continue;
        }
        // An installed but unsatisfied dependency cannot advertise executable
        // tools. The same check runs again under the dispatch lock.
        if store.validate_dependencies(&record.manifest).is_err() {
            continue;
        }
        for service in &record.services {
            if service.kind == mewu_extension::ServiceKind::Tool && service_platform_error(service, mewu_extension::current_plugin_platform()).is_none() {
                let descriptor=crate::extension_tools::ToolDescriptor {
                    plugin_id: record.manifest.id.clone(),
                    revision: record.revision,
                    version: record.manifest.version.clone(),
                    sha256: record.sha256.clone(),
                    service: service.clone(),
                };
                if (descriptor.requires_memory()? || descriptor.required_provider()?==Some("memory")) && !memory_enabled {continue;}
                if let Some(kind)=descriptor.required_provider()? {
                    if !store.binding(&agent_id,kind)?.is_some_and(|b|provider_binding_matches(&record,&b,kind)) {continue;}
                }
                tools.push(descriptor);
            }
        }
    }
    Ok(tools)
}

pub(crate) async fn invoke_tool_for_run(
    app: AppHandle,
    descriptor: crate::extension_tools::ToolDescriptor,
    scene_id: String,
    run_id: String,
    canceled: Arc<AtomicBool>,
    chat_region: Option<Arc<crate::extension_chat_region::ChatRegionScope>>,
    model_lease: Option<Arc<crate::extension_model_lease::ModelLease>>,
    input: Value,
    lease: mewu_core::DispatchLease,
) -> crate::extension_tools::ToolInvocationOutcome {
    use crate::extension_tools::ToolInvocationOutcome;
    use mewu_core::{NoResponseReason, NotSentReason};
    let prepare = (|| -> Result<Context, NotSentReason> {
        if canceled.load(Ordering::SeqCst) {
            return Err(NotSentReason::RunEnded);
        }
        if chat_region.as_ref().is_some_and(|s| {
            s.scene_id() != scene_id
                || s.run_id() != run_id
                || !Arc::ptr_eq(&s.cancel_token(), &canceled)
        }) {
            return Err(NotSentReason::AuthorityChanged);
        }
        let prepared_agent={let host=app.state::<Host>();let engine=host.lock().map_err(|_|NotSentReason::PreparationFailed)?;
            engine.store.scene(&scene_id).map_err(|_|NotSentReason::RunEnded)?.agent_id.clone()};
        let (record,provider_binding) = {
            let extensions = app.state::<ExtensionHost>();
            let store = extensions
                .store()
                .map_err(|_| NotSentReason::PreparationFailed)?;
            let record=validate_tool_record(&store, &descriptor).map_err(|_| NotSentReason::CatalogChanged)?;
            let binding=match descriptor.required_provider().map_err(|_|NotSentReason::CatalogChanged)? {
                Some(kind)=>{
                    let b=store.binding(&prepared_agent,kind).map_err(|_|NotSentReason::PreparationFailed)?.ok_or(NotSentReason::AuthorityChanged)?;
                    if !provider_binding_matches(&record,&b,kind){return Err(NotSentReason::AuthorityChanged);}
                    Some((kind.to_owned(),b.revision))
                },None=>None,
            };
            (record,binding)
        };
        if descriptor
            .requires_region()
            .map_err(|_| NotSentReason::CatalogChanged)?
            && chat_region.is_none()
        {
            return Err(NotSentReason::AuthorityChanged);
        }
        if let Some(schema) = &descriptor.service.input_schema {
            let validator =
                jsonschema::validator_for(schema).map_err(|_| NotSentReason::CatalogChanged)?;
            if !validator.is_valid(&input) {
                return Err(NotSentReason::InvalidArguments);
            }
        }
        let host = app.state::<Host>();
        host.exit
            .ensure_background()
            .map_err(|_| NotSentReason::RunEnded)?;
        let engine = host.lock().map_err(|_| NotSentReason::PreparationFailed)?;
        if !crate::run_is_authorized(&engine, &scene_id, &run_id, None) {
            return Err(NotSentReason::RunEnded);
        }
        if let Some(scope) = &chat_region {
            scope
                .check_engine(&engine)
                .map_err(|_| NotSentReason::AuthorityChanged)?;
        }
        let agent_id = engine
            .store
            .scene(&scene_id)
            .map_err(|_| NotSentReason::RunEnded)?
            .agent_id
            .clone();
        if agent_id!=prepared_agent{return Err(NotSentReason::AuthorityChanged);}
        let requires_memory=descriptor.requires_memory().map_err(|_|NotSentReason::CatalogChanged)?
            || provider_binding.as_ref().is_some_and(|(kind,_)|kind=="memory");
        Ok(Context {
            record,
            run_id: uuid::Uuid::new_v4().to_string(),
            canceled: canceled.clone(),
            scene_id: Some(scene_id.clone()),
            region_id: chat_region.as_ref().map(|s| s.region_id().into()),
            scope: agent_id,
            region: None,
            chat_region,
            native_run: Some((scene_id.clone(), run_id.clone())),
            model_connection: model_lease.as_ref().map(|lease| lease.connection().clone()),
            model_lease,
            tool_preparing: true,
            #[cfg(windows)]
            escape: crate::extension_escape::RunEscapeLease::default(),
            #[cfg(windows)]
            focus_return: None,
            frames: HashMap::new(),
            default_messages: vec![],
            history: None,
            default_images: vec![],
            binding: provider_binding,
            delegated_permissions: None,
            authority_chain: vec![],
            public_reasoning: None,
            memory_enabled_required: requires_memory,
            depth: 0,
                object_access: None,
                processes: Default::default(),
        surface_owner: None,
        surface_window: None,
        })
    })();
    let mut context = match prepare {
        Ok(context) => context,
        Err(reason) => return ToolInvocationOutcome::NotSent(reason),
    };
    if context.check(&app).is_err() {
        return ToolInvocationOutcome::NotSent(NotSentReason::AuthorityChanged);
    }
    let registration = match register_run(&app, &context) {
        Ok(registration) => registration,
        Err(_) => return ToolInvocationOutcome::NotSent(NotSentReason::PreparationFailed),
    };
    let sent = Arc::new(AtomicBool::new(false));
    let safe_failure = Arc::new(Mutex::new(None));
    let dispatch = ToolDispatch {
        descriptor: descriptor.clone(),
        scene_id,
        run_id,
        lease,
        sent: sent.clone(),
        safe_failure: safe_failure.clone(),
    };
    let mut cancellation = FutureCancellation {
        signal: canceled.clone(),
        armed: true,
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut registration = registration;
        let result = execute_with_dispatch(
            &app,
            &mut context,
            &descriptor.service.id,
            input,
            Some(&dispatch),
        );
        registration.success = result.is_ok();
        result
    })
    .await;
    cancellation.armed = false;
    match result {
        Ok(Ok(output)) => ToolInvocationOutcome::Responded(output),
        _ if sent.load(Ordering::SeqCst) => ToolInvocationOutcome::Unknown {
            reason: if canceled.load(Ordering::SeqCst) {
                NoResponseReason::Canceled
            } else {
                NoResponseReason::IncompleteResponse
            },
            safe_diagnostic: safe_failure
                .lock()
                .ok()
                .and_then(|diagnostic| diagnostic.clone()),
        },
        _ => ToolInvocationOutcome::NotSent(if canceled.load(Ordering::SeqCst) {
            NotSentReason::RunEnded
        } else {
            NotSentReason::PreparationFailed
        }),
    }
}

pub(crate) async fn run_external_agent(
    app: AppHandle,
    provider: ExternalAgentProvider,
    scene_id: String,
    run_id: String,
    chat_region: Option<Arc<crate::extension_chat_region::ChatRegionScope>>,
    model_connection: Option<mewu_core::ConnectionProfile>,
    input: Value,
) -> Result<ExternalAgentResult, String> {
    let authority = &provider.authority;
    if authority.canceled.load(Ordering::SeqCst)
        || provider.agent_id != authority.agent_id
        || provider.record.manifest.id != authority.plugin_id
        || provider.record.revision != authority.plugin_revision
        || provider.record.sha256 != authority.sha256
        || provider.service_id != authority.service_id
    {
        return Err("Agent 扩展授权已变化".into());
    }
    let prepared = crate::extension_input::prepare(input)?;
    let images = prepared
        .input
        .get("images")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let reasoning = Arc::new(Mutex::new(String::new()));
    let context = Context {
        record: provider.record,
        run_id: run_id.clone(),
        canceled: authority.canceled.clone(),
        scene_id: Some(scene_id.clone()),
        region_id: chat_region.as_ref().map(|s| s.region_id().to_owned()),
        scope: provider.agent_id,
        region: None,
        chat_region,
        native_run: Some((scene_id, run_id)),
        model_connection,
        model_lease: None,
        tool_preparing: false,
        #[cfg(windows)]
        focus_return: None,
        #[cfg(windows)]
        escape: crate::extension_escape::RunEscapeLease::default(),
        frames: prepared.frames,
        default_messages: prepared.messages,
        history: prepared.history,
        default_images: images,
        binding: Some(("agent".into(), authority.binding_revision)),
        delegated_permissions: None,
        authority_chain: vec![],
        public_reasoning: Some(reasoning.clone()),
        memory_enabled_required: false,
        depth: 0,
                object_access: None,
                processes: Default::default(),
        surface_owner: None,
        surface_window: None,
    };
    context.check(&app)?;
    let output = run_context(app, context, provider.service_id, prepared.input).await?;
    let reasoning = reasoning
        .lock()
        .map_err(|_| "Agent 思考状态不可用")?
        .clone();
    Ok(ExternalAgentResult {
        output,
        reasoning: if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        },
    })
}
fn execute(
    app: &AppHandle,
    context: &mut Context,
    service_id: &str,
    input: Value,
) -> Result<Value, String> {
    execute_with_dispatch(app, context, service_id, input, None)
}
struct ToolDispatch {
    descriptor: crate::extension_tools::ToolDescriptor,
    scene_id: String,
    run_id: String,
    lease: mewu_core::DispatchLease,
    sent: Arc<AtomicBool>,
    safe_failure: Arc<Mutex<Option<SafeToolDiagnostic>>>,
}
fn execute_with_dispatch(
    app: &AppHandle,
    context: &mut Context,
    service_id: &str,
    input: Value,
    dispatch: Option<&ToolDispatch>,
) -> Result<Value, String> {
    if let Some(service) = context.record.services.iter().find(|s| s.id == service_id) {
        if let Some(reason) = service_platform_error(service, mewu_extension::current_plugin_platform()) { return Err(reason.into()); }
    }
    let (mut worker, registrations) = start_worker(app, context)?;
    let registration = registrations
        .iter()
        .find(|s| s.id == service_id)
        .ok_or("插件没有注册此服务")?;
    if let Some(reason) = service_platform_error(registration, mewu_extension::current_plugin_platform()) {
        return Err(reason.into());
    }
    if let Some(schema) = &registration.input_schema {
        let validator = jsonschema::validator_for(schema).map_err(|_| "插件服务参数规范无效")?;
        if !validator.is_valid(&input) {
            return Err("插件参数不符合服务规范".into());
        }
    }
    if let Some(dispatch) = dispatch {
        if registration != &dispatch.descriptor.service {
            return Err("扩展工具目录已变化".into());
        }
        context.check(app)?;
        let extensions = app.state::<ExtensionHost>();
        let store = extensions.store()?;
        validate_tool_record(&store, &dispatch.descriptor)?;
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_background()?;
        if context.canceled.load(Ordering::SeqCst)
            || !crate::run_is_authorized(&engine, &dispatch.scene_id, &dispatch.run_id, None)
        {
            return Err("Agent 运行已结束".into());
        }
        if let Some(scope) = &context.chat_region {
            scope.check_engine(&engine)?;
        }
        let commit = engine
            .store
            .mark_extension_tool_dispatched(&dispatch.lease, &dispatch.descriptor.binding())
            .map_err(crate::journal_runtime::store_error)?;
        crate::journal_runtime::emit_changed(
            app,
            &dispatch.scene_id,
            &dispatch.run_id,
            commit.journal_revision,
        );
        dispatch.sent.store(true, Ordering::SeqCst);
        context.tool_preparing = false;
    }
    let _=app.emit_to("space","extension-run-started",json!({"runId":context.run_id,"pluginId":context.record.manifest.id,"serviceId":service_id,"title":registration.title}));
    let id = uuid::Uuid::new_v4().to_string();
    let memory_origin=memory_event_origin(registration.kind,context.binding.as_ref(),&input);
    worker.send(&WorkerRequest::Invoke{id:id.clone(),service_id:service_id.into(),input,context:json!({"runId":context.run_id,"sceneId":context.scene_id,"regionId":context.region_id,"agentId":context.scope,"platform":std::env::consts::OS,"history":context.history,"memoryOrigin":memory_origin})})?;
    let result = worker.receive(app, context, &id, false);
    if let Some(dispatch) = dispatch {
        if let Ok(mut diagnostic) = dispatch.safe_failure.lock() {
            *diagnostic = worker.safe_failure.take();
        }
    }
    result
}

fn memory_event_origin(kind:mewu_extension::ServiceKind,binding:Option<&(String,u64)>,input:&Value)->Option<&'static str>{
    (kind==mewu_extension::ServiceKind::Memory && binding.is_some_and(|(kind,_)|kind=="memory")
        && input.get("operation").and_then(Value::as_str)==Some("retain")).then_some("completedTurn")
}
fn provider_binding_matches(record:&Record,binding:&Binding,kind:&str)->bool{
    let expected=match kind {"memory"=>mewu_extension::ServiceKind::Memory,"agent"=>mewu_extension::ServiceKind::Agent,_=>return false};
    binding.plugin_id.as_deref()==Some(&record.manifest.id)&&record.services.iter().any(|s|Some(s.id.as_str())==binding.service_id.as_deref()&&s.kind==expected)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少参数：{key}"))
}
fn live_store<T>(
    app: &AppHandle,
    context: &Context,
    operation: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let extensions = app.state::<ExtensionHost>();
    let mut store = extensions.store()?;
    if context.canceled.load(Ordering::SeqCst) {
        return Err("扩展运行已停止".into());
    }
    let record = store.get(&context.record.manifest.id)?;
    if record.state != "enabled" || record.revision != context.record.revision {
        return Err("扩展授权已撤销".into());
    }
    for (id, revision) in &context.authority_chain {
        let caller = store.get(id)?;
        if caller.state != "enabled" || caller.revision != *revision {
            return Err("调用方扩展授权已撤销".into());
        }
    }
    if let Some((kind, revision)) = &context.binding {
        let binding = store
            .binding(&context.scope, kind)?
            .ok_or("扩展绑定已撤销")?;
        if binding.revision != *revision {
            return Err("扩展绑定已变化".into());
        }
    }
    match &context.surface_owner {
        Some((id,owner))=>extensions.surfaces.with_owner(id,owner,||operation(&mut store)),
        None=>operation(&mut store)
    }
}
fn bridge(
    app: &AppHandle,
    context: &mut Context,
    method: &str,
    params: Value,
) -> Result<Value, HostCallFailure> {
    let mut diagnostic = None;
    bridge_impl(app, context, method, params, &mut diagnostic)
        .map_err(|message| HostCallFailure::from_error(method, &message, diagnostic))
}
fn tool_initialization_read(method: &str) -> bool {
    matches!(method, "runtime.describe" | "configuration.get" | "storage.get" | "storage.list" | "services.list" | "objects.library.describe")
}
pub(crate) fn runtime_capabilities()->Value {
    json!({"hostApi":mewu_extension::HOST_API,"capabilities":["ui.settings","configuration.shared","secrets.network","memory.archive","process.jsonlines","process.frame-bindings"]})
}

fn bridge_impl(
    app: &AppHandle,
    context: &mut Context,
    method: &str,
    params: Value,
    diagnostic: &mut Option<RegionDiagnostic>,
) -> Result<Value, String> {
    context.check(app)?;
    if context.tool_preparing && !tool_initialization_read(method) {
        return Err("工具初始化阶段不能执行宿主操作".into());
    }
    if method=="runtime.describe" {
        return Ok(runtime_capabilities());
    }
    let permission = match method {
        "storage.get" | "storage.list" => "storage.read",
        "configuration.get" => "configuration.read",
        "configuration.compareAndSet" => "configuration.write",
        "secrets.status" => "secrets.use",
        "secrets.set" => "secrets.write",
        "memory.archive.list"|"memory.archive.page"|"memory.archive.lookup"=>"memory.archive.read",
        "memory.archive.credential"|"memory.archive.activate"|"memory.archive.forget"=>"memory.archive.write",
        "storage.set" | "storage.delete" | "storage.compareAndSet" => "storage.write",
        "model.invoke" => "model.invoke",
        "region.observe" => "region.observe",
        "region.action" => "region.input",
        "services.list" | "services.call" => "services.call",
        "ui.render" => "ui.render",
        "objects.library.describe" | "objects.library.get" | "objects.library.preview" | "objects.library.commit" | "objects.library.restore" => "objects.library",
        "process.resolve" | "process.open" | "process.write" | "process.read" | "process.close" => "process.rpc",
        "network.fetch" => "network.fetch",
        "events.emit" => "events.emit",
        _ => return Err("宿主不支持此扩展接口".into()),
    };
    context.permission(permission)?;
    if matches!(permission, "region.observe" | "region.input") && !region::RegionSession::supported() {
        return Err("当前平台尚不支持区域操作".into());
    }
    let result = match method {
        "memory.archive.list"|"memory.archive.page"|"memory.archive.lookup"|"memory.archive.credential"|"memory.archive.activate"|"memory.archive.forget"=>memory_archive::call(app,context,method,params),
        "secrets.status" => secrets::status(app,context,&params),
        "secrets.set" => secrets::set(app,context,params),
        "configuration.get" => live_store(app,context,|store|store.value(&context.record.manifest.id,"plugin-configuration",string(&params,"key")?)),
        "configuration.compareAndSet" => {
            if !context.surface_owner.as_ref().is_some_and(|(_,owner)|owner.scene.is_empty()) {
                return Err("请在插件设置界面修改配置".into());
            }
            Ok(Value::Bool(live_store(app,context,|store|store.compare_and_set(
                &context.record.manifest.id,"plugin-configuration",string(&params,"key")?,
                params.get("expected").ok_or("缺少比较值")?,params.get("value").ok_or("缺少插件配置")?
            ))?))
        },
        "objects.library.describe" => Ok(crate::extension_objects_host::describe()),
        "objects.library.get" | "objects.library.preview" | "objects.library.commit" | "objects.library.restore" => {
            let access=context.object_access.as_mut().ok_or("此运行没有对象库界面授权")?;
            crate::extension_objects_host::bridge(app, access, method, params)
        }
        "storage.get" => live_store(app, context, |store| {
            store.value(
                &context.record.manifest.id,
                &context.scope,
                string(&params, "key")?,
            )
        }),
        "storage.set" => {
            live_store(app, context, |store| {
                store.set_value(
                    &context.record.manifest.id,
                    &context.scope,
                    string(&params, "key")?,
                    params.get("value").ok_or("缺少插件数据")?,
                )
            })?;
            Ok(Value::Null)
        }
        "storage.delete" => {
            live_store(app, context, |store| {
                store.delete_value(
                    &context.record.manifest.id,
                    &context.scope,
                    string(&params, "key")?,
                )
            })?;
            Ok(Value::Null)
        }
        "storage.compareAndSet" => Ok(Value::Bool(live_store(app, context, |store| {
            store.compare_and_set(
                &context.record.manifest.id,
                &context.scope,
                string(&params, "key")?,
                params.get("expected").ok_or("缺少比较值")?,
                params.get("value").ok_or("缺少插件数据")?,
            )
        })?)),
        "storage.list" => live_store(app, context, |store| {
            store.keys(
                &context.record.manifest.id,
                &context.scope,
                params.get("prefix").and_then(Value::as_str).unwrap_or(""),
                params.get("cursor").and_then(Value::as_str).unwrap_or(""),
            )
        }),
        "services.list" => {
            let records = app.state::<ExtensionHost>().store()?.list()?;
            Ok(Value::Array(records.iter().filter(|r|r.state=="enabled" && r.ensure_supported().is_ok()).flat_map(|r|r.services.iter().filter(|s|service_platform_error(s, mewu_extension::current_plugin_platform()).is_none() && params.get("kind").is_none_or(|k|serde_json::to_value(s.kind).ok().as_ref()==Some(k))).map(|s|json!({"pluginId":r.manifest.id,"revision":r.revision,"serviceId":s.id,"kind":s.kind,"title":s.title}))).collect()))
        }
        "services.call" => {
            if context.depth >= 3 {
                return Err("插件服务调用层级过深".into());
            }
            let target = string(&params, "serviceId")?;
            let (plugin_id, service_id) = target
                .split_once('/')
                .ok_or("服务地址须为 pluginId/serviceId")?;
            let record = app.state::<ExtensionHost>().store()?.get(plugin_id)?;
            if plugin_id == context.record.manifest.id {
                return Err("插件不能通过宿主递归调用自身".into());
            }
            let requirement = context
                .record
                .manifest
                .dependencies
                .get(plugin_id)
                .ok_or("调用其他插件前必须声明依赖")?;
            if !semver::VersionReq::parse(requirement)
                .map_err(|_| "依赖版本规范无效")?
                .matches(
                    &semver::Version::parse(&record.manifest.version)
                        .map_err(|_| "依赖版本无效")?,
                )
            {
                return Err("依赖插件版本不匹配".into());
            }
            let delegated_permissions = record
                .manifest
                .permissions
                .iter()
                .filter(|p| context.permission(p).is_ok())
                .cloned()
                .collect();
            let mut authority_chain = context.authority_chain.clone();
            authority_chain.push((context.record.manifest.id.clone(), context.record.revision));
            let mut child = Context {
                record,
                run_id: context.run_id.clone(),
                canceled: context.canceled.clone(),
                scene_id: context.scene_id.clone(),
                region_id: None,
                scope: context.scope.clone(),
                region: None,
                chat_region: None,
                native_run: context.native_run.clone(),
                model_connection: context.model_connection.clone(),
                model_lease: context.model_lease.clone(),
                tool_preparing: false,
                #[cfg(windows)]
                focus_return: None,
                #[cfg(windows)]
                escape: crate::extension_escape::RunEscapeLease::default(),
                frames: HashMap::new(),
                default_messages: vec![],
                history: context.history.clone(),
                default_images: vec![],
                binding: None,
                delegated_permissions: Some(delegated_permissions),
                authority_chain,
                public_reasoning: context.public_reasoning.clone(),
                memory_enabled_required: context.memory_enabled_required,
                depth: context.depth + 1,
                object_access: None,
                processes: Default::default(),
                surface_owner: context.surface_owner.clone(),
                surface_window: context.surface_window.clone(),
            };
            execute(
                app,
                &mut child,
                service_id,
                params.get("input").cloned().unwrap_or(Value::Null),
            )
        }
        "region.observe" | "region.action" => {
            if params.get("regionId").and_then(Value::as_str) != context.region_id.as_deref()
                || context.region_id.is_none()
            {
                return Err("插件不能访问未授权的选区".into());
            }
            if context.region.is_none() {
                context.region = Some(begin_region(app, context, diagnostic)?);
            }
            let region = context.region.as_ref().unwrap().clone();
            if method == "region.action" {
                let action =
                    serde_json::from_value(params.get("action").cloned().ok_or("缺少区域动作")?)
                        .map_err(|_| "区域动作无效")?;
                region
                    .execute(action)
                    .map_err(|e| record_region_error(diagnostic, e))?;
                Ok(json!({"ok":true}))
            } else {
                if context.frames.len() >= 32 {
                    context.frames.clear();
                }
                let frame = region
                    .observe()
                    .map_err(|e| record_region_error(diagnostic, e))?;
                let id = uuid::Uuid::new_v4().to_string();
                let info = json!({"artifactId":id,"width":frame.width,"height":frame.height,"sha256":frame.sha256,"mimeType":"image/png"});
                context.frames.insert(id, frame);
                Ok(info)
            }
        }
        "model.invoke" => model_invoke(app, context, params),
        "ui.render" => {
            let html = string(&params, "html")?;
            let scene = context
                .scene_id
                .as_deref()
                .ok_or("此服务没有会话界面授权")?;
            context.check(app)?;
            let value = crate::extension_ui::render(
                app,
                scene,
                &context.record.manifest.id,
                context.record.revision,
                context.surface_owner.as_ref(),
                html,
                params
                    .get("width")
                    .and_then(Value::as_u64)
                    .unwrap_or(400)
                    .clamp(80, 1600) as u32,
                params
                    .get("height")
                    .and_then(Value::as_u64)
                    .unwrap_or(300)
                    .clamp(40, 1200) as u32,
            )?;
            context.check(app)?;
            Ok(value)
        }
        "events.emit" => {
            let name = string(&params, "name")?;
            if matches!(name, "agent.text" | "agent.reasoning") {
                let text = params
                    .get("data")
                    .and_then(|value| {
                        value
                            .as_str()
                            .or_else(|| value.get("text").and_then(Value::as_str))
                    })
                    .ok_or("Agent 流事件缺少文本")?;
                if text.len() > 64 * 1024 {
                    return Err("Agent 单次流事件过大".into());
                }
                let scene = context
                    .scene_id
                    .as_deref()
                    .ok_or("Agent 流事件没有会话授权")?;
                let host = app.state::<Host>();
                let engine = host.lock()?;
                if !crate::run_is_authorized(&engine, scene, &context.run_id, None) {
                    return Err("Agent 对话运行授权已撤销".into());
                }
                if name == "agent.reasoning" {
                    if let Some(reasoning) = &context.public_reasoning {
                        let mut reasoning = reasoning.lock().map_err(|_| "Agent 思考状态不可用")?;
                        if reasoning.len().saturating_add(text.len()) > 2 * 1024 * 1024 {
                            return Err("Agent 公开思考内容过长".into());
                        }
                        reasoning.push_str(text);
                    }
                }
                let mut event = json!({"sceneId":scene,"runId":context.run_id,"status":"running"});
                event[if name == "agent.text" {
                    "text"
                } else {
                    "reasoning"
                }] = Value::String(text.into());
                crate::emit_run_event(app, event).map_err(|_| "无法发送 Agent 流事件")?;
            }
            app.emit("extension-event",json!({"pluginId":context.record.manifest.id,"runId":context.run_id,"name":string(&params,"name")?,"data":params.get("data")})).map_err(|_|"无法发送插件事件")?;
            Ok(Value::Null)
        }
        "process.resolve" | "process.open" | "process.write" | "process.read" | "process.close" => {
            let mut params=params;
            if method=="process.open" && params.pointer("/command/cwd").is_none_or(Value::is_null) {
                let host=app.state::<Host>();
                let root=host.assets.parent().ok_or("插件工作目录不可用")?.join("plugin-workspaces").join(&context.record.manifest.id);
                std::fs::create_dir_all(&root).map_err(|_|"无法创建插件工作目录")?;
                let command=params.get_mut("command").and_then(Value::as_object_mut).ok_or("缺少进程配置")?;
                command.insert("cwd".into(),Value::String(root.to_string_lossy().into_owned()));
            }
            if method=="process.write" {
                if let Some(bindings)=params.as_object_mut().and_then(|value|value.remove("bindings")) {
                    context.permission("media.read")?;
                    crate::extension_process::bind_frames(&mut params,bindings,&context.frames)?;
                }
            }
            let value=context.processes.call(method,params,context.canceled.clone())?;context.check(app)?;Ok(value)
        },
        "network.fetch" => network_fetch(app, context, params),
        _ => unreachable!(),
    };
    context.check(app)?;
    result
}
// External Agent persistence intentionally has no fabricated model origin. An
// optional real host-model choice is therefore retained only in this native
// invocation and revoked through the existing shared cancellation signal.
fn guard_model_connection(
    store: &mewu_core::Store,
    scene_id: &str,
    expected: Option<&mewu_core::ConnectionProfile>,
    canceled: &AtomicBool,
) -> Result<(), String> {
    if let Some(expected) = expected {
        if store.connection_for_scene(scene_id).as_ref().ok() != Some(expected) {
            canceled.store(true, Ordering::SeqCst);
            return Err("本轮模型连接已更改，请重新发送".into());
        }
    }
    Ok(())
}

fn model_canonical_body(
    connection: &mewu_core::ConnectionProfile,
    messages: Vec<Value>,
) -> Result<Value, String> {
    let mut body = json!({"model":connection.model,"messages":messages,"stream":true});
    crate::connection_host::apply_parameters(&mut body, connection)?;
    Ok(body)
}
fn model_invoke(app: &AppHandle, context: &Context, params: Value) -> Result<Value, String> {
    let scene_id = context
        .scene_id
        .as_deref()
        .ok_or("此服务没有会话模型授权")?;
    let host = app.state::<Host>();
    let connection = {
        let engine = host.lock()?;
        guard_model_connection(
            &engine.store,
            scene_id,
            context.model_connection.as_ref(),
            &context.canceled,
        )?;
        if let Some(connection) = &context.model_connection {
            connection.clone()
        } else {
            engine
                .store
                .connection_for_scene(scene_id)
                .map_err(|e| e.to_string())?
        }
    };
    let transport = crate::connection_host::transport_for(&connection)?;
    // Native Agent tools borrow the preflight key for this exact run. Opening
    // the OS keychain here can block inside a finite plugin worker deadline.
    // Standalone services retain their independent credential read path.
    let fallback_key = if context.model_lease.is_none() {
        Some(if connection.advanced.auth_mode == mewu_core::ConnectionAuthMode::None {
            zeroize::Zeroizing::new(String::new())
        } else {
            crate::credentials::read_profile(&host.root, &connection)?
        })
    } else { None };
    let key = if let Some(lease) = &context.model_lease {
        lease.key(scene_id, context.native_run.as_ref(), &context.canceled, &connection)?
    } else {
        fallback_key.as_ref().ok_or("模型授权缺失")?.as_str()
    };
    if key.is_empty() && connection.advanced.auth_mode != mewu_core::ConnectionAuthMode::None {
        return Err("请先配置模型密钥".into());
    }
    let mut messages = match params.get("messages") {
        Some(messages) => messages.as_array().cloned().ok_or("模型消息必须是列表")?,
        None => context.default_messages.clone(),
    };
    if messages.len() > 128 {
        return Err("插件模型上下文过长".into());
    }
    for message in &messages {
        if !matches!(
            message.get("role").and_then(Value::as_str),
            Some("user" | "assistant" | "system")
        ) || !message.get("content").is_some_and(Value::is_string)
        {
            return Err("模型消息仅接受明确的角色和文本".into());
        }
    }
    // Only the public text protocol crosses this API, never caller-supplied
    // private continuation, tool authority or transport fields.
    messages = messages
        .iter()
        .map(|m| json!({"role":m["role"],"content":m["content"]}))
        .collect();
    let mut content = vec![
        json!({"type":"text","text":params.get("prompt").and_then(Value::as_str).unwrap_or("")}),
    ];
    let images = if let Some(images) = params.get("images") {
        images
            .as_array()
            .ok_or("模型图片必须是引用列表")?
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or("图片引用无效"))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        context.default_images.clone()
    };
    if images.len() > 8 {
        return Err("单次模型图片过多".into());
    }
    for id in images {
        let frame = context.frames.get(&id).ok_or("图片不属于当前插件运行")?;
        content.push(json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(&frame.png))}}));
    }
    messages.push(json!({"role":"user","content":content}));
    let canonical = model_canonical_body(&connection, messages)?;
    let body = crate::provider_transport::request_body(transport.protocol, canonical)?;
    tauri::async_runtime::block_on(async {
        context.check(app)?;
        let request = crate::provider_transport::stream_turn(&transport, key, body, |text| {
            if context.check(app).is_ok() {
                let _ = app.emit_to(
                    "space",
                    "extension-model-delta",
                    json!({"runId":context.run_id,"text":text}),
                );
            }
        });
        tokio::pin!(request);
        loop {
            tokio::select! {result=&mut request=>{let turn=result?;context.check(app)?;return Ok(json!({"content":turn.content,"toolCalls":turn.tool_calls.into_iter().map(|c|json!({"id":c.id,"name":c.name,"arguments":c.arguments})).collect::<Vec<_>>()}))},_=tokio::time::sleep(Duration::from_millis(80))=>{context.check(app)?;}}
        }
    })
}
fn network_fetch(app: &AppHandle, context: &Context, params: Value) -> Result<Value, String> {
    let url = url::Url::parse(string(&params, "url")?).map_err(|_| "插件网络地址无效")?;
    if !(url.scheme() == "https"
        || (url.scheme() == "http" && crate::network_policy::is_local(&url)))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("插件网络接口只接受 HTTPS 或本机 HTTP 地址，不能在地址中包含凭据".into());
    }
    let method = params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET");
    if !["GET", "POST", "PUT", "DELETE"].contains(&method) {
        return Err("网络方法不支持".into());
    }
    let body = params.get("body").cloned();
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(values) = params.get("headers") {
        let values = values.as_object().ok_or("网络请求头必须是对象")?;
        if values.len() > 32 {
            return Err("网络请求头过多".into());
        }
        let mut total = 0usize;
        for (name, value) in values {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "网络请求头名称无效")?;
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-authorization"
                    | "cookie"
            ) {
                return Err("网络请求头不能覆盖传输或浏览器凭据".into());
            }
            let value = value.as_str().ok_or("网络请求头值必须是文本")?;
            total = total.saturating_add(name.as_str().len() + value.len());
            if total > 16 * 1024 {
                return Err("网络请求头过大".into());
            }
            headers.insert(
                name,
                reqwest::header::HeaderValue::from_str(value).map_err(|_| "网络请求头值无效")?,
            );
        }
    }
    if let Some(authorization) = params.get("authorization") {
        if headers.contains_key(reqwest::header::AUTHORIZATION) {
            return Err("不能同时提供认证头和托管凭据".into());
        }
        headers.insert(reqwest::header::AUTHORIZATION, secrets::authorization(app,context,authorization,&url)?);
    }
    tauri::async_runtime::block_on(async {
        context.check(app)?;
        let client = crate::network_policy::apply(reqwest::Client::builder(), Some(&url))?
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "网络接口不可用")?;
        let mut request = client
            .request(method.parse().map_err(|_| "网络方法无效")?, url)
            .headers(headers);
        if let Some(body) = body {
            request = if let Some(text) = body.as_str() {
                request.body(text.to_owned())
            } else {
                request.json(&body)
            }
        }
        let request = request.send();
        tokio::pin!(request);
        let response = loop {
            tokio::select! {r=&mut request=>break r.map_err(|_|"插件网络请求失败")?,_=tokio::time::sleep(Duration::from_millis(80))=>{context.check(app)?;}}
        };
        let status = response.status().as_u16();
        let mut stream = futures_util::StreamExt::boxed(response.bytes_stream());
        let mut bytes = Vec::new();
        loop {
            let part = tokio::select! {part=futures_util::StreamExt::next(&mut stream)=>part,_=tokio::time::sleep(Duration::from_millis(80))=>{context.check(app)?;continue;}};
            let Some(part) = part else { break };
            let part = part.map_err(|_| "网络读取失败")?;
            if bytes.len().saturating_add(part.len()) > 512 * 1024 {
                return Err("插件网络结果过大".into());
            }
            bytes.extend_from_slice(&part);
        }
        context.check(app)?;
        let text = String::from_utf8(bytes).map_err(|_| "插件网络结果不是文本")?;
        Ok(json!({"status":status,"body":text}))
    })
}

struct OverlayHooks {
    app: AppHandle,
    plugin: String,
    revision: u64,
    canceled: Arc<AtomicBool>,
    chat_region: Option<Weak<crate::extension_chat_region::ChatRegionScope>>,
    source: crate::extension_overlay_policy::RestoreSource,
    #[cfg(windows)]
    _escape: Arc<crate::extension_escape::EscapeLease>,
    #[cfg(windows)]
    focus_return: Option<Arc<crate::extension_focus::FocusTicket>>,
    hidden: Arc<Mutex<Vec<HiddenWindow>>>,
}
struct HiddenWindow {
    window: tauri::WebviewWindow,
    pin: Option<crate::pin_host::PinRestoreToken>,
}
impl region::RegionOverlayHooks for OverlayHooks {
    fn validate_source(&self, binding: &region::RegionBinding) -> Result<(), String> {
        if self.canceled.load(Ordering::SeqCst) {
            return Err("区域运行已停止".into());
        }
        if let Some(scope) = &self.chat_region {
            scope.upgrade().ok_or("区域会话已结束")?.check(&self.app)?;
        } else {
            let record = self
                .app
                .state::<ExtensionHost>()
                .store()?
                .get(&self.plugin)?;
            if record.revision != self.revision || record.state != "enabled" {
                return Err("插件授权已撤销".into());
            }
        }
        let host = self.app.state::<Host>();
        host.exit.ensure_background()?;
        let engine = host.lock()?;
        let scene = engine
            .store
            .scenes()
            .iter()
            .find(|s| s.id == binding.scene_id)
            .ok_or("会话已关闭")?;
        if engine.store.snapshot().active_scene_id != binding.scene_id
            || scene.closed
            || scene.background.as_ref().map(|b| &b.id) != Some(&binding.background_id)
            || scene.background.as_ref().is_none_or(|b| {
                b.width != Some(binding.source_width)
                    || b.height != Some(binding.source_height)
                    || b.capture_coordinates != Some(binding.coordinates)
                    || b.kind != mewu_core::AssetKind::Image
            })
        {
            return Err("截图来源已变化".into());
        }
        let selection = scene
            .regions
            .iter()
            .find(|r| r.id == binding.region_id)
            .ok_or("选区已删除")?;
        if !crate::extension_chat_region::selection_matches(
            selection,
            &binding.region_id,
            binding.selection,
            binding.drawing_revision,
            self.chat_region.is_some(),
        ) {
            return Err("选区已变化".into());
        }
        Ok(())
    }
    fn hide_owned_overlays(&self) -> Result<(), String> {
        let app = self.app.clone();
        let source = self.source.clone();
        #[cfg(windows)]
        let focus_return = self.focus_return.clone();
        let hidden = self.hidden.clone();
        let abandoned = Arc::new(AtomicBool::new(false));
        let ticket = abandoned.clone();
        let (send, receive) = mpsc::sync_channel(1);
        self.app
            .run_on_main_thread(move || {
                #[cfg(windows)]
                if let Some(ticket) = &focus_return {
                    ticket.before_hide();
                }
                let result = (|| {
                    for (label, window) in app.webview_windows() {
                        if label == "space"
                            || label == crate::frozen::LABEL
                            || label.starts_with("pin-")
                        {
                            let mut hidden = hidden.lock().map_err(|_| "覆盖层状态无效")?;
                            if ticket.load(Ordering::SeqCst) {
                                return Err("截图覆盖层操作已撤销");
                            }
                            if window.is_visible().unwrap_or(false) {
                                let pin = if crate::pin_host::is_pin_label(&label) {
                                    Some(
                                        crate::pin_host::PinRestoreToken::capture(&app, &label)
                                            .ok_or("贴图已改变")?,
                                    )
                                } else {
                                    None
                                };
                                #[cfg(windows)]
                                {
                                    let handle = window.hwnd().map_err(|_| "无法隐藏截图覆盖层")?;
                                    hide_overlay_window(handle.0 as _)?;
                                }
                                #[cfg(not(windows))]
                                window.hide().map_err(|_| "无法隐藏截图覆盖层")?;
                                hidden.push(HiddenWindow { window, pin });
                            }
                        }
                    }
                    Ok(())
                })();
                #[cfg(windows)]
                if let Some(ticket) = &focus_return {
                    ticket.after_hide();
                }
                if send.send(result).is_err() {
                    let labels = hidden
                        .lock()
                        .map(|mut v| std::mem::take(&mut *v))
                        .unwrap_or_default();
                    restore_ui_windows(&app, &source, labels);
                }
            })
            .map_err(|_| "无法隐藏截图覆盖层")?;
        match receive.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            failed => {
                abandoned.store(true, Ordering::SeqCst);
                self.restore_owned_overlays();
                return Err(match failed {
                    Ok(Err(message)) => message.into(),
                    _ => "隐藏截图覆盖层超时".into(),
                });
            }
        }
        #[cfg(windows)]
        unsafe {
            let _ = windows_sys::Win32::Graphics::Dwm::DwmFlush();
        }
        std::thread::sleep(Duration::from_millis(32));
        Ok(())
    }
    fn restore_owned_overlays(&self) {
        let labels = self
            .hidden
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default();
        let app = self.app.clone();
        let source = self.source.clone();
        let _ = self
            .app
            .run_on_main_thread(move || restore_ui_windows(&app, &source, labels));
    }
}
#[cfg(windows)]
fn hide_overlay_window(handle: windows_sys::Win32::Foundation::HWND) -> Result<(), &'static str> {
    use windows_sys::Win32::{
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            GetWindowThreadProcessId, IsWindow, IsWindowVisible, ShowWindow, SW_HIDE,
        },
    };
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(handle, &mut process);
        if IsWindow(handle) == 0 || process != GetCurrentProcessId() {
            return Err("无法隐藏截图覆盖层");
        }
        // Restoration below uses User32 without activating the window. Mixing
        // that with Tao's cached VISIBLE=false makes its next hide a no-op.
        // Keep both halves on this UI thread's actual HWND state instead.
        ShowWindow(handle, SW_HIDE);
        if IsWindowVisible(handle) != 0 {
            return Err("无法隐藏截图覆盖层");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn show_overlay_window(handle: windows_sys::Win32::Foundation::HWND) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            handle,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
        );
    }
}

#[cfg(all(test, windows))]
mod overlay_visibility_tests {
    use super::{hide_overlay_window, show_overlay_window};
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetWindowLongPtrW, IsWindowVisible, GWL_STYLE,
            WS_CHILD, WS_POPUP, WS_VISIBLE,
        },
    };

    struct OwnedWindow(HWND);
    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn repeated_region_hide_after_nonactivating_restore_uses_real_hwnd_state() {
        // A visible-style child of an always-hidden parent exercises real
        // User32 visibility changes without displaying a desktop test window.
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let parent = OwnedWindow(unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        });
        assert!(!parent.0.is_null());
        let child = OwnedWindow(unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                1,
                1,
                parent.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        });
        assert!(!child.0.is_null());
        let visible_style =
            || unsafe { GetWindowLongPtrW(child.0, GWL_STYLE) as u32 & WS_VISIBLE != 0 };
        assert!(visible_style());
        for _ in 0..3 {
            hide_overlay_window(child.0).unwrap();
            assert!(!visible_style(), "a restored HWND must actually hide again");
            show_overlay_window(child.0);
            assert!(visible_style());
            assert_eq!(unsafe { IsWindowVisible(parent.0) }, 0);
            assert_eq!(unsafe { IsWindowVisible(child.0) }, 0);
        }
        hide_overlay_window(child.0).unwrap();
        assert!(!visible_style());
    }

    #[test]
    fn invalid_overlay_handle_is_rejected_before_any_visibility_change() {
        assert!(hide_overlay_window(std::ptr::null_mut()).is_err());
    }
}

fn restore_ui_windows(
    app: &AppHandle,
    source: &crate::extension_overlay_policy::RestoreSource,
    mut windows: Vec<HiddenWindow>,
) {
    let Some(host) = app.try_state::<Host>() else {
        return;
    };
    // Exit can begin without SpaceTransition. Its existing attempt mutex makes
    // reservation and this synchronous show mutually exclusive. Never retain
    // Engine/plugin locks while Windows dispatches native window messages.
    let status = crate::extension_overlay_policy::restore_groups(
        source,
        &host.capturing,
        &host.exit,
        || host.lock().ok().map(|engine| engine.store.snapshot()),
        |scene_current| {
            for hidden in windows.drain(..) {
                if let Some(pin) = &hidden.pin {
                    if !pin.is_current(app) {
                        continue;
                    }
                } else if !scene_current {
                    continue;
                }
                let window = hidden.window;
                #[cfg(windows)]
                {
                    if let Ok(hwnd) = window.hwnd() {
                        show_overlay_window(hwnd.0 as _);
                    }
                }
                #[cfg(not(windows))]
                let _ = window.show();
            }
        },
    );
    if status == crate::extension_overlay_policy::RestoreStatus::Busy {
        // An explicit capture/close owns visibility now. Keep only independent
        // live pin compensation, never replay stale space/frozen visibility.
        windows.retain(|hidden| hidden.pin.as_ref().is_some_and(|pin| pin.is_current(app)));
        if !windows.is_empty() && host.exit.accepts_new_operations() {
            let app = app.clone();
            let source = source.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let dispatcher = app.clone();
                let _ = dispatcher
                    .run_on_main_thread(move || restore_ui_windows(&app, &source, windows));
            });
        }
    }
}
fn begin_region(
    app: &AppHandle,
    context: &Context,
    diagnostic: &mut Option<RegionDiagnostic>,
) -> Result<Arc<region::RegionSession>, String> {
    if let Some(scope) = &context.chat_region {
        scope.check(app)?;
        return scope.session(|| begin_region_inner(app, context, diagnostic));
    }
    begin_region_inner(app, context, diagnostic)
}
fn begin_region_inner(
    app: &AppHandle,
    context: &Context,
    diagnostic: &mut Option<RegionDiagnostic>,
) -> Result<Arc<region::RegionSession>, String> {
    let scene_id = context.scene_id.as_ref().ok_or("没有当前会话授权")?;
    let region_id = context.region_id.as_ref().ok_or("没有当前选区授权")?;
    let (mut background, selection) = if let Some(scope) = &context.chat_region {
        scope.check(app)?;
        scope.source()
    } else {
        let host = app.state::<Host>();
        let engine = host.lock()?;
        let scene = engine
            .store
            .scenes()
            .iter()
            .find(|s| &s.id == scene_id)
            .ok_or("会话不存在")?;
        let background = scene.background.clone().ok_or("没有截图背景")?;
        let selection = scene
            .regions
            .iter()
            .find(|r| &r.id == region_id)
            .ok_or("选区不存在")?
            .clone();
        (background, selection)
    };
    let host = app.state::<Host>();
    background.path = crate::assets::verified_path(&background, &host.assets)?
        .to_str()
        .ok_or("截图路径无效")?
        .into();
    let mut file = std::fs::File::open(&background.path)
        .map_err(|_| "无法读取截图")?
        .take(128 * 1024 * 1024 + 1);
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| "无法读取截图")?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > 128 * 1024 * 1024 {
            return Err("截图源文件过大".into());
        }
        hash.update(&buffer[..read]);
        context.check(app)?;
    }
    let source = region::RegionSource {
        scene_id: scene_id.clone(),
        background,
        selection,
        asset_sha256: format!("{:x}", hash.finalize()),
    };
    #[cfg(windows)]
    let escape = context
        .chat_region
        .as_ref()
        .map_or_else(
            || context.escape.acquire(context.canceled.clone()),
            |scope| scope.escape(),
        )
        .map_err(|message| {
            record_region_error(
                diagnostic,
                region::RegionError {
                    code: "region_stop_unavailable",
                    message: message.into(),
                },
            )
        })?;
    #[cfg(windows)]
    let focus_return = if let Some(scope) = &context.chat_region {
        scope.focus(app)?
    } else {
        context.focus_return.clone()
    };
    let hooks = Arc::new(OverlayHooks {
        app: app.clone(),
        plugin: context.record.manifest.id.clone(),
        revision: context.record.revision,
        canceled: context.canceled.clone(),
        chat_region: context.chat_region.as_ref().map(Arc::downgrade),
        source: crate::extension_overlay_policy::RestoreSource {
            scene_id: source.scene_id.clone(),
            background_id: source.background.id.clone(),
        },
        #[cfg(windows)]
        _escape: escape,
        #[cfg(windows)]
        focus_return,
        hidden: Arc::new(Mutex::new(vec![])),
    });
    region::RegionSession::begin(source, hooks).map_err(|e| record_region_error(diagnostic, e))
}

#[cfg(test)]
mod extension_tool_registry_tests {
    use super::*;

    #[test]
    fn only_the_host_memory_service_attests_automatic_retention() {
        use mewu_extension::ServiceKind;
        let binding=("memory".into(),7);
        let forged=json!({"operation":"retain","memoryOrigin":"completedTurn"});
        assert_eq!(memory_event_origin(ServiceKind::Memory,Some(&binding),&forged),Some("completedTurn"));
        for kind in [ServiceKind::Tool,ServiceKind::Ui,ServiceKind::Agent] {
            assert_eq!(memory_event_origin(kind,Some(&binding),&forged),None);
        }
        assert_eq!(memory_event_origin(ServiceKind::Memory,None,&forged),None);
        assert_eq!(memory_event_origin(ServiceKind::Memory,Some(&("agent".into(),7)),&forged),None);
        assert_eq!(memory_event_origin(ServiceKind::Memory,Some(&binding),&json!({"operation":"recall"})),None);
    }
    fn external_model_run(
        connection: bool,
    ) -> (
        mewu_core::Store,
        String,
        String,
        mewu_core::ConnectionProfile,
    ) {
        let mut store = mewu_core::Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        let profile = mewu_core::ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "own MiniMax test".into(),
            provider_id: "minimax".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "MiniMax-M3.1-Flash-Preview".into(),
            has_key: false,
            credential_id: None,
            revision: 1,
            advanced: Default::default(),
        };
        store
            .save_connection_profile(profile.clone(), None)
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::SetSceneConnection {
                scene_id: scene.clone(),
                connection_id: connection.then(|| profile.id.clone()),
            })
            .unwrap();
        store
            .apply(mewu_core::SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "own independent task".into(),
            })
            .unwrap();
        let context = store
            .begin_external_run(
                &scene,
                mewu_core::ExternalRunOrigin {
                    plugin_id: "community.own-agent".into(),
                    service_id: "agent".into(),
                    revision: 1,
                    sha256: "ab".repeat(32),
                },
            )
            .unwrap();
        (store, scene, context.run_id, profile)
    }
    #[test]
    fn external_agent_rejects_an_actual_selected_profile_revision_change_with_shared_cancellation()
    {
        let (mut store, scene, run_id, profile) = external_model_run(true);
        let canceled = AtomicBool::new(false);
        assert!(guard_model_connection(&store, &scene, Some(&profile), &canceled).is_ok());
        let mut revised = profile.clone();
        revised.revision += 1;
        revised.model = "another-model".into();
        store
            .save_connection_profile(revised, Some(profile.revision))
            .unwrap();
        // External provenance deliberately remains independent from host-model
        // provenance, so core's connection-linked cancellation does not apply.
        assert!(store.run_is_active(&scene, &run_id));
        assert!(store
            .scene(&scene)
            .unwrap()
            .run
            .as_ref()
            .unwrap()
            .connection_id
            .is_none());
        assert!(guard_model_connection(&store, &scene, Some(&profile), &canceled).is_err());
        assert!(canceled.load(Ordering::SeqCst));
        // The external Agent finalizer already settles a revoked shared lease
        // through this normal Store API, never by forging a model-backed run.
        store.cancel_run(&scene).unwrap();
        assert!(!store.run_is_active(&scene, &run_id));
    }
    #[test]
    fn keyless_external_agent_keeps_running_without_inventing_a_host_model_grant() {
        let (mut store, scene, run_id, profile) = external_model_run(false);
        let canceled = AtomicBool::new(false);
        assert!(guard_model_connection(&store, &scene, None, &canceled).is_ok());
        assert!(matches!(
            store.connection_for_scene(&scene),
            Err(mewu_core::CoreError::ConnectionUnselected)
        ));
        let mut revised = profile.clone();
        revised.revision += 1;
        revised.model = "unrelated-host-model".into();
        store
            .save_connection_profile(revised, Some(profile.revision))
            .unwrap();
        assert!(guard_model_connection(&store, &scene, None, &canceled).is_ok());
        assert!(!canceled.load(Ordering::SeqCst));
        assert!(store.run_is_active(&scene, &run_id));
        store
            .finish_run(&scene, &run_id, "own provider answer")
            .unwrap();
    }
    #[test]
    fn plugin_model_requests_use_the_selected_connections_actual_generation_parameters() {
        let mut connection = mewu_core::ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            provider_id: "openai-compatible".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "configured-model".into(),
            has_key: false,
            credential_id: None,
            revision: 1,
            advanced: Default::default(),
        };
        connection.advanced.request_parameters = mewu_core::ConnectionParameters {
            temperature: Some(0.42),
            top_p: Some(0.85),
            service_tier: Some(mewu_core::ConnectionServiceTier::Priority),
        };
        let messages = vec![json!({"role":"user","content":[{"type":"text","text":"own test"}]})];
        let canonical = model_canonical_body(&connection, messages.clone()).unwrap();
        let body = crate::provider_transport::request_body(connection.advanced.protocol, canonical)
            .unwrap();
        assert_eq!(body["temperature"], json!(0.42));
        assert_eq!(body["top_p"], json!(0.85));
        assert_eq!(body["service_tier"], json!("priority"));
        assert_eq!(body["model"], json!(connection.model));
        assert_eq!(body["messages"], json!(messages));
        assert_eq!(body["stream"], json!(true));
        assert!(body.get("reasoning").is_none());
        assert!(body.get("reasoning_effort").is_none());
        connection.advanced.request_parameters = Default::default();
        let default = model_canonical_body(&connection, vec![]).unwrap();
        for parameter in ["temperature", "top_p", "service_tier"] {
            assert!(default.get(parameter).is_none());
        }
    }
    #[test]
    fn tool_activation_cannot_perform_side_effects_before_the_dispatch_receipt() {
        for method in ["storage.get", "storage.list", "services.list"] {
            assert!(tool_initialization_read(method));
        }
        for method in [
            "storage.set",
            "storage.delete",
            "storage.compareAndSet",
            "region.observe",
            "region.action",
            "model.invoke",
            "ui.render",
            "network.fetch",
            "services.call",
            "events.emit",
            "unknown",
        ] {
            assert!(!tool_initialization_read(method));
        }
    }
    #[test]
    fn a_frozen_tool_requires_the_actual_installed_revision_digest_schema_and_enabled_state() {
        let root = std::env::temp_dir().join(format!("mewu-chat-tool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut store = Store::open(&root).unwrap();
        let manifest = json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript",
            "id":"community.chat-tool","version":"1.0.0","name":"Chat tool","description":"",
            "license":"MPL-2.0","entry":"index.mjs"});
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("mewu-plugin.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.start_file("index.mjs", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"export default ctx=>ctx.tools.register({id:'read',title:'Read'},()=>42);")
            .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let package = crate::extension_market::decode_package(&bytes).unwrap();
        let installed = store
            .install(
                &package,
                &bytes,
                &Source::Local {
                    path: "own-test-package.zip".into(),
                },
                None,
            )
            .unwrap();
        let service: ServiceRegistration =
            serde_json::from_value(json!({"kind":"tool","id":"read","title":"Read",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false}}))
            .unwrap();
        store
            .set_services(
                &installed.manifest.id,
                installed.revision,
                &[service.clone()],
            )
            .unwrap();
        let descriptor = crate::extension_tools::ToolDescriptor {
            plugin_id: installed.manifest.id.clone(),
            revision: installed.revision,
            version: installed.manifest.version.clone(),
            sha256: installed.sha256.clone(),
            service,
        };
        assert!(validate_tool_record(&store, &descriptor).is_ok());
        let mut provider=installed.clone();provider.services=vec![ServiceRegistration{kind:mewu_extension::ServiceKind::Memory,id:"memory".into(),title:"Independent memory".into(),description:None,input_schema:None,metadata:None}];
        let binding=Binding{agent_id:"own-agent".into(),kind:"memory".into(),plugin_id:Some(provider.manifest.id.clone()),service_id:Some("memory".into()),revision:1};
        assert!(provider_binding_matches(&provider,&binding,"memory"));
        assert!(!provider_binding_matches(&provider,&binding,"agent"));
        let mut changed=binding.clone();changed.plugin_id=Some("another.provider".into());
        assert!(!provider_binding_matches(&provider,&changed,"memory"));
        changed=binding.clone();changed.service_id=Some("removed-memory".into());
        assert!(!provider_binding_matches(&provider,&changed,"memory"));
        for mutation in 0..5 {
            let mut changed = descriptor.clone();
            match mutation {
                0 => changed.revision += 1,
                1 => changed.sha256 = "0".repeat(64),
                2 => changed.version = "2.0.0".into(),
                3 => {
                    changed.service.input_schema = Some(json!({"type":"object","required":["new"]}))
                }
                _ => changed.service.kind = mewu_extension::ServiceKind::Command,
            }
            assert!(validate_tool_record(&store, &changed).is_err());
        }
        store
            .set_state(&installed.manifest.id, installed.revision, "disabled")
            .unwrap();
        assert!(validate_tool_record(&store, &descriptor).is_err());
        drop(store);
        for name in [
            "extensions.db",
            "extensions.db-wal",
            "extensions.db-shm",
            "extensions.db-journal",
        ] {
            let _ = std::fs::remove_file(root.join(name));
        }
        std::fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod failure_diagnostic_tests {
    use super::*;

    #[test]
    fn successful_host_progress_is_static_and_survives_a_handler_failure_as_unknown() {
        let mut progress = HostCallProgress::default();
        progress.succeeded("region.observe", &json!({"artifactId":"private-frame"}));
        progress.succeeded(
            "model.invoke",
            &json!({"content":"private model text", "toolCalls":[]}),
        );
        let diagnostic = SafeToolDiagnostic::from_worker(
            &ProtocolError {
                code: "JS_SYNTAX_ERROR".into(),
                message: "Bearer private-key C:/private/body.json raw model response".into(),
            },
            None,
            progress,
        );
        let outcome = crate::extension_tools::ToolInvocationOutcome::Unknown {
            reason: mewu_core::NoResponseReason::IncompleteResponse,
            safe_diagnostic: Some(diagnostic),
        };
        let public = outcome.diagnostic().unwrap();
        assert!(public.contains("JS_SYNTAX_ERROR"));
        assert!(public.contains("已完成宿主调用：2"));
        assert!(public.contains("最近成功：model.invoke"));
        assert!(public.contains("模型正文：text；工具调用数：0"));
        assert!(!public.contains("调用失败"));
        for private in [
            "Bearer",
            "private-key",
            "C:/private",
            "body.json",
            "raw model",
            "private model text",
        ] {
            assert!(!public.contains(private));
        }
        progress.succeeded("Bearer private-key C:/private/body.json", &Value::Null);
        let public = worker_failure(
            &ProtocolError {
                code: "JS_private-key".into(),
                message: "private-key".into(),
            },
            None,
            progress,
        );
        assert!(public.contains("RUNTIME_ERROR"));
        assert!(public.contains("已完成宿主调用：3"));
        assert!(public.contains("最近成功：unsupported"));
        assert!(!public.contains("private-key"));
    }

    #[test]
    fn model_shape_diagnostics_distinguish_empty_text_and_tool_only_without_payloads() {
        let mut progress = HostCallProgress::default();
        for (result, shape) in [
            (
                json!({"content":" \n", "toolCalls":[]}),
                "empty；工具调用数：0",
            ),
            (
                json!({"content":"", "toolCalls":[{"name":"private-tool", "arguments":"private-key"}]}),
                "empty；工具调用数：1",
            ),
            (
                json!({"content":"private-body", "toolCalls":[]}),
                "text；工具调用数：0",
            ),
        ] {
            progress.succeeded("model.invoke", &result);
            let public = worker_failure(
                &ProtocolError {
                    code: "JS_SYNTAX_ERROR".into(),
                    message: "private-key".into(),
                },
                None,
                progress,
            );
            assert!(public.contains(shape));
            for private in ["private-key", "private-body", "private-tool", "arguments"] {
                assert!(!public.contains(private));
            }
        }
    }

    #[test]
    fn dispatched_tool_unknown_preserves_the_existing_sanitized_host_failure_without_private_error_text(
    ) {
        let private =
            "HTTP 403 Bearer sk-private-test-token C:\\private\\payload.json arbitrary JS stack";
        let mut region_diagnostic = None;
        let message = record_region_error(
            &mut region_diagnostic,
            region::RegionError {
                code: "region_target_obscured",
                message: "操作区域被其他窗口遮挡".into(),
            },
        );
        for failure in [
            HostCallFailure::from_error("region.observe", &message, region_diagnostic),
            HostCallFailure::from_error("model.invoke", private, None),
        ] {
            let expected = failure.public_message();
            let diagnostic = SafeToolDiagnostic::from_worker(
                &ProtocolError {
                    code: "RUNTIME_ERROR".into(),
                    message: private.into(),
                },
                Some(failure),
                HostCallProgress::default(),
            );
            let outcome = crate::extension_tools::ToolInvocationOutcome::Unknown {
                reason: mewu_core::NoResponseReason::IncompleteResponse,
                safe_diagnostic: Some(diagnostic),
            };
            let public = outcome.diagnostic().unwrap();
            assert!(public.contains("RUNTIME_ERROR"));
            assert!(public.contains(&expected));
            for secret in [
                "HTTP 403",
                "Bearer",
                "sk-private-test-token",
                "C:\\private",
                "payload.json",
                "JS stack",
            ] {
                assert!(!public.contains(secret));
            }
            assert!(public.len() <= 512);
            assert!(matches!(
                outcome,
                crate::extension_tools::ToolInvocationOutcome::Unknown {
                    reason: mewu_core::NoResponseReason::IncompleteResponse,
                    ..
                }
            ));
        }
    }

    #[test]
    fn unavailable_region_stop_key_keeps_its_exact_typed_failure() {
        let mut diagnostic = None;
        let message = record_region_error(
            &mut diagnostic,
            region::RegionError {
                code: "region_stop_unavailable",
                message: "无法注册区域停止键，请先结束其他使用 Esc 的操作".into(),
            },
        );
        let failure = HostCallFailure::from_error("region.observe", &message, diagnostic);
        assert_eq!(failure.code, "region_stop_unavailable");
        let public = worker_failure(
            &ProtocolError {
                code: "RUNTIME_ERROR".into(),
                message: "service handler failed".into(),
            },
            Some(failure),
            HostCallProgress::default(),
        );
        assert!(public.contains("region.observe"));
        assert!(public.contains("region_stop_unavailable"));
        assert!(public.contains("无法注册区域停止键"));
        assert!(!public.contains("service handler failed"));
    }

    #[test]
    fn region_host_failure_keeps_real_typed_guard_code_and_fixed_reason() {
        let mut diagnostic = None;
        let message = record_region_error(
            &mut diagnostic,
            region::RegionError {
                code: "region_target_obscured",
                message: "操作区域被其他窗口遮挡".into(),
            },
        );
        let failure = HostCallFailure::from_error("region.observe", &message, diagnostic);
        assert_eq!(failure.code, "region_target_obscured");
        assert_eq!(failure.reason, "操作区域被其他窗口遮挡");
        let public = worker_failure(
            &ProtocolError {
                code: "RUNTIME_ERROR".into(),
                message: "service handler failed".into(),
            },
            Some(failure),
            HostCallProgress::default(),
        );
        assert!(public.contains("region.observe"));
        assert!(public.contains("region_target_obscured"));
        assert!(public.contains("操作区域被其他窗口遮挡"));
        assert!(!public.contains("service handler failed"));
        assert!(public.len() <= 512);
    }

    #[test]
    fn host_diagnostics_never_forward_provider_bodies_paths_keys_or_unknown_methods() {
        let sensitive = "HTTP 403 {\"token\":\"sk-private-test-token\"} C:\\Users\\Private\\secret.json\nBearer private-test-token";
        for (method, code) in [
            ("model.invoke", "MODEL_REQUEST_FAILED"),
            ("network.fetch", "NETWORK_REQUEST_FAILED"),
            ("storage.set", "STORAGE_OPERATION_FAILED"),
        ] {
            let failure = HostCallFailure::from_error(method, sensitive, None);
            assert_eq!(failure.code, code);
            let protocol = failure.protocol_error();
            assert!(protocol.message.contains(method));
            let public = worker_failure(
                &ProtocolError {
                    code: sensitive.into(),
                    message: sensitive.into(),
                },
                Some(failure),
                HostCallProgress::default(),
            );
            for output in [protocol.message, public] {
                for secret in [
                    "sk-private-test-token",
                    "C:\\Users",
                    "Bearer",
                    "HTTP 403",
                    "secret.json",
                ] {
                    assert!(!output.contains(secret), "unsafe diagnostic for {method}");
                }
                assert!(output.len() <= 512);
            }
        }
        let failure = HostCallFailure::from_error(sensitive, sensitive, None);
        assert_eq!(failure.method, "unsupported");
        assert_eq!(failure.code, "HOST_UNSUPPORTED");
        assert!(!failure.public_message().contains(sensitive));
    }

    #[test]
    fn region_codes_survive_without_trusting_underlying_error_text_or_prefixes() {
        let sensitive = "操作区域被其他窗口遮挡\nBearer private-test-token";
        let mut diagnostic = None;
        let message = record_region_error(
            &mut diagnostic,
            region::RegionError {
                code: "region_capture_failed",
                message: sensitive.into(),
            },
        );
        let failure = HostCallFailure::from_error("region.observe", &message, diagnostic);
        assert_eq!(failure.code, "region_capture_failed");
        assert_eq!(failure.reason, "区域操作失败");
        assert!(!failure.public_message().contains("private-test-token"));
        let mut diagnostic = None;
        let message = record_region_error(
            &mut diagnostic,
            region::RegionError {
                code: "unknown_internal_code_with_private_test_token",
                message: sensitive.into(),
            },
        );
        let failure = HostCallFailure::from_error("region.action", &message, diagnostic);
        assert_eq!(failure.code, "region_operation_failed");
        let public = worker_failure(
            &ProtocolError {
                code: "RUNTIME_ERROR".into(),
                message: "arbitrary JS stack with private-test-token".into(),
            },
            Some(failure),
            HostCallProgress::default(),
        );
        assert!(!public.contains("private-test-token"));
        assert!(!public.contains("unknown_internal_code"));
    }

    #[test]
    fn mac_region_failures_preserve_actionable_codes_without_disclosing_raw_errors() {
        for (code, reason) in [
            ("region_accessibility_permission", "请在系统设置中允许 MewuAI 使用辅助功能"),
            ("region_focus_outside", "输入焦点不在授权区域内"),
            ("region_secure_input", "不能向密码框输入"),
            ("region_input_busy", "请松开修饰键或停止键后重试"),
            ("region_input_busy", "请松开鼠标后重试"),
        ] {
            for method in ["region.observe", "region.action"] {
                let mut diagnostic = None;
                let message = record_region_error(&mut diagnostic, region::RegionError {
                    code, message: reason.into(),
                });
                let failure = HostCallFailure::from_error(method, &message, diagnostic);
                assert_eq!(failure.code, code);
                assert_eq!(failure.reason, reason);

                let mut diagnostic = None;
                let message = record_region_error(&mut diagnostic, region::RegionError {
                    code, message: format!("{reason}\nprivate-test-token"),
                });
                let failure = HostCallFailure::from_error(method, &message, diagnostic);
                assert_eq!(failure.code, code);
                assert_eq!(failure.reason, "区域操作失败");
                assert!(!failure.public_message().contains("private-test-token"));
            }
        }
    }
}
