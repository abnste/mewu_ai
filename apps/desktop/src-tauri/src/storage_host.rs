// SPDX-License-Identifier: MPL-2.0
//! A directory selection is a native, window-bound proposal. The renderer never
//! supplies a filesystem path to the migration command. Copying is cold-start work.
use crate::{storage_root, Host};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewWindow};

const PROPOSAL_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirectoryState {
    path: String,
    generation: u64,
    phase: &'static str,
    can_change: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirectoryProposal {
    proposal_id: String,
    path: String,
    generation: u64,
}
struct Proposal {
    public: DataDirectoryProposal,
    owner: usize,
    until: Instant,
    target: PathBuf,
    pin: File,
}
#[derive(Default)]
struct Inner {
    proposal: Mutex<Option<Proposal>>,
    choosing: AtomicBool,
    migrating: AtomicBool,
    ready: AtomicBool,
    epoch: AtomicU64,
}
pub struct StorageRuntime {
    paths: storage_root::StoragePaths,
    root: PathBuf,
    generation: u64,
    inner: Arc<Inner>,
    _resolved: storage_root::ResolvedStorage,
}
impl StorageRuntime {
    pub fn new(
        paths: storage_root::StoragePaths,
        resolved: &storage_root::ResolvedStorage,
    ) -> Self {
        Self {
            paths,
            root: resolved.root.clone(),
            generation: resolved.generation,
            inner: Arc::new(Inner::default()),
            _resolved: resolved.clone(),
        }
    }
    pub fn mark_ready(&self) {
        self.inner.ready.store(true, Ordering::Release);
    }
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::Acquire)
    }
    pub fn allows_command(&self, command: &str) -> bool {
        self.is_ready()
            || matches!(
                command,
                "get_snapshot"
                    | "get_plugins"
                    | "runtime_info"
                    | "get_recording_status"
                    | "get_recording_audio"
                    | "get_capture_shortcut"
                    | "get_capture_preferences"
                    | "get_system_preferences"
            )
    }
    fn invalidate(&self) {
        self.inner.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut proposal) = self.inner.proposal.lock() {
            *proposal = None;
        }
    }
    pub fn picker_idle(&self) -> bool {
        !self.inner.choosing.load(Ordering::Acquire)
    }
    fn view(&self, can_change: bool) -> DataDirectoryState {
        DataDirectoryState {
            path: self.root.to_string_lossy().into_owned(),
            generation: self.generation,
            phase: if self.inner.migrating.load(Ordering::Acquire) {
                "requested"
            } else {
                "committed"
            },
            can_change: can_change
                && self.is_ready()
                && self.picker_idle()
                && !self.inner.migrating.load(Ordering::Acquire),
        }
    }
}
struct PickerPermit(Arc<Inner>);
impl PickerPermit {
    fn begin(inner: &Arc<Inner>) -> Result<Self, String> {
        let _slot = inner.proposal.lock().map_err(|_| "目录选择状态不可用")?;
        if inner.migrating.load(Ordering::Acquire) {
            return Err("数据目录正在迁移".into());
        }
        inner
            .choosing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "请先完成目录选择")?;
        Ok(Self(Arc::clone(inner)))
    }
}
impl Drop for PickerPermit {
    fn drop(&mut self) {
        self.0.choosing.store(false, Ordering::Release);
    }
}

fn owner(window: &WebviewWindow) -> Result<usize, String> {
    if window.label() != "settings" {
        return Err("此窗口不能更改数据目录".into());
    }
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|handle| handle.0 as usize)
            .map_err(|_| "设置窗口已关闭".into())
    }
    #[cfg(not(windows))]
    {
        if window.is_visible().unwrap_or(false) {
            Ok(1)
        } else {
            Err("设置窗口已关闭".into())
        }
    }
}
fn current_owner(app: &AppHandle, window: &WebviewWindow, expected: usize) -> Result<(), String> {
    let current = app.get_webview_window("settings").ok_or("设置窗口已关闭")?;
    if owner(&current)? != expected
        || owner(window)? != expected
        || !window.is_visible().unwrap_or(false)
    {
        return Err("设置窗口已关闭".into());
    }
    Ok(())
}
fn running_idle(app: &AppHandle) -> Result<(), String> {
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    if !host.storage.is_ready() {
        return Err("正在启动，请稍候".into());
    }
    crate::recording::ensure_idle(app)
}

pub fn migration_preflight(app: &AppHandle) -> Result<(), String> {
    let host = app.state::<Host>();
    let (servers, sources, catalog) = {
        // Established plugin -> Engine order; only owned SQLite metadata and
        // small typed path records are copied here, never package-file reads.
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        let engine = host.lock()?;
        (
            engine.store.mcp_servers().to_vec(),
            plugins.store.migration_sources()?,
            plugins.store.get_catalog_source()?,
        )
    };
    crate::storage_dependencies::validate_dependencies(
        &host.root,
        &servers,
        &sources,
        catalog.as_deref(),
    )?;
    let environment: Vec<String> = [
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "LANG",
        "LC_ALL",
    ]
    .into_iter()
    .filter_map(std::env::var_os)
    .map(|value| {
        value
            .into_string()
            .map_err(|_| "无法准确检查 MCP 环境路径".to_string())
    })
    .collect::<Result<_, _>>()?;
    crate::storage_dependencies::validate_environment(
        &host.root,
        environment.iter().map(String::as_str),
    )
}
fn ordinary_directory(path: &Path) -> Result<File, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "无法读取所选目录")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("数据目录不能是链接".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("数据目录不能是重解析点".into());
        }
        OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|_| "无法锁定所选目录".into())
    }
    #[cfg(not(windows))]
    {
        File::open(path).map_err(|_| "无法锁定所选目录".into())
    }
}
fn same_directory(pin: &File, path: &Path) -> Result<(), String> {
    let expected = same_file::Handle::from_file(pin.try_clone().map_err(|_| "目录选择已失效")?)
        .map_err(|_| "目录选择已失效")?;
    let actual =
        same_file::Handle::from_file(ordinary_directory(path)?).map_err(|_| "目录选择已失效")?;
    if expected != actual {
        return Err("所选目录已变化，请重新选择".into());
    }
    Ok(())
}

/// Owned by the actual shutdown task, not by an abandoned invoke future.
pub struct MigrationTask {
    paths: storage_root::StoragePaths,
    source: PathBuf,
    generation: u64,
    target: PathBuf,
    pin: File,
    inner: Arc<Inner>,
}
impl MigrationTask {
    pub fn prepare(&self) -> Result<String, String> {
        same_directory(&self.pin, &self.target)?;
        storage_root::request_migration(&self.paths, &self.source, self.generation, &self.target)
            .map_err(|error| error.to_string())
    }
    pub fn cancel_requested(&self, id: &str) -> Result<(), String> {
        storage_root::cancel_requested(&self.paths, id)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
impl Drop for MigrationTask {
    fn drop(&mut self) {
        self.inner.migrating.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub fn get_data_directory_state(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<DataDirectoryState, String> {
    owner(&window)?;
    let host = app.state::<Host>();
    let can_change = running_idle(&app).is_ok() && !host.capturing.load(Ordering::Acquire);
    Ok(host.storage.view(can_change))
}
#[tauri::command]
pub async fn choose_data_directory(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<DataDirectoryProposal>, String> {
    let owner_id = owner(&window)?;
    current_owner(&app, &window, owner_id)?;
    running_idle(&app)?;
    let host = app.state::<Host>();
    let permit = PickerPermit::begin(&host.storage.inner)?;
    let transition =
        crate::SpaceTransition::try_begin(&host.capturing).ok_or("请先完成当前采集或空间切换")?;
    running_idle(&app)?;
    let epoch = host.storage.inner.epoch.load(Ordering::Acquire);
    let inner = Arc::clone(&host.storage.inner);
    let generation = host.storage.generation;
    let initial = host.storage.root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_permit, _transition) = (permit, transition);
        running_idle(&app)?;
        current_owner(&app, &window, owner_id)?;
        let selected = rfd::FileDialog::new()
            .set_parent(&window)
            .set_directory(&initial)
            .pick_folder();
        let Some(target) = selected else {
            return Ok(None);
        };
        running_idle(&app)?;
        current_owner(&app, &window, owner_id)?;
        if inner.epoch.load(Ordering::Acquire) != epoch {
            return Err("目录选择已结束".into());
        }
        let pin = ordinary_directory(&target)?;
        let target = target.canonicalize().map_err(|_| "无法确定所选目录")?;
        same_directory(&pin, &target)?;
        let public = DataDirectoryProposal {
            proposal_id: uuid::Uuid::new_v4().to_string(),
            path: target.to_string_lossy().into_owned(),
            generation,
        };
        current_owner(&app, &window, owner_id)?;
        running_idle(&app)?;
        let mut slot = inner.proposal.lock().map_err(|_| "目录选择状态不可用")?;
        if inner.epoch.load(Ordering::Acquire) != epoch {
            return Err("目录选择已结束".into());
        }
        *slot = Some(Proposal {
            public: public.clone(),
            owner: owner_id,
            until: Instant::now() + PROPOSAL_TTL,
            target,
            pin,
        });
        Ok(Some(public))
    })
    .await
    .map_err(|_| "目录选择中断".to_string())?
}
#[tauri::command]
pub fn cancel_data_directory_proposal(
    app: AppHandle,
    window: WebviewWindow,
    proposal_id: String,
) -> Result<(), String> {
    let owner_id = owner(&window)?;
    let host = app.state::<Host>();
    let mut slot = host
        .storage
        .inner
        .proposal
        .lock()
        .map_err(|_| "目录选择状态不可用")?;
    if let Some(current) = slot
        .as_ref()
        .filter(|value| value.public.proposal_id == proposal_id)
    {
        if current.owner != owner_id {
            return Err("目录选择不属于此窗口".into());
        }
        *slot = None;
    }
    Ok(())
}
#[tauri::command]
pub async fn migrate_data_directory(
    app: AppHandle,
    window: WebviewWindow,
    proposal_id: String,
    expected_generation: u64,
) -> Result<(), String> {
    let owner_id = owner(&window)?;
    current_owner(&app, &window, owner_id)?;
    running_idle(&app)?;
    migration_preflight(&app)?;
    let host = app.state::<Host>();
    if host.capturing.load(Ordering::Acquire) || !host.storage.picker_idle() {
        return Err("请先完成当前采集或目录选择".into());
    }
    let migration = {
        let slot = host
            .storage
            .inner
            .proposal
            .lock()
            .map_err(|_| "目录选择状态不可用")?;
        let selected = slot
            .as_ref()
            .filter(|p| {
                p.public.proposal_id == proposal_id
                    && p.owner == owner_id
                    && p.public.generation == expected_generation
                    && p.until > Instant::now()
            })
            .ok_or("目录选择已失效，请重新选择")?;
        if expected_generation != host.storage.generation {
            return Err("数据目录已变化，请重新选择".into());
        }
        if host.storage.inner.choosing.load(Ordering::Acquire) {
            return Err("请先完成目录选择".into());
        }
        let pin = selected.pin.try_clone().map_err(|_| "目录选择已失效")?;
        host.storage
            .inner
            .migrating
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "数据目录正在迁移")?;
        MigrationTask {
            paths: host.storage.paths.clone(),
            source: host.storage.root.clone(),
            generation: expected_generation,
            target: selected.target.clone(),
            pin,
            inner: Arc::clone(&host.storage.inner),
        }
    };
    // A concurrent tray exit can reserve after proposal validation. Drop this
    // still-unprepared task on rejection; it cannot create a durable intent.
    host.exit.ensure_new_operation()?;
    crate::lifecycle::request_storage_restart(&app, migration).await
}
pub fn invalidate_owner(app: &AppHandle) {
    if let Some(host) = app.try_state::<Host>() {
        host.storage.invalidate();
    }
}
pub async fn drain_picker(app: &AppHandle, timeout: Duration) -> Result<(), String> {
    let until = Instant::now() + timeout;
    while !app.state::<Host>().storage.picker_idle() {
        if Instant::now() >= until {
            return Err("目录选择尚未结束，已取消退出".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
