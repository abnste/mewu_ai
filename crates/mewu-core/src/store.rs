// SPDX-License-Identifier: MPL-2.0
use crate::*;
use rusqlite::{params, Connection as SqlConnection, OpenFlags, TransactionBehavior};
use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

const APPLICATION_ID: i64 = 0x4d455755;
pub(crate) const DB_SCHEMA_VERSION: u32 = 9;

#[path = "agent_cleanup_store.rs"]
mod agent_cleanup_store;
#[path = "journal_store.rs"]
mod journal_store;
#[path = "memory_ledger_store.rs"]
mod memory_ledger_store;
#[path = "video_annotations_store.rs"]
mod video_annotations_store;
#[path = "visual_annotations_store.rs"]
mod visual_annotations_store;
#[path = "raster_edit_store.rs"]
mod raster_edit_store;
#[path = "blackboard_store.rs"]
mod blackboard_store;
#[path = "history_storage.rs"]
pub(crate) mod history_storage;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("存储错误：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("场景数据格式错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("不支持的数据版本：{0}；原数据未被覆盖")]
    UnsupportedSchema(u64),
    #[error("无效场景数据：{0}")]
    Invalid(String),
    #[error("找不到{kind}：{id}")]
    NotFound { kind: &'static str, id: String },
    #[error("此场景已有正在进行的运行")]
    RunInProgress,
    #[error("运行已取消、结束或被替换，拒绝迟到结果")]
    StaleRun,
    #[error("已有消息的场景需要新建场景后才能切换 Agent")]
    AgentContextConflict,
    #[error("请输入消息或选择引用内容")]
    EmptyInput,
    #[error("数据已被另一个实例修改，请重新打开以避免覆盖")]
    ConcurrentModification,
    #[error("记忆已被修改，请重新读取后再保存")]
    MemoryConflict,
    #[error("当前 Agent 未启用长期记忆")]
    MemoryDisabled,
    #[error("记忆来源或授权已改变，请重新读取")]
    MemoryProviderConflict,
    #[error("记忆后台任务已改变，拒绝迟到操作")]
    MemoryWorkConflict,
    #[error("记忆检索超时，请缩小搜索范围")]
    MemoryQueryTimedOut,
    #[error("工具步骤已结束或不存在，拒绝迟到结果")]
    StaleTool,
    #[error("工具配置已被修改，请重新读取后再保存")]
    McpConflict,
    #[error("当前 Agent 未获此工具授权，或工具已停用")]
    McpPermissionDenied,
    #[error("选区或绘制已更改，请重新操作")]
    DrawingConflict,
    #[error("选区已更改，请重新操作")]
    GeometryConflict,
    #[error("几何历史已更改，请重新操作")]
    GeometryHistoryConflict,
    #[error("视频范围或来源已更改，请重新操作")]
    VideoEditConflict,
    #[error("视频裁剪历史已更改，请重新操作")]
    VideoHistoryConflict,
    #[error("视频批注或来源已更改，请重新操作")]
    VideoAnnotationConflict,
    #[error("识别选区已更改或关闭，请重新识别")]
    OcrConflict,
    #[error("翻译选区已更改或关闭，请重新翻译")]
    TranslationConflict,
    #[error("连接已被修改，请重新读取后再保存")]
    ConnectionConflict,
    #[error("请为此会话选择连接")]
    ConnectionUnselected,
    #[error("执行记录已更新，请重新读取")]
    JournalConflict,
}

type Result<T> = std::result::Result<T, CoreError>;

/// One transactional writer. The host should place this behind its application mutex.
/// A revision fence also prevents a second process from silently overwriting this writer.
pub struct Store {
    db: SqlConnection,
    state: Snapshot,
    revision: i64,
    geometry_group: Option<crate::geometry_history::ActiveEdit>,
}

/// An independent read snapshot for explicit exports. Opening it never migrates
/// data or settles live runs. Keep it only for the duration of one export: WAL
/// writers may proceed, but checkpoint reclamation waits for older readers.
pub struct ExportView {
    db: SqlConnection,
    state: Snapshot,
}

impl ExportView {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = SqlConnection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("BEGIN DEFERRED")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(version as u64));
        }
        let app_id: i64 = db.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if app_id != APPLICATION_ID {
            return Err(invalid("数据库应用标识不匹配"));
        }
        let mode: String = db.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(invalid("导出需要已打开的本机 WAL 存储"));
        }
        let (mut state, _) = read_stored_snapshot(&db, false)?;
        crate::memory::validate(&db, &state)?;
        crate::external_memory::validate(&db, &state)?;
        crate::journal::validate(&db, &state)?;
        crate::visual_annotations::validate(&db, &state)?;
        crate::rich_annotations::validate(&db, &state)?;
        crate::video_registered_layouts::validate(&db, &state)?;
        crate::video_annotations::validate_ledger(&db, &state)?;
        state.memories.clear();
        state.memory_stats = crate::memory::stats(&db, &state)?;
        Ok(Self { db, state })
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.state
    }

    pub fn memory_page(
        &self,
        agent_id: &str,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MemoryPage> {
        if !self.state.agents.iter().any(|agent| agent.id == agent_id) {
            return Err(not_found("Agent", agent_id));
        }
        crate::memory::page_in_transaction(&self.db, agent_id, query, cursor, limit)
            .map_err(crate::memory::query_error)
    }
}

impl Drop for ExportView {
    fn drop(&mut self) {
        let _ = self.db.execute_batch("ROLLBACK");
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        // Validate/migrate before changing journal mode, so an unknown or
        // malformed database is never converted just by attempting to open it.
        let store = Self::from_connection(SqlConnection::open(path)?)?;
        let mode: String = store
            .db
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(invalid("无法启用本机 WAL 存储"));
        }
        store.db.pragma_update(None, "synchronous", "FULL")?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(SqlConnection::open_in_memory()?)
    }

    fn from_connection(mut db: SqlConnection) -> Result<Self> {
        db.busy_timeout(Duration::from_secs(5))?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let app_id: i64 =
            transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let (mut state, mut revision) = match version {
            0 => {
                let count: i64 = transaction.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )?;
                if count != 0 || app_id != 0 {
                    return Err(invalid("未识别的数据库结构，拒绝初始化覆盖"));
                }
                let state = initial_snapshot();
                validate_snapshot(&state)?;
                transaction.execute_batch(
                    "CREATE TABLE app_state (
                        id INTEGER PRIMARY KEY CHECK (id = 1),
                        schema_version INTEGER NOT NULL CHECK (schema_version = 1),
                        revision INTEGER NOT NULL CHECK (revision >= 0),
                        payload TEXT NOT NULL
                    );",
                )?;
                transaction.execute(
                    "INSERT INTO app_state (id, schema_version, revision, payload) VALUES (1, 1, 0, ?1)",
                    [serde_json::to_string(&state)?],
                )?;
                transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
                (state, 0)
            }
            1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | DB_SCHEMA_VERSION => {
                if app_id != APPLICATION_ID {
                    return Err(invalid("数据库应用标识不匹配"));
                }
                read_stored_snapshot(&transaction, version < 3)?
            }
            other => return Err(CoreError::UnsupportedSchema(other as u64)),
        };
        if version < 2 {
            crate::memory::migrate(&transaction, &state)?;
        }
        crate::memory::validate(&transaction, &state)?;
        if version < 4 {
            crate::external_memory::migrate(&transaction)?;
        }
        crate::external_memory::validate(&transaction, &state)?;
        if version < 5 {
            crate::journal::migrate(&transaction)?;
        }
        crate::journal::validate(&transaction, &state)?;
        if version < 6 {
            crate::visual_annotations::migrate(&transaction)?;
        }
        crate::visual_annotations::validate(&transaction, &state)?;
        if version < 7 {
            crate::rich_annotations::migrate(&transaction)?;
        }
        crate::rich_annotations::validate(&transaction, &state)?;
        if version < 8 {
            crate::video_annotation_layouts::migrate(&transaction)?;
            crate::video_annotations::migrate(&transaction)?;
        }
        if version < 9 {
            crate::video_vector_layouts::migrate(&transaction)?;
        }
        crate::video_registered_layouts::validate(&transaction, &state)?;
        crate::video_annotations::validate_ledger(&transaction, &state)?;
        // Old payload fields are an import source only. Modern snapshots must
        // never broadcast or rewrite the entire durable memory collection.
        state.memories.clear();
        state.memory_stats = crate::memory::stats(&transaction, &state)?;
        let recovering = state.scenes.iter().any(is_running);
        if recovering {
            let previous = state.clone();
            for target in &mut state.scenes {
                if is_running(target) {
                    let run = target.run.as_mut().expect("checked running run");
                    run.status = RunStatus::Failed;
                    run.error = Some("上次运行已中断".into());
                    settle_steps(run, "上次运行已中断");
                    archive_steps(target)?;
                    touch(target);
                }
            }
            crate::journal::on_snapshot_change(&transaction, &previous, &mut state)?;
        }
        if version < DB_SCHEMA_VERSION || recovering {
            // Connection migration and its version gate are one transaction:
            // a v2 binary must never overwrite the new routing identities.
            validate_snapshot(&state)?;
            let changed = if matches!(version, 6 | 7 | 8) && !recovering {
                // Rich/video layouts add empty, independent binary tables. Keep
                // the exact existing document payload during this migration,
                // including fields derived only for the in-memory view.
                transaction.execute(
                    "UPDATE app_state SET revision=revision+1 WHERE id=1 AND revision=?1",
                    params![revision],
                )?
            } else {
                transaction.execute(
                    "UPDATE app_state SET payload=?1, revision=revision+1 WHERE id=1 AND revision=?2",
                    params![serde_json::to_string(&state)?, revision],
                )?
            };
            if changed != 1 {
                return Err(CoreError::ConcurrentModification);
            }
            transaction.pragma_update(None, "user_version", DB_SCHEMA_VERSION)?;
            revision += 1;
        }
        transaction.commit()?;
        let store = Self {
            db,
            state,
            revision,
            geometry_group: None,
        };
        Ok(store)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.clone()
    }

    /// Narrow metadata view for host relocation preflight; avoid cloning every
    /// conversation and attachment when only local server paths are needed.
    pub fn mcp_servers(&self) -> &[McpServer] {
        &self.state.mcp_servers
    }
    pub fn rich_layout(&self, reference: &RichDrawingRef) -> Result<RichLayout> {
        crate::rich_annotations::load(&self.db, reference)
    }
    pub fn read_rich_layout(path: &Path, reference: &RichDrawingRef) -> Result<RichLayout> {
        crate::rich_annotations::read_layout(path, reference)
    }

    /// Check a streaming callback's run fence without cloning scene contents.
    pub fn run_is_active(&self, scene_id: &str, run_id: &str) -> bool {
        self.state.scenes.iter().any(|scene| {
            scene.id == scene_id
                && !scene.closed
                && scene
                    .run
                    .as_ref()
                    .is_some_and(|run| run.id == run_id && run.status == RunStatus::Running)
        })
    }

    /// The native minimize window failed after its storage transaction. Restore
    /// only selection and its prior minimize choice, retaining late run results.
    pub fn compensate_failed_minimize(
        &mut self,
        scene_id: &str,
        previously_minimized: bool,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        if target.closed {
            return Err(invalid("会话已关闭"));
        }
        target.minimized = previously_minimized;
        for candidate in &mut next.scenes {
            candidate.frozen = candidate.id != scene_id;
        }
        next.active_scene_id = scene_id.into();
        let snapshot = self.persist(next)?;
        self.geometry_group = None;
        Ok(snapshot)
    }

    pub fn apply(&mut self, command: SceneCommand) -> Result<Snapshot> {
        if let SceneCommand::FinishRegionGeometryEdit { scene_id, edit_id } = &command {
            crate::geometry_history::validate_edit_id(edit_id)?;
            if self
                .geometry_group
                .as_ref()
                .is_some_and(|group| group.matches(scene_id, edit_id))
            {
                self.geometry_group = None;
            }
            return Ok(self.snapshot());
        }
        let redo_geometry = matches!(&command, SceneCommand::RedoRegionGeometry { .. });
        let seal_geometry_group = matches!(
            &command,
            SceneCommand::NewScene
                | SceneCommand::NewConversation { .. }
                | SceneCommand::FreezeScene { .. }
                | SceneCommand::ActivateScene { .. }
                | SceneCommand::CloseScene { .. }
        );
        let mut next = self.state.clone();
        match command {
            SceneCommand::NewScene => create_scene(&mut next)?,
            SceneCommand::NewConversation { scene_id } => {
                if next.active_scene_id != scene_id {
                    return Err(invalid("仅活动会话可以开始新会话"));
                }
                let target = scene_mut(&mut next, &scene_id)?;
                if target.frozen {
                    return Err(invalid("冻结会话不能开始新会话"));
                }
                if target.closed {
                    return Err(invalid("会话已关闭"));
                }
                if is_running(target) {
                    return Err(CoreError::RunInProgress);
                }
                if target.conversation_start.unwrap_or(0) == target.messages.len()
                    && target.draft.is_empty()
                    && target.run.is_none()
                {
                    return Ok(self.snapshot());
                }
                target.conversation_start = Some(target.messages.len());
                target.draft.clear();
                target.run = None;
                touch(target);
            }
            SceneCommand::SetSceneConnection {
                scene_id,
                connection_id,
            } => {
                ensure_connection_exists(&next, connection_id.as_deref())?;
                let target = scene_mut(&mut next, &scene_id)?;
                if target.connection_id == connection_id {
                    return Ok(self.snapshot());
                }
                if is_running(target) {
                    return Err(CoreError::RunInProgress);
                }
                target.connection_id = connection_id;
                touch(target);
            }
            SceneCommand::SetDefaultConnection { connection_id } => {
                ensure_connection_exists(&next, connection_id.as_deref())?;
                if next.default_connection_id == connection_id {
                    return Ok(self.snapshot());
                }
                next.default_connection_id = connection_id;
                crate::connections::sync_compatibility(&mut next);
            }
            SceneCommand::FreezeScene { scene_id } => {
                if scene(&next, &scene_id)?.closed {
                    return Err(invalid("会话已关闭，请先恢复会话"));
                }
                if next.active_scene_id != scene_id {
                    return Err(invalid("只能冻结当前活动场景"));
                }
                scene_mut(&mut next, &scene_id)?.minimized = true;
                create_scene(&mut next)?;
            }
            SceneCommand::ActivateScene { scene_id } => {
                scene_mut(&mut next, &scene_id)?.closed = false;
                for candidate in &mut next.scenes {
                    candidate.frozen = candidate.id != scene_id;
                }
                next.active_scene_id = scene_id;
            }
            SceneCommand::CloseScene { scene_id } => {
                let target = scene_mut(&mut next, &scene_id)?;
                if target.closed {
                    return Ok(self.snapshot());
                }
                if is_running(target) {
                    let run = target.run.as_mut().expect("checked running run");
                    run.status = RunStatus::Canceled;
                    run.error = Some("会话已关闭".into());
                    settle_steps(run, "会话已关闭");
                    archive_steps(target)?;
                }
                target.closed = true;
                target.frozen = true;
                touch(target);
                if next.active_scene_id == scene_id {
                    create_scene(&mut next)?;
                }
            }
            SceneCommand::SetDraft { scene_id, draft } => {
                let target = scene_mut(&mut next, &scene_id)?;
                target.draft = draft;
                touch(target);
            }
            SceneCommand::RenameScene { scene_id, title } => {
                if title.trim().is_empty() {
                    return Err(invalid("场景名称不能为空"));
                }
                let target = scene_mut(&mut next, &scene_id)?;
                target.title = title.trim().into();
                touch(target);
            }
            SceneCommand::SetRefs { scene_id, refs } => {
                let target = scene_mut(&mut next, &scene_id)?;
                target.refs = refs;
                touch(target);
            }
            SceneCommand::AddRegion { scene_id, region } => {
                let target = scene_mut(&mut next, &scene_id)?;
                if region.image_override.is_some() {
                    return Err(invalid("新选区不能携带替换图片"));
                }
                if region.ocr.is_some() {
                    return Err(invalid("新选区不能携带文字识别结果"));
                }
                if region.translation.is_some() {
                    return Err(invalid("新选区不能携带翻译图层"));
                }
                if !region.drawings.is_empty()
                    || !region.drawing_history.is_empty()
                    || region.drawing_revision != 0
                {
                    return Err(invalid("新选区不能携带绘制历史，请使用绘制命令"));
                }
                if target.regions.iter().any(|r| r.id == region.id) {
                    return Err(invalid("区域 ID 重复"));
                }
                target.regions.push(region);
                touch(target);
            }
            SceneCommand::RemoveRegion {
                scene_id,
                region_id,
            } => {
                let target = scene_mut(&mut next, &scene_id)?;
                if !target.regions.iter().any(|r| r.id == region_id) {
                    return Err(not_found("区域", &region_id));
                }
                target.regions.retain(|r| r.id != region_id);
                target
                    .refs
                    .retain(|r| r.kind != ReferenceKind::Region || r.id != region_id);
                touch(target);
            }
            SceneCommand::UpdateRegion { scene_id, region } => {
                let target = scene_mut(&mut next, &scene_id)?;
                let existing = target
                    .regions
                    .iter_mut()
                    .find(|r| r.id == region.id)
                    .ok_or_else(|| not_found("区域", &region.id))?;
                // Geometry updates from a stale view must never overwrite the
                // editable objects/history or host-registered replacement image.
                if (existing.x, existing.y, existing.width, existing.height)
                    != (region.x, region.y, region.width, region.height)
                {
                    existing.drawing_revision = crate::drawing::next_revision(existing)?;
                    existing.ocr = None;
                    existing.translation = None;
                    existing.x = region.x;
                    existing.y = region.y;
                    existing.width = region.width;
                    existing.height = region.height;
                    crate::geometry_history::invalidate_region(target, &region.id)?;
                }
                touch(target);
            }
            SceneCommand::SetRegionGeometry {
                scene_id,
                region_id,
                background_id,
                source_id,
                expected_revision,
                from,
                to,
                edit_id,
            } => {
                let mut group = self.geometry_group.clone();
                let changed = crate::geometry_history::edit(
                    &mut next,
                    crate::geometry_history::Target {
                        scene_id: &scene_id,
                        region_id: &region_id,
                        background_id: &background_id,
                        source_id: &source_id,
                        revision: expected_revision,
                        from,
                    },
                    to,
                    edit_id.as_deref(),
                    &mut group,
                )?;
                if !changed {
                    self.geometry_group = group;
                    return Ok(self.snapshot());
                }
                touch(scene_mut(&mut next, &scene_id)?);
                let snapshot = self.persist(next)?;
                self.geometry_group = group;
                return Ok(snapshot);
            }
            SceneCommand::UndoRegionGeometry {
                scene_id,
                expected_history_revision,
                expected_operation_id,
                region_id,
                background_id,
                source_id,
                expected_revision,
                from,
            }
            | SceneCommand::RedoRegionGeometry {
                scene_id,
                expected_history_revision,
                expected_operation_id,
                region_id,
                background_id,
                source_id,
                expected_revision,
                from,
            } => {
                crate::geometry_history::replay(
                    &mut next,
                    crate::geometry_history::Target {
                        scene_id: &scene_id,
                        region_id: &region_id,
                        background_id: &background_id,
                        source_id: &source_id,
                        revision: expected_revision,
                        from,
                    },
                    expected_history_revision,
                    &expected_operation_id,
                    redo_geometry,
                )?;
                touch(scene_mut(&mut next, &scene_id)?);
                let snapshot = self.persist(next)?;
                self.geometry_group = None;
                return Ok(snapshot);
            }
            SceneCommand::FinishRegionGeometryEdit { .. } => {
                unreachable!("handled without persistence")
            }
            SceneCommand::AddDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
                drawing,
            } => {
                edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::Add(drawing),
                )?;
            }
            SceneCommand::UpdateDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
                drawing,
            } => {
                if !edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::Update(drawing),
                )? {
                    self.ensure_current_revision()?;
                    return Ok(self.snapshot());
                }
            }
            SceneCommand::RemoveDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
                drawing_id,
            } => {
                edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::Remove(drawing_id),
                )?;
            }
            SceneCommand::UpdateDrawings {
                scene_id,
                region_id,
                background_id,
                expected_revision,
                drawings,
            } => {
                if !edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::UpdateBatch(drawings),
                )? {
                    self.ensure_current_revision()?;
                    return Ok(self.snapshot());
                }
            }
            SceneCommand::UndoDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
            } => {
                edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::Undo,
                )?;
            }
            SceneCommand::RedoDrawing {
                scene_id,
                region_id,
                background_id,
                expected_revision,
            } => {
                edit_drawing(
                    &mut next,
                    &scene_id,
                    &region_id,
                    &background_id,
                    expected_revision,
                    crate::drawing::Mutation::Redo,
                )?;
            }
            SceneCommand::UpdateItem { scene_id, item } => {
                let target = scene_mut(&mut next, &scene_id)?;
                let old = target
                    .items
                    .iter_mut()
                    .find(|i| i.id == item.id)
                    .ok_or_else(|| not_found("素材", &item.id))?;
                // The UI may move/resize a card; it cannot retarget a native asset path.
                if old.asset != item.asset {
                    return Err(invalid("更新素材位置不能替换素材来源"));
                }
                if old.state.as_ref().and_then(|s| s.get(crate::blackboard_objects::SOURCE_KEY))
                    != item.state.as_ref().and_then(|s| s.get(crate::blackboard_objects::SOURCE_KEY)) {
                    return Err(invalid("更新素材不能替换黑板来源"));
                }
                // Range metadata and history are host-owned. A stale card move
                // cannot erase a newer trim, nor can generic IPC forge one.
                let video_edit = old.video_edit.clone();
                let video_annotations = old.video_annotations.clone();
                *old = item;
                old.video_edit = video_edit;
                old.video_annotations = video_annotations;
                touch(target);
            }
            SceneCommand::RemoveItem { scene_id, item_id } => {
                let target = scene_mut(&mut next, &scene_id)?;
                if !target.items.iter().any(|i| i.id == item_id) {
                    return Err(not_found("素材", &item_id));
                }
                target.items.retain(|i| i.id != item_id);
                target
                    .refs
                    .retain(|r| r.kind != ReferenceKind::Item || r.id != item_id);
                touch(target);
            }
            SceneCommand::SaveAgent { agent } => {
                if next
                    .agents
                    .iter()
                    .find(|a| a.id == agent.id)
                    .map_or(!agent.memory_provider.is_default(), |a| {
                        a.memory_provider != agent.memory_provider
                    })
                {
                    return Err(CoreError::MemoryProviderConflict);
                }
                let revoke_memory = !agent.memory_enabled
                    && next
                        .agents
                        .iter()
                        .any(|existing| existing.id == agent.id && existing.memory_enabled);
                if revoke_memory {
                    // RunContext already contains an immutable memory snapshot. Cancel
                    // all of this identity's runs to prevent further use after revocation.
                    for target in next
                        .scenes
                        .iter_mut()
                        .filter(|target| target.agent_id == agent.id && is_running(target))
                    {
                        let run = target.run.as_mut().expect("checked running run");
                        run.status = RunStatus::Canceled;
                        run.error = Some("长期记忆已关闭".into());
                        settle_steps(run, "长期记忆已关闭");
                        archive_steps(target)?;
                        touch(target);
                    }
                }
                if let Some(existing) = next.agents.iter_mut().find(|a| a.id == agent.id) {
                    *existing = agent;
                } else {
                    next.agents.push(agent);
                }
            }
            SceneCommand::SetAgent { scene_id, agent_id } => {
                if !next.agents.iter().any(|a| a.id == agent_id) {
                    return Err(not_found("Agent", &agent_id));
                }
                let target = scene_mut(&mut next, &scene_id)?;
                if is_running(target) {
                    return Err(CoreError::RunInProgress);
                }
                if target.agent_id != agent_id && !target.messages.is_empty() {
                    return Err(CoreError::AgentContextConflict);
                }
                target.agent_id = agent_id;
                touch(target);
            }
            SceneCommand::SaveMemory {
                agent_id,
                id,
                text,
                expected_revision,
            } => {
                return self.persist_memory(
                    next,
                    agent_id,
                    MemoryMutation::Save {
                        id,
                        text,
                        expected_revision,
                    },
                    None,
                );
            }
            SceneCommand::DeleteMemory {
                agent_id,
                id,
                expected_revision,
            } => {
                return self.persist_memory(
                    next,
                    agent_id,
                    MemoryMutation::Delete {
                        id,
                        expected_revision,
                    },
                    None,
                );
            }
            SceneCommand::SetMcpGrants {
                server_id,
                expected_revision,
                agent_id,
                mut tool_names,
            } => {
                if !next.agents.iter().any(|agent| agent.id == agent_id) {
                    return Err(not_found("Agent", &agent_id));
                }
                let server = mcp_server(&next, &server_id)?;
                check_mcp_revision(server, expected_revision)?;
                unique(tool_names.iter().map(String::as_str), "授权工具")?;
                if tool_names
                    .iter()
                    .any(|name| !server.tools.iter().any(|tool| tool.name == *name))
                {
                    return Err(invalid("授权包含未发现的工具"));
                }
                tool_names.sort();
                let mut existing = server
                    .grants
                    .iter()
                    .find(|grant| grant.agent_id == agent_id)
                    .map(|grant| grant.tool_names.clone())
                    .unwrap_or_default();
                existing.sort();
                if existing == tool_names {
                    return Ok(self.snapshot());
                }
                let revoke = granted_agents(server);
                let server = next
                    .mcp_servers
                    .iter_mut()
                    .find(|server| server.id == server_id)
                    .expect("checked server");
                server.revision = next_mcp_revision(server.revision)?;
                server.grants.retain(|grant| grant.agent_id != agent_id);
                if !tool_names.is_empty() {
                    server.grants.push(McpGrant {
                        agent_id,
                        tool_names,
                    });
                }
                cancel_mcp_runs(&mut next, &revoke)?;
            }
            SceneCommand::SetMcpEnabled {
                server_id,
                expected_revision,
                enabled,
            } => {
                let server = mcp_server(&next, &server_id)?;
                check_mcp_revision(server, expected_revision)?;
                if server.enabled == enabled {
                    return Ok(self.snapshot());
                }
                let revoke = granted_agents(server);
                let server = next
                    .mcp_servers
                    .iter_mut()
                    .find(|server| server.id == server_id)
                    .expect("checked server");
                server.revision = next_mcp_revision(server.revision)?;
                server.enabled = enabled;
                cancel_mcp_runs(&mut next, &revoke)?;
            }
            SceneCommand::RemoveMcpServer {
                server_id,
                expected_revision,
            } => {
                let server = mcp_server(&next, &server_id)?;
                check_mcp_revision(server, expected_revision)?;
                let revoke = granted_agents(server);
                next.mcp_servers.retain(|server| server.id != server_id);
                cancel_mcp_runs(&mut next, &revoke)?;
            }
        }
        let snapshot = self.persist(next)?;
        if seal_geometry_group {
            self.geometry_group = None;
        }
        Ok(snapshot)
    }

    pub fn set_background(&mut self, scene_id: &str, asset: Asset) -> Result<Snapshot> {
        validate_background(&asset)?;
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        target.background = Some(asset);
        target.regions.clear();
        target.refs.retain(|r| r.kind != ReferenceKind::Region);
        touch(target);
        self.persist(next)
    }

    /// Adopt a host-generated board, its full-frame drawing region and reference
    /// in one transaction. Existing content is never replaced by a blank board.
    pub fn create_blackboard(&mut self, scene_id: &str, asset: Asset) -> Result<Snapshot> {
        validate_background(&asset)?;
        let original = scene(&self.state, scene_id)?;
        if self.state.active_scene_id != scene_id || original.closed || original.frozen {
            return Err(invalid("会话已变化"));
        }
        if is_running(original) {
            return Err(CoreError::RunInProgress);
        }
        let connection_id = original.connection_id.clone();
        let needs_scene = original.background.is_some() || !original.regions.is_empty()
            || !original.items.is_empty() || !original.messages.is_empty()
            || !original.draft.is_empty();
        let mut next = self.state.clone();
        if needs_scene {
            create_scene(&mut next)?;
        }
        let active_id = next.active_scene_id.clone();
        let target = scene_mut(&mut next, &active_id)?;
        let region = Region {
            id: Uuid::new_v4().to_string(), x: 0.0, y: 0.0,
            width: asset.width.unwrap() as f64, height: asset.height.unwrap() as f64,
            ..Region::default()
        };
        target.title = "黑板".into();
        target.connection_id = connection_id;
        target.background = Some(asset);
        target.refs = vec![Reference { kind: ReferenceKind::Region, id: region.id.clone() }];
        target.regions = vec![region];
        touch(target);
        self.persist(next)
    }

    /// Read the current background identity without cloning the scene or store.
    /// Frozen scenes remain readable; closed or replaced targets do not.
    pub fn background_for_preview(&self, scene_id: &str, background_id: &str) -> Result<Asset> {
        let target = scene(&self.state, scene_id)?;
        if target.closed {
            return Err(invalid("会话已关闭，请先恢复会话"));
        }
        target
            .background
            .as_ref()
            .filter(|background| background.id == background_id)
            .cloned()
            .ok_or(CoreError::DrawingConflict)
    }

    /// Pointer-driven sampling is only valid on the current visible scene.
    /// Borrow its metadata and clone one Asset, never a Snapshot or Scene.
    pub fn active_background_for_preview(
        &self,
        scene_id: &str,
        background_id: &str,
    ) -> Result<Asset> {
        if self.state.active_scene_id != scene_id {
            return Err(CoreError::DrawingConflict);
        }
        let target = scene(&self.state, scene_id)?;
        if target.frozen || target.closed {
            return Err(CoreError::DrawingConflict);
        }
        self.background_for_preview(scene_id, background_id)
    }

    /// Only replacement-image pixels of this exact live region may be read.
    /// Geometry changes also advance drawing_revision, so no whole scene clone
    /// or caller-supplied path is needed for asynchronous preview fences.
    pub fn region_image_for_preview(
        &self,
        scene_id: &str,
        region_id: &str,
        background_id: &str,
        source_id: &str,
        drawing_revision: u64,
    ) -> Result<Asset> {
        let target = scene(&self.state, scene_id)?;
        if target.closed
            || target.background.as_ref().map(|asset| asset.id.as_str()) != Some(background_id)
        {
            return Err(CoreError::DrawingConflict);
        }
        target
            .regions
            .iter()
            .find(|region| region.id == region_id && region.drawing_revision == drawing_revision)
            .and_then(|region| region.image_override.as_ref())
            .filter(|source| source.id == source_id)
            .cloned()
            .ok_or(CoreError::DrawingConflict)
    }

    /// The capture host owns file registration. Replacing a region keeps its
    /// identity, composer references and runs; it cannot replace a replacement
    /// again or masquerade as a reversible drawing edit.
    pub fn set_region_image(&mut self, target: &OcrTarget, asset: Asset) -> Result<Snapshot> {
        validate_region_image(&asset)?;
        let mut next = self.state.clone();
        let target_scene = next
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::DrawingConflict)?;
        let index = crate::ocr::target_region(target_scene, target)
            .map_err(|_| CoreError::DrawingConflict)?;
        let background = target_scene
            .background
            .as_ref()
            .ok_or(CoreError::DrawingConflict)?;
        let background_width = background.width.ok_or(CoreError::DrawingConflict)? as f64;
        let background_height = background.height.ok_or(CoreError::DrawingConflict)? as f64;
        let region = &mut target_scene.regions[index];
        if region.image_override.is_some() {
            return Err(invalid("此选区已是替换图片，请重新截图后开始长截图"));
        }
        let scale = (region.width.min(background_width) / asset.width.unwrap() as f64)
            .min(background_height * 0.85 / asset.height.unwrap() as f64);
        region.width = asset.width.unwrap() as f64 * scale;
        region.height = asset.height.unwrap() as f64 * scale;
        region.x = region.x.clamp(0., background_width - region.width);
        region.y = region.y.clamp(0., background_height - region.height);
        region.drawing_revision = crate::drawing::next_revision(region)?;
        region.image_override = Some(asset);
        region.drawings.clear();
        region.drawing_history = DrawingHistory::default();
        region.ocr = None;
        region.translation = None;
        touch(target_scene);
        self.persist(next)
    }

    /// Only completed assistant messages are ever persisted. Read one immutable
    /// result for local table viewing/export, including frozen or closed history.
    pub fn completed_message_text(&self, scene_id: &str, message_id: &str) -> Result<String> {
        let target = scene(&self.state, scene_id)?;
        target
            .messages
            .iter()
            .find(|message| message.id == message_id && message.role == MessageRole::Assistant)
            .map(|message| message.text.clone())
            .ok_or_else(|| invalid("完整回答不存在"))
    }

    /// Capture the exact on-screen image document and placement for an
    /// independent pinned copy. The host must recompare both values after
    /// rendering: translated pixels can change without drawing_revision.
    pub fn region_for_pin(&self, target: &OcrTarget) -> Result<(Asset, Region)> {
        if self.state.active_scene_id != target.scene_id {
            return Err(CoreError::DrawingConflict);
        }
        let target_scene = scene(&self.state, &target.scene_id)?;
        if target_scene.frozen || target_scene.closed {
            return Err(CoreError::DrawingConflict);
        }
        let index = crate::ocr::target_region(target_scene, target)
            .map_err(|_| CoreError::DrawingConflict)?;
        let background = target_scene
            .background
            .as_ref()
            .ok_or(CoreError::DrawingConflict)?;
        Ok((background.clone(), target_scene.regions[index].clone()))
    }

    /// Capture only the image and current layers, without cloning other scenes
    /// or retaining old OCR/undo documents during the native operation.
    pub fn region_for_ocr(&self, target: &OcrTarget) -> Result<(Asset, Region)> {
        let target_scene = self
            .state
            .scenes
            .iter()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::OcrConflict)?;
        let index = crate::ocr::target_region(target_scene, target)?;
        let region = &target_scene.regions[index];
        let (source, x, y, width, height) = if let Some(source) = &region.image_override {
            (
                source,
                0.,
                0.,
                source.width.ok_or(CoreError::OcrConflict)? as f64,
                source.height.ok_or(CoreError::OcrConflict)? as f64,
            )
        } else {
            (
                target_scene
                    .background
                    .as_ref()
                    .ok_or(CoreError::OcrConflict)?,
                region.x,
                region.y,
                region.width,
                region.height,
            )
        };
        Ok((
            source.clone(),
            Region {
                id: region.id.clone(),
                x,
                y,
                width,
                height,
                image_override: None,
                drawings: region.drawings.clone(),
                drawing_revision: region.drawing_revision,
                drawing_history: DrawingHistory::default(),
                ocr: None,
                translation: region.translation.clone(),
            },
        ))
    }

    /// Translation reads source pixels with privacy mosaics, excluding other
    /// drawings, prior translated pixels and selection/history metadata.
    pub fn region_for_translation(&self, target: &TranslationTarget) -> Result<(Asset, Region)> {
        let (source, mut region) = self.region_for_ocr(target).map_err(|error| match error {
            CoreError::OcrConflict => CoreError::TranslationConflict,
            other => other,
        })?;
        region
            .drawings
            .retain(|drawing| drawing.kind == DrawingKind::Mosaic);
        region.translation = None;
        Ok((source, region))
    }

    pub fn set_region_translation(
        &mut self,
        target: &TranslationTarget,
        document: TranslationDocument,
        overlay: Asset,
        selection: OcrDocument,
    ) -> Result<Snapshot> {
        validate_translation_document(&document)?;
        validate_region_image(&overlay)?;
        let mut next = self.state.clone();
        let scene = next
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::TranslationConflict)?;
        let index =
            crate::ocr::target_region(scene, target).map_err(|_| CoreError::TranslationConflict)?;
        let source_id = scene.regions[index]
            .image_override
            .as_ref()
            .or(scene.background.as_ref())
            .ok_or(CoreError::TranslationConflict)?
            .id
            .clone();
        let saved = SavedTranslation {
            background_id: target.background_id.clone(),
            drawing_revision: target.drawing_revision,
            source_id,
            document,
            overlay,
            selection,
        };
        let region = &mut scene.regions[index];
        if region.translation.as_ref() == Some(&saved) && region.ocr.is_none() {
            return Ok(self.snapshot());
        }
        region.translation = Some(saved);
        region.ocr = None;
        touch(scene);
        self.persist(next)
    }

    pub fn clear_region_translation(&mut self, target: &TranslationTarget) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let scene = next
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::TranslationConflict)?;
        let index =
            crate::ocr::target_region(scene, target).map_err(|_| CoreError::TranslationConflict)?;
        if scene.regions[index].translation.is_none() {
            return Ok(self.snapshot());
        }
        scene.regions[index].translation = None;
        scene.regions[index].ocr = None;
        touch(scene);
        self.persist(next)
    }

    /// Recognition completion is independent of whichever scene is visible.
    /// The host additionally fences the lifetime/authorization of its OCR job.
    pub fn set_region_ocr(
        &mut self,
        target: &OcrTarget,
        document: OcrDocument,
    ) -> Result<Snapshot> {
        validate_ocr_document(&document)?;
        let mut next = self.state.clone();
        let target_scene = next
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::OcrConflict)?;
        let index = crate::ocr::target_region(target_scene, target)?;
        let result = RegionOcr {
            background_id: target.background_id.clone(),
            drawing_revision: target.drawing_revision,
            document,
        };
        let region = &mut target_scene.regions[index];
        if region.ocr.as_ref() == Some(&result) {
            return Ok(self.snapshot());
        }
        region.ocr = Some(result);
        touch(target_scene);
        self.persist(next)
    }

    pub fn clear_region_ocr(&mut self, target: &OcrTarget) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let target_scene = next
            .scenes
            .iter_mut()
            .find(|scene| scene.id == target.scene_id)
            .ok_or(CoreError::OcrConflict)?;
        let index = crate::ocr::target_region(target_scene, target)?;
        if target_scene.regions[index].ocr.take().is_none() {
            return Ok(self.snapshot());
        }
        touch(target_scene);
        self.persist(next)
    }

    pub fn add_asset(&mut self, scene_id: &str, asset: Asset) -> Result<Snapshot> {
        validate_asset(&asset)?;
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        target.items.push(SpaceItem {
            id: id(),
            asset,
            x: 0.32,
            y: 0.2,
            width: 0.36,
            height: 0.5,
            state: None,
            video_edit: None,
            video_annotations: None,
        });
        touch(target);
        self.persist(next)
    }

    /// Register a user-selected batch and append its references in one commit.
    /// File decoding/copying belongs to the host; this final admission rejects a
    /// dialog that returned after its original space was frozen or replaced.
    pub fn import_assets(&mut self, scene_id: &str, assets: Vec<Asset>) -> Result<Snapshot> {
        let target = scene(&self.state, scene_id)?;
        if self.state.active_scene_id != scene_id || target.frozen || target.closed {
            return Err(invalid("会话已切换或关闭，请重新导入"));
        }
        if assets.is_empty() || assets.len() > 12 {
            return Err(invalid("每次可导入 1 至 12 个文件"));
        }
        unique(assets.iter().map(|asset| asset.id.as_str()), "导入素材")?;
        for asset in &assets {
            validate_asset(asset)?;
            if !matches!(
                asset.kind,
                AssetKind::Image | AssetKind::Text | AssetKind::Html | AssetKind::Svg
            ) {
                return Err(invalid("此类素材不能通过文件导入"));
            }
        }
        let incoming: HashSet<&str> = assets.iter().map(|asset| asset.id.as_str()).collect();
        // Asset IDs resolve globally in the host. Reusing one anywhere in the
        // history could otherwise resolve a different file than this new item.
        for existing in &self.state.scenes {
            let registered =
                existing
                    .background
                    .iter()
                    .chain(existing.items.iter().map(|item| &item.asset))
                    .chain(
                        existing
                            .regions
                            .iter()
                            .filter_map(|region| region.image_override.as_ref()),
                    )
                    .chain(existing.regions.iter().filter_map(|region| {
                        region.translation.as_ref().map(|saved| &saved.overlay)
                    }))
                    .chain(
                        existing
                            .messages
                            .iter()
                            .flat_map(|message| message.attachments.iter().flatten()),
                    );
            if registered
                .into_iter()
                .any(|asset| incoming.contains(asset.id.as_str()))
            {
                return Err(invalid("导入素材 ID 已存在"));
            }
        }
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        for asset in assets {
            let item_id = id();
            target.refs.push(Reference {
                kind: ReferenceKind::Item,
                id: item_id.clone(),
            });
            target.items.push(SpaceItem {
                id: item_id,
                asset,
                x: 0.32,
                y: 0.2,
                width: 0.36,
                height: 0.5,
                state: None,
                video_edit: None,
                video_annotations: None,
            });
        }
        touch(target);
        self.persist(next)
    }

    /// A desktop image object can be referenced by several independent scenes.
    /// Only the host supplies its immutable asset; generic position IPC cannot
    /// register or replace sources. Existing references are idempotent.
    pub fn reference_shared_image(&mut self, scene_id: &str, asset: Asset) -> Result<Snapshot> {
        let target = scene(&self.state, scene_id)?;
        if self.state.active_scene_id != scene_id || target.frozen || target.closed || is_running(target) || target.blackboard_link.is_some() {
            return Err(invalid("会话已切换或不可引用"));
        }
        validate_asset(&asset)?;
        if asset.kind != AssetKind::Image { return Err(invalid("此对象不是图片")); }
        for existing in self.state.scenes.iter().flat_map(|scene| scene.items.iter()) {
            if existing.asset.id == asset.id && existing.asset != asset { return Err(invalid("图片来源已变更")); }
        }
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        let item_id = if let Some(item) = target.items.iter().find(|item| item.asset == asset) { item.id.clone() } else {
            let item_id = id();
            target.items.push(SpaceItem { id: item_id.clone(), asset, x: 0.32, y: 0.2, width: 0.36, height: 0.5, state: None, video_edit: None, video_annotations: None });
            item_id
        };
        let reference = Reference { kind: ReferenceKind::Item, id: item_id };
        if target.refs.contains(&reference) { return Ok(self.snapshot()); }
        target.refs.push(reference);
        touch(target);
        self.persist(next)
    }

    pub fn save_connection(&mut self, connection: Connection) -> Result<Snapshot> {
        validate_connection(&connection)?;
        let mut profile = self
            .state
            .default_connection_id
            .as_deref()
            .and_then(|id| self.state.connections.iter().find(|p| p.id == id))
            .cloned()
            .ok_or(CoreError::ConnectionUnselected)?;
        let expected = profile.revision;
        profile.revision = expected
            .checked_add(1)
            .ok_or(CoreError::ConnectionConflict)?;
        profile.base_url = connection.base_url;
        profile.model = connection.model;
        profile.has_key = connection.has_key;
        profile.credential_id = if profile.has_key {
            profile.credential_id.or_else(|| Some(id()))
        } else {
            None
        };
        self.save_connection_profile(profile, Some(expected))
    }

    /// Only the native credential owner may call this method. Generic scene
    /// commands cannot set has_key or a credential reference.
    pub fn save_connection_profile(
        &mut self,
        profile: ConnectionProfile,
        expected_revision: Option<u64>,
    ) -> Result<Snapshot> {
        validate_connection_profile(&profile)?;
        let mut next = self.state.clone();
        if let Some(index) = next.connections.iter().position(|p| p.id == profile.id) {
            let current = &next.connections[index];
            if expected_revision != Some(current.revision)
                || current.revision.checked_add(1) != Some(profile.revision)
            {
                return Err(CoreError::ConnectionConflict);
            }
            cancel_connection_runs(&mut next, &profile.id)?;
            next.connections[index] = profile;
        } else {
            if expected_revision.is_some()
                || profile.revision != 1
                || profile.id == LEGACY_CONNECTION_ID
            {
                return Err(CoreError::ConnectionConflict);
            }
            next.connections.push(profile);
        }
        crate::connections::sync_compatibility(&mut next);
        self.persist(next)
    }

    pub fn remove_connection_profile(
        &mut self,
        connection_id: &str,
        expected_revision: u64,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let profile = next
            .connections
            .iter()
            .find(|p| p.id == connection_id)
            .ok_or_else(|| not_found("连接", connection_id))?;
        if profile.revision != expected_revision {
            return Err(CoreError::ConnectionConflict);
        }
        cancel_connection_runs(&mut next, connection_id)?;
        next.connections.retain(|p| p.id != connection_id);
        if next.default_connection_id.as_deref() == Some(connection_id) {
            next.default_connection_id = None;
        }
        for agent in &mut next.agents {
            if agent.default_connection_id.as_deref() == Some(connection_id) {
                agent.default_connection_id = None;
            }
        }
        for scene in &mut next.scenes {
            if scene.connection_id.as_deref() == Some(connection_id) {
                scene.connection_id = None;
                touch(scene);
            }
        }
        crate::connections::sync_compatibility(&mut next);
        self.persist(next)
    }

    pub fn connection_for_scene(&self, scene_id: &str) -> Result<ConnectionProfile> {
        let target = scene(&self.state, scene_id)?;
        let connection_id = target
            .connection_id
            .as_deref()
            .ok_or(CoreError::ConnectionUnselected)?;
        self.state
            .connections
            .iter()
            .find(|profile| profile.id == connection_id)
            .cloned()
            .ok_or_else(|| not_found("连接", connection_id))
    }

    /// Register a finalized recording and replace its original region atomically.
    /// The captured background identity is a fence against late completion.
    pub fn finish_recording(
        &mut self,
        scene_id: &str,
        background_id: &str,
        region: &Region,
        asset: Asset,
    ) -> Result<Snapshot> {
        validate_asset(&asset)?;
        if asset.kind != AssetKind::Video {
            return Err(invalid("录屏必须是视频素材"));
        }
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        let background = target
            .background
            .as_ref()
            .ok_or_else(|| invalid("录屏背景已失效"))?;
        if background.id != background_id || !target.regions.contains(region) {
            return Err(invalid("录屏区域已改变，拒绝替换"));
        }
        let width = background.width.ok_or_else(|| invalid("背景尺寸缺失"))? as f64;
        let height = background.height.ok_or_else(|| invalid("背景尺寸缺失"))? as f64;
        let item_id = id();
        target.items.push(SpaceItem {
            id: item_id.clone(),
            asset,
            x: region.x / width,
            y: region.y / height,
            width: region.width / width,
            height: region.height / height,
            state: Some(std::collections::BTreeMap::from([
                ("coordinateSpace".into(), serde_json::json!("background")),
                ("backgroundId".into(), serde_json::json!(background_id)),
            ])),
            video_edit: None,
            video_annotations: None,
        });
        target.regions.retain(|r| r.id != region.id);
        target
            .refs
            .retain(|r| r.kind != ReferenceKind::Region || r.id != region.id);
        target.refs.push(Reference {
            kind: ReferenceKind::Item,
            id: item_id,
        });
        touch(target);
        self.persist(next)
    }

    pub fn begin_run(&mut self, scene_id: &str) -> Result<RunContext> {
        self.begin_run_with_input(scene_id, None, None, None)
    }

    pub fn begin_run_with_memory(
        &mut self,
        scene_id: &str,
        grant: Option<MemoryAuthorityGrant>,
    ) -> Result<RunContext> {
        self.begin_run_with_input(scene_id, None, grant.as_ref(), None)
    }

    /// Start an explicit region workflow without consuming the user's pending
    /// composer input. Store the real prompt in history and model context alike;
    /// the host may show a short tool title separately and restrict its tools.
    pub fn begin_workflow(
        &mut self,
        scene_id: &str,
        prompt: &str,
        region_id: &str,
    ) -> Result<RunContext> {
        bounded_text(prompt, 16_000, "工作流提示")?;
        if prompt.contains('\0') {
            return Err(invalid("工作流提示不能包含空字符"));
        }
        self.begin_run_with_input(scene_id, Some((prompt, region_id)), None, None)
    }

    fn begin_run_with_input(
        &mut self,
        scene_id: &str,
        workflow: Option<(&str, &str)>,
        grant: Option<&MemoryAuthorityGrant>,
        video_input: Option<VerifiedVideoRunInput>,
    ) -> Result<RunContext> {
        let connection_profile = self.connection_for_scene(scene_id)?;
        let memory_authority = if workflow.is_some() {
            None
        } else {
            crate::external_memory::capture_authority(&self.db, &self.state, scene_id, grant)?
        };
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        if target.closed {
            return Err(invalid("会话已关闭，请先恢复会话"));
        }
        if is_running(target) {
            return Err(CoreError::RunInProgress);
        }
        let (text, refs) = if let Some((prompt, region_id)) = workflow {
            let video_workflow = video_input.is_some();
            if if video_workflow {
                !target
                    .items
                    .iter()
                    .any(|item| item.id == region_id && item.asset.kind == AssetKind::Video)
            } else {
                target.background.is_none()
                    || !target.regions.iter().any(|region| region.id == region_id)
            } {
                return Err(not_found("区域", region_id));
            }
            (
                prompt.to_owned(),
                vec![Reference {
                    kind: if video_workflow {
                        ReferenceKind::Item
                    } else {
                        ReferenceKind::Region
                    },
                    id: region_id.into(),
                }],
            )
        } else {
            if target.draft.trim().is_empty() && target.refs.is_empty() {
                return Err(CoreError::EmptyInput);
            }
            (std::mem::take(&mut target.draft), target.refs.clone())
        };
        let run_id = id();
        let conversation_start = target
            .conversation_start
            .filter(|start| *start > 0 && *start == target.messages.len());
        target.messages.push(Message {
            id: id(),
            role: MessageRole::User,
            text,
            reasoning: None,
            conversation_start,
            created_at: now(),
            run_id: Some(run_id.clone()),
            refs: Some(refs.clone()),
            attachments: None,
            tool_steps: vec![],
        });
        target.run = Some(Run {
            id: run_id.clone(),
            kind: if workflow.is_some() {
                RecordedRunKind::Workflow
            } else {
                RecordedRunKind::Chat
            },
            status: RunStatus::Running,
            connection_id: Some(connection_profile.id.clone()),
            connection_revision: Some(connection_profile.revision),
            memory_authority: memory_authority.clone(),
            error: None,
            steps: vec![],
        });
        if let Some(input) = &video_input {
            target
                .messages
                .last_mut()
                .expect("new user message")
                .attachments = Some(input.assets().to_vec());
        }
        touch(target);
        let target = scene(&next, scene_id)?;
        let mut agent = next
            .agents
            .iter()
            .find(|agent| agent.id == target.agent_id)
            .ok_or_else(|| not_found("Agent", &target.agent_id))?
            .clone();
        if workflow.is_some() {
            agent.memory_enabled = false;
        }
        let context = RunContext {
            scene_id: scene_id.into(),
            run_id,
            memory_authority: memory_authority.clone(),
            memories: if matches!(memory_authority, Some(RunMemoryAuthority::Local { .. })) {
                let query: String = target
                    .messages
                    .last()
                    .map(|message| message.text.chars().take(200).collect())
                    .unwrap_or_default();
                crate::memory::recall(&self.db, &target.agent_id, &query)
                    .map_err(crate::memory::query_error)?
            } else {
                vec![]
            },
            agent,
            mcp_servers: if workflow.is_some() {
                vec![]
            } else {
                next.mcp_servers
                    .iter()
                    .filter(|server| {
                        server.enabled
                            && server.grants.iter().any(|grant| {
                                grant.agent_id == target.agent_id && !grant.tool_names.is_empty()
                            })
                    })
                    .map(|server| mcp_for_agent(server, &target.agent_id))
                    .collect()
            },
            connection: connection_profile.connection(),
            connection_profile,
            messages: target
                .messages
                .iter()
                .skip(target.conversation_start.unwrap_or(0))
                .cloned()
                .collect(),
            background: target.background.clone(),
            regions: target.regions.clone(),
            items: target.items.clone(),
            refs,
            projection: RunProjection::Conversation,
        };
        self.persist_change_with_video_input(
            next,
            None,
            video_input.map(|input| (context.run_id.clone(), input)),
        )?;
        Ok(context)
    }

    /// Bind the host's prepared immutable images or documents to this run's user message.
    /// Repeating the same attachment registration is safe; replacement is forbidden.
    pub fn attach_run_assets(
        &mut self,
        scene_id: &str,
        run_id: &str,
        assets: Vec<Asset>,
    ) -> Result<Snapshot> {
        validate_attachments(&assets)?;
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        require_running(target, run_id)?;
        let message = target
            .messages
            .last_mut()
            .ok_or_else(|| invalid("运行缺少用户消息"))?;
        if message.role != MessageRole::User || message.run_id.as_deref() != Some(run_id) {
            return Err(invalid("只能为当前运行的最后一条用户消息绑定素材"));
        }
        if let Some(existing) = &message.attachments {
            if *existing == assets {
                return Ok(self.snapshot());
            }
            return Err(invalid("已绑定的运行素材不可替换"));
        }
        message.attachments = Some(assets);
        touch(target);
        self.persist(next)
    }

    pub fn finish_run(
        &mut self,
        scene_id: &str,
        run_id: &str,
        text: impl Into<String>,
    ) -> Result<Snapshot> {
        self.finish_run_with_assets(scene_id, run_id, text, vec![])
    }

    /// Commit the assistant text and generated scene objects as one transaction.
    pub fn finish_run_with_assets(
        &mut self,
        scene_id: &str,
        run_id: &str,
        text: impl Into<String>,
        assets: Vec<Asset>,
    ) -> Result<Snapshot> {
        self.finish_run_with_reasoning_and_assets(scene_id, run_id, text, None, assets)
    }

    pub fn finish_run_with_reasoning_and_assets(
        &mut self,
        scene_id: &str,
        run_id: &str,
        text: impl Into<String>,
        reasoning: Option<String>,
        assets: Vec<Asset>,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        require_running(target, run_id)?;
        let text = text.into();
        if text.trim().is_empty() {
            return Err(invalid("AI 返回了空内容，不能作为成功结果保存"));
        }
        let reasoning = reasoning.filter(|value| !value.trim().is_empty());
        if reasoning
            .as_ref()
            .is_some_and(|value| value.len() > 2 * 1024 * 1024)
        {
            return Err(invalid("思考文字超过长度上限"));
        }
        target.messages.push(Message {
            id: id(),
            role: MessageRole::Assistant,
            text,
            reasoning,
            conversation_start: None,
            created_at: now(),
            run_id: Some(run_id.into()),
            refs: None,
            attachments: None,
            tool_steps: vec![],
        });
        for asset in assets {
            validate_asset(&asset)?;
            target.items.push(SpaceItem {
                id: id(),
                asset,
                x: 0.32,
                y: 0.2,
                width: 0.36,
                height: 0.5,
                state: None,
                video_edit: None,
                video_annotations: None,
            });
        }
        let run = target.run.as_mut().expect("checked running run");
        run.status = RunStatus::Completed;
        settle_steps(run, "运行已结束，工具未完成");
        archive_steps(target)?;
        touch(target);
        // Completing a background run never changes the active scene or freeze state.
        self.persist(next)
    }

    pub fn fail_run(
        &mut self,
        scene_id: &str,
        run_id: &str,
        error: impl Into<String>,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        require_running(target, run_id)?;
        let run = target.run.as_mut().expect("checked running run");
        run.status = RunStatus::Failed;
        run.error = Some(error.into());
        settle_steps(run, "运行失败，工具已停止");
        archive_steps(target)?;
        touch(target);
        self.persist(next)
    }

    pub fn cancel_run(&mut self, scene_id: &str) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        if !is_running(target) {
            return Err(CoreError::StaleRun);
        }
        let run = target.run.as_mut().expect("checked running run");
        run.status = RunStatus::Canceled;
        run.error = None;
        settle_steps(run, "运行已取消");
        archive_steps(target)?;
        touch(target);
        self.persist(next)
    }

    /// Only the host may register a catalog after successful process discovery.
    /// Rediscovery requires fresh approval even if a server advertises identical tools.
    pub fn upsert_mcp_server(
        &mut self,
        server_id: Option<String>,
        expected_revision: Option<u64>,
        label: String,
        command: McpCommand,
        tools: Vec<McpTool>,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        let label = label.trim().to_string();
        if let Some(server_id) = server_id {
            let current = mcp_server(&next, &server_id)?;
            if expected_revision != Some(current.revision) {
                return Err(CoreError::McpConflict);
            }
            let revision = next_mcp_revision(current.revision)?;
            let revoke = granted_agents(current);
            let server = next
                .mcp_servers
                .iter_mut()
                .find(|server| server.id == server_id)
                .expect("checked server");
            *server = McpServer {
                id: server_id,
                label,
                revision,
                command,
                tools,
                grants: vec![],
                enabled: server.enabled,
            };
            cancel_mcp_runs(&mut next, &revoke)?;
        } else {
            if expected_revision.is_some() {
                return Err(CoreError::McpConflict);
            }
            next.mcp_servers.push(McpServer {
                id: id(),
                label,
                revision: 1,
                command,
                tools,
                grants: vec![],
                enabled: true,
            });
        }
        self.persist(next)
    }

    /// Return only this Agent's grant. The full catalog remains ordered so its
    /// stable index matches the host's model-visible MCP tool name.
    pub fn mcp_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        server_id: &str,
        revision: u64,
        tool_name: &str,
    ) -> Result<McpServer> {
        let target = scene(&self.state, scene_id)?;
        require_running(target, run_id)?;
        let server = mcp_server(&self.state, server_id)?;
        check_mcp_revision(server, revision)?;
        if !server.enabled
            || !server.tools.iter().any(|tool| tool.name == tool_name)
            || !server.grants.iter().any(|grant| {
                grant.agent_id == target.agent_id
                    && grant.tool_names.iter().any(|name| name == tool_name)
            })
        {
            return Err(CoreError::McpPermissionDenied);
        }
        Ok(mcp_for_agent(server, &target.agent_id))
    }

    /// Permission validation and step registration share the exclusive Store
    /// borrow and one revision-fenced commit. The host must recheck before I/O
    /// and cancel its transport when a later configuration commit cancels Run.
    pub fn begin_mcp_tool(
        &mut self,
        scene_id: &str,
        run_id: &str,
        tool_call_id: &str,
        server_id: &str,
        revision: u64,
        tool_name: &str,
    ) -> Result<Snapshot> {
        let server = self.mcp_for_run(scene_id, run_id, server_id, revision, tool_name)?;
        let index = server
            .tools
            .iter()
            .position(|tool| tool.name == tool_name)
            .expect("checked tool");
        let server_uuid = Uuid::parse_str(&server.id).map_err(|_| invalid("MCP 服务 ID 无效"))?;
        let name = format!("mcp_{}_{}", server_uuid.simple(), index);
        let label = format!(
            "{} · {}",
            server.label.chars().take(32).collect::<String>(),
            tool_name.chars().take(45).collect::<String>()
        );
        self.begin_tool(scene_id, run_id, tool_call_id, &name, &label)
    }

    pub fn begin_tool(
        &mut self,
        scene_id: &str,
        run_id: &str,
        tool_call_id: &str,
        name: &str,
        label: &str,
    ) -> Result<Snapshot> {
        bounded_text(tool_call_id, 200, "工具调用 ID")?;
        bounded_text(name, 128, "工具名称")?;
        bounded_text(label, 80, "工具标签")?;
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        require_running(target, run_id)?;
        let run = target.run.as_mut().expect("checked running run");
        if run.steps.len() >= 64 {
            return Err(invalid("每次运行最多记录 64 个工具步骤"));
        }
        if run.steps.iter().any(|step| step.id == tool_call_id) {
            return Err(invalid("工具调用 ID 重复，不能再次执行"));
        }
        run.steps.push(ToolStep {
            id: tool_call_id.into(),
            name: name.into(),
            label: label.into(),
            status: ToolStepStatus::Running,
            started_at: now(),
            finished_at: None,
            summary: None,
        });
        touch(target);
        self.persist(next)
    }

    pub fn finish_tool(
        &mut self,
        scene_id: &str,
        run_id: &str,
        tool_call_id: &str,
        status: ToolStepStatus,
        summary: impl Into<String>,
    ) -> Result<Snapshot> {
        if status == ToolStepStatus::Running {
            return Err(invalid("工具结算状态不能是运行中"));
        }
        let summary = summary.into();
        if summary.chars().count() > 2000 {
            return Err(invalid("工具摘要不能超过 2000 个字符"));
        }
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, scene_id)?;
        require_running(target, run_id)?;
        let step = target
            .run
            .as_mut()
            .expect("checked running run")
            .steps
            .iter_mut()
            .find(|step| step.id == tool_call_id && step.status == ToolStepStatus::Running)
            .ok_or(CoreError::StaleTool)?;
        step.status = status;
        step.finished_at = Some(now().max(step.started_at));
        step.summary = (!summary.is_empty()).then_some(summary);
        touch(target);
        self.persist(next)
    }

    /// Resolve Agent and provenance inside the fenced transaction. The caller
    /// cannot choose another Agent or forge a source scene/message.
    /// This commit is independent of finish_tool: a later trace-write failure or
    /// cancellation does not roll back an already committed memory mutation.
    pub fn memory_for_run(
        &mut self,
        scene_id: &str,
        run_id: &str,
        mutation: MemoryMutation,
    ) -> Result<Snapshot> {
        let (agent_id, source) = local_memory_scope(&self.state, scene_id, run_id)?;
        self.persist_memory(self.state.clone(), agent_id, mutation, Some(source))
    }

    pub fn memories_for_run(&self, scene_id: &str, run_id: &str) -> Result<Vec<MemoryEntry>> {
        self.recall_for_run(scene_id, run_id, "")
    }

    pub fn recall_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        query: &str,
    ) -> Result<Vec<MemoryEntry>> {
        let (agent_id, _) = local_memory_scope(&self.state, scene_id, run_id)?;
        crate::memory::recall(&self.db, &agent_id, query).map_err(crate::memory::query_error)
    }

    pub fn memory_page(
        &self,
        agent_id: &str,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MemoryPage> {
        if !self.state.agents.iter().any(|agent| agent.id == agent_id) {
            return Err(not_found("Agent", agent_id));
        }
        crate::memory::page(&self.db, agent_id, query, cursor, limit)
            .map_err(crate::memory::query_error)
    }

    pub fn memory_entry(&self, agent_id: &str, id: &str) -> Result<Option<MemoryEntry>> {
        if !self.state.agents.iter().any(|agent| agent.id == agent_id) {
            return Err(not_found("Agent", agent_id));
        }
        crate::memory::entry(&self.db, agent_id, id)
    }

    /// Search bounded textual history. Asset metadata is deliberately absent.
    /// Only the latest 200 same-Agent messages are considered, with at most
    /// 16,000 Unicode scalar values scanned per message and five 800-char hits.
    pub fn search_history_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        query: &str,
    ) -> Result<Vec<HistoryMatch>> {
        let (agent_id, _) = memory_scope(&self.state, scene_id, run_id)?;
        let query = query.trim();
        bounded_text(query, 200, "历史查询")?;
        let mut recent: Vec<(&Scene, &Message)> = Vec::new();
        for scene in self
            .state
            .scenes
            .iter()
            .filter(|scene| scene.agent_id == agent_id)
        {
            for message in &scene.messages {
                if message.run_id.as_deref() == Some(run_id) {
                    continue;
                }
                let position = recent
                    .binary_search_by(|(_, previous)| {
                        previous
                            .created_at
                            .cmp(&message.created_at)
                            .reverse()
                            .then_with(|| previous.id.cmp(&message.id))
                    })
                    .unwrap_or_else(|position| position);
                if position < 200 {
                    recent.insert(position, (scene, message));
                    recent.truncate(200);
                }
            }
        }
        let mut matches = Vec::new();
        for (scene, message) in recent {
            let text: String = message.text.chars().take(16_000).collect();
            let Some(index) = text.find(query) else {
                continue;
            };
            let start = text[..index].chars().count().saturating_sub(100);
            matches.push(HistoryMatch {
                scene_id: scene.id.clone(),
                message_id: message.id.clone(),
                role: message.role.clone(),
                text: text.chars().skip(start).take(800).collect(),
                created_at: message.created_at,
            });
            if matches.len() == 5 {
                break;
            }
        }
        Ok(matches)
    }

    fn ensure_current_revision(&mut self) -> Result<()> {
        let transaction = self
            .db
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let version: u32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(version as u64));
        }
        let app_id: i64 =
            transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if app_id != APPLICATION_ID {
            return Err(invalid("数据库应用标识已改变"));
        }
        let (schema_version, revision): (u32, i64) = transaction.query_row(
            "SELECT schema_version, revision FROM app_state WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if schema_version != SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(schema_version as u64));
        }
        if revision != self.revision {
            return Err(CoreError::ConcurrentModification);
        }
        transaction.commit()?;
        Ok(())
    }

    fn persist(&mut self, next: Snapshot) -> Result<Snapshot> {
        self.persist_change(next, None)
    }

    fn persist_memory(
        &mut self,
        next: Snapshot,
        agent_id: String,
        mutation: MemoryMutation,
        source: Option<MemorySource>,
    ) -> Result<Snapshot> {
        if !next.agents.iter().any(|agent| agent.id == agent_id) {
            return Err(not_found("Agent", &agent_id));
        }
        self.persist_change(next, Some((agent_id, mutation, source)))
    }

    fn persist_change(
        &mut self,
        next: Snapshot,
        mutation: Option<(String, MemoryMutation, Option<MemorySource>)>,
    ) -> Result<Snapshot> {
        self.persist_change_with_video_input(next, mutation, None)
    }

    fn persist_change_with_video_input(
        &mut self,
        mut next: Snapshot,
        mutation: Option<(String, MemoryMutation, Option<MemorySource>)>,
        initial_video: Option<(String, VerifiedVideoRunInput)>,
    ) -> Result<Snapshot> {
        let source_set_changed = self
            .geometry_group
            .as_ref()
            .is_some_and(|group| group.source_set_changed(&self.state, &next));
        crate::geometry_history::cleanup(&self.state, &mut next)?;
        validate_snapshot(&next)?;
        let transaction = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(version as u64));
        }
        let app_id: i64 =
            transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if app_id != APPLICATION_ID {
            return Err(invalid("数据库应用标识已改变"));
        }
        if let Some((agent_id, mutation, source)) = mutation {
            crate::memory::mutate(&transaction, &agent_id, mutation, source)?;
        }
        crate::external_memory::on_snapshot_change(&transaction, &self.state, &next)?;
        crate::journal::on_snapshot_change(&transaction, &self.state, &mut next)?;
        if let Some((run_id, input)) = initial_video {
            crate::video_annotations::register_input(&transaction, &mut next, &run_id, input)?;
        }
        validate_snapshot(&next)?;
        next.memories.clear();
        crate::rich_annotations::validate(&transaction, &next)?;
        crate::video_registered_layouts::validate(&transaction, &next)?;
        crate::video_annotations::validate_ledger(&transaction, &next)?;
        next.memory_stats = crate::memory::stats(&transaction, &next)?;
        let payload = serde_json::to_string(&next)?;
        let changed = transaction.execute(
            "UPDATE app_state SET payload = ?1, revision = revision + 1 WHERE id = 1 AND revision = ?2 AND schema_version = 1",
            params![payload, self.revision],
        )?;
        if changed != 1 {
            return Err(CoreError::ConcurrentModification);
        }
        transaction.commit()?;
        self.revision += 1;
        self.state = next;
        if source_set_changed
            || self
                .geometry_group
                .as_ref()
                .is_some_and(|group| !group.is_current(&self.state))
        {
            self.geometry_group = None;
        }
        Ok(self.state.clone())
    }
}

fn initial_snapshot() -> Snapshot {
    let agent = AgentProfile {
        id: id(),
        name: "Mewu".into(),
        instructions: "你是 Mewu，与用户在同一个 AI 空间中协作。".into(),
        memory: String::new(),
        memory_enabled: true,
        memory_provider: MemoryProviderSelection::default(),
        default_connection_id: None,
    };
    let first = new_scene(&agent.id, None);
    let mut state = Snapshot {
        schema_version: SCHEMA_VERSION,
        active_scene_id: first.id.clone(),
        scenes: vec![first],
        agents: vec![agent],
        memories: vec![],
        memory_stats: vec![],
        mcp_servers: vec![],
        connection: Connection {
            base_url: "https://api.openai.com/v1".into(),
            model: String::new(),
            has_key: false,
        },
        connections: vec![],
        default_connection_id: None,
    };
    crate::connections::migrate_legacy(&mut state);
    state
}

fn new_scene(agent_id: &str, connection_id: Option<String>) -> Scene {
    let timestamp = now();
    Scene {
        blackboard_link: None,
        id: id(),
        title: "新场景".into(),
        agent_id: agent_id.into(),
        connection_id,
        created_at: timestamp,
        updated_at: timestamp,
        frozen: false,
        minimized: false,
        closed: false,
        background: None,
        regions: vec![],
        geometry_history: RegionGeometryHistory::default(),
        items: vec![],
        refs: vec![],
        draft: String::new(),
        messages: vec![],
        conversation_start: None,
        run: None,
    }
}

fn create_scene(state: &mut Snapshot) -> Result<()> {
    let agent_id = scene(state, &state.active_scene_id)?.agent_id.clone();
    let connection_id = state
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .and_then(|agent| agent.default_connection_id.clone())
        .or_else(|| state.default_connection_id.clone());
    for candidate in &mut state.scenes {
        candidate.frozen = true;
    }
    let created = new_scene(&agent_id, connection_id);
    state.active_scene_id = created.id.clone();
    state.scenes.push(created);
    Ok(())
}

fn ensure_connection_exists(state: &Snapshot, connection_id: Option<&str>) -> Result<()> {
    if let Some(id) = connection_id {
        if !state.connections.iter().any(|profile| profile.id == id) {
            return Err(not_found("连接", id));
        }
    }
    Ok(())
}

fn cancel_connection_runs(state: &mut Snapshot, connection_id: &str) -> Result<()> {
    for target in &mut state.scenes {
        if is_running(target)
            && target
                .run
                .as_ref()
                .is_some_and(|run| run.connection_id.as_deref() == Some(connection_id))
        {
            let run = target.run.as_mut().expect("checked running run");
            run.status = RunStatus::Canceled;
            run.error = Some("连接配置已更改".into());
            settle_steps(run, "连接配置已更改");
            archive_steps(target)?;
            touch(target);
        }
    }
    Ok(())
}

fn scene<'a>(state: &'a Snapshot, scene_id: &str) -> Result<&'a Scene> {
    state
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .ok_or_else(|| not_found("场景", scene_id))
}
fn edit_drawing(
    state: &mut Snapshot,
    scene_id: &str,
    region_id: &str,
    background_id: &str,
    expected_revision: u64,
    mutation: crate::drawing::Mutation,
) -> Result<bool> {
    let target = scene_mut(state, scene_id)?;
    if target.closed {
        return Err(invalid("会话已关闭，请先恢复会话"));
    }
    let background = target
        .background
        .as_ref()
        .filter(|b| b.id == background_id)
        .ok_or(CoreError::DrawingConflict)?;
    let region = target
        .regions
        .iter_mut()
        .find(|r| r.id == region_id)
        .ok_or(CoreError::DrawingConflict)?;
    if region.drawing_revision != expected_revision {
        return Err(CoreError::DrawingConflict);
    }
    let changed = crate::drawing::apply(region, background, mutation)?;
    if changed {
        region.ocr = None;
        if let Some(translation) = &mut region.translation {
            translation.drawing_revision = region.drawing_revision;
        }
        touch(target);
    }
    Ok(changed)
}

fn scene_mut<'a>(state: &'a mut Snapshot, scene_id: &str) -> Result<&'a mut Scene> {
    state
        .scenes
        .iter_mut()
        .find(|s| s.id == scene_id)
        .ok_or_else(|| not_found("场景", scene_id))
}
fn is_running(scene: &Scene) -> bool {
    scene
        .run
        .as_ref()
        .is_some_and(|r| r.status == RunStatus::Running)
}
fn require_running(scene: &Scene, run_id: &str) -> Result<()> {
    if scene.closed {
        return Err(CoreError::StaleRun);
    }
    match &scene.run {
        Some(run) if run.id == run_id && run.status == RunStatus::Running => Ok(()),
        _ => Err(CoreError::StaleRun),
    }
}

/// A historical run keeps the range of its own conversation after later resets.
pub(crate) fn original_conversation_range(
    scene: &Scene,
    user_ordinal: usize,
) -> Result<std::ops::RangeInclusive<usize>> {
    if !scene
        .messages
        .get(user_ordinal)
        .is_some_and(|message| message.role == MessageRole::User)
    {
        return Err(CoreError::JournalConflict);
    }
    let start = scene.messages[..=user_ordinal]
        .iter()
        .enumerate()
        .filter_map(|(ordinal, message)| message.conversation_start.map(|start| (ordinal, start)))
        .last();
    let start = match start {
        Some((ordinal, start)) if start == ordinal && start <= user_ordinal => start,
        Some(_) => return Err(CoreError::JournalConflict),
        None => 0,
    };
    Ok(start..=user_ordinal)
}

fn mcp_server<'a>(state: &'a Snapshot, server_id: &str) -> Result<&'a McpServer> {
    state
        .mcp_servers
        .iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| not_found("MCP 服务", server_id))
}

fn check_mcp_revision(server: &McpServer, expected: u64) -> Result<()> {
    if server.revision != expected {
        return Err(CoreError::McpConflict);
    }
    Ok(())
}

fn next_mcp_revision(revision: u64) -> Result<u64> {
    revision
        .checked_add(1)
        .ok_or_else(|| invalid("MCP 配置修订号已达上限"))
}

fn granted_agents(server: &McpServer) -> HashSet<String> {
    server
        .grants
        .iter()
        .filter(|grant| !grant.tool_names.is_empty())
        .map(|grant| grant.agent_id.clone())
        .collect()
}

fn mcp_for_agent(server: &McpServer, agent_id: &str) -> McpServer {
    let mut server = server.clone();
    server.grants.retain(|grant| grant.agent_id == agent_id);
    server
}

fn cancel_mcp_runs(state: &mut Snapshot, agents: &HashSet<String>) -> Result<()> {
    for target in state
        .scenes
        .iter_mut()
        .filter(|target| agents.contains(&target.agent_id) && is_running(target))
    {
        let run = target.run.as_mut().expect("checked running run");
        run.status = RunStatus::Canceled;
        run.error = Some("工具配置已更改".into());
        settle_steps(run, "工具配置已更改");
        archive_steps(target)?;
        touch(target);
    }
    Ok(())
}

fn plain_mcp_text(text: &str, limit: usize, kind: &str) -> Result<()> {
    bounded_text(text, limit, kind)?;
    if text.chars().any(char::is_control) {
        return Err(invalid(format!("{kind}不能包含控制字符")));
    }
    Ok(())
}

fn json_depth(value: &serde_json::Value, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match value {
        serde_json::Value::Object(values) => {
            values.values().all(|value| json_depth(value, depth + 1))
        }
        serde_json::Value::Array(values) => values.iter().all(|value| json_depth(value, depth + 1)),
        _ => true,
    }
}

fn validate_mcp(state: &Snapshot) -> Result<()> {
    if state.mcp_servers.len() > 8 {
        return Err(invalid("最多添加 8 个 MCP 服务"));
    }
    unique(
        state.mcp_servers.iter().map(|server| server.id.as_str()),
        "MCP 服务",
    )?;
    for server in &state.mcp_servers {
        if Uuid::parse_str(&server.id).is_err() || server.revision == 0 {
            return Err(invalid("MCP 服务 ID 或修订号无效"));
        }
        plain_mcp_text(&server.label, 80, "MCP 服务名称")?;
        plain_mcp_text(&server.command.executable, 4096, "MCP 程序路径")?;
        if let Some(cwd) = &server.command.cwd {
            plain_mcp_text(cwd, 4096, "MCP 工作目录")?;
        }
        if server.command.args.len() > 64
            || server
                .command
                .args
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
            || server.command.executable.len()
                + server.command.cwd.as_ref().map_or(0, String::len)
                + server.command.args.iter().map(String::len).sum::<usize>()
                > 16 * 1024
        {
            return Err(invalid(
                "MCP 命令最多 64 个参数、合计 16 KiB，参数不能包含控制字符",
            ));
        }
        if server.tools.len() > 64 || serde_json::to_vec(&server.tools)?.len() > 256 * 1024 {
            return Err(invalid("MCP 工具目录最多 64 项、256 KiB"));
        }
        unique(
            server.tools.iter().map(|tool| tool.name.as_str()),
            "MCP 工具",
        )?;
        for tool in &server.tools {
            plain_mcp_text(&tool.name, 128, "MCP 工具名称")?;
            if tool.description.chars().count() > 4000 {
                return Err(invalid("MCP 工具描述最多 4000 个字符"));
            }
            if !tool.input_schema.is_object()
                || tool
                    .input_schema
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    != Some("object")
                || !json_depth(&tool.input_schema, 1)
                || serde_json::to_vec(&tool.input_schema)?.len() > 16 * 1024
            {
                return Err(invalid(
                    "MCP 输入 schema 必须声明 object 类型，深度最多 32、大小最多 16 KiB",
                ));
            }
        }
        unique(
            server.grants.iter().map(|grant| grant.agent_id.as_str()),
            "MCP 授权 Agent",
        )?;
        for grant in &server.grants {
            if !state.agents.iter().any(|agent| agent.id == grant.agent_id) {
                return Err(invalid("MCP 授权 Agent 不存在"));
            }
            unique(grant.tool_names.iter().map(String::as_str), "MCP 授权工具")?;
            if grant
                .tool_names
                .iter()
                .any(|name| !server.tools.iter().any(|tool| tool.name == *name))
            {
                return Err(invalid("MCP 授权工具不存在"));
            }
        }
    }
    for agent in &state.agents {
        let count: usize = state
            .mcp_servers
            .iter()
            .flat_map(|server| &server.grants)
            .filter(|grant| grant.agent_id == agent.id)
            .map(|grant| grant.tool_names.len())
            .sum();
        if count > 60 {
            return Err(invalid("每个 Agent 最多授权 60 个 MCP 工具"));
        }
    }
    Ok(())
}

fn bounded_text(text: &str, limit: usize, kind: &str) -> Result<()> {
    if text.trim().is_empty() || text.chars().count() > limit {
        return Err(invalid(format!("{kind}不能为空且不能超过 {limit} 个字符")));
    }
    Ok(())
}

fn settle_steps(run: &mut Run, reason: &str) {
    for step in &mut run.steps {
        if step.status == ToolStepStatus::Running {
            step.status = ToolStepStatus::Failed;
            step.finished_at = Some(now().max(step.started_at));
            step.summary = Some(reason.into());
        }
    }
}

/// Keep a terminal run's trace with its user message before the next run replaces it.
fn archive_steps(scene: &mut Scene) -> Result<()> {
    let run = scene.run.as_ref().ok_or(CoreError::StaleRun)?;
    let message = scene
        .messages
        .iter_mut()
        .find(|message| {
            message.role == MessageRole::User && message.run_id.as_deref() == Some(run.id.as_str())
        })
        .ok_or_else(|| invalid("运行缺少用户消息"))?;
    message.tool_steps = run.steps.clone();
    Ok(())
}

fn validate_steps(steps: &[ToolStep], allow_running: bool) -> Result<()> {
    unique(steps.iter().map(|step| step.id.as_str()), "工具调用")?;
    if steps.len() > 64 {
        return Err(invalid("工具步骤过多"));
    }
    for step in steps {
        bounded_text(&step.id, 200, "工具调用 ID")?;
        bounded_text(&step.name, 128, "工具名称")?;
        bounded_text(&step.label, 80, "工具标签")?;
        if step
            .summary
            .as_ref()
            .is_some_and(|summary| summary.chars().count() > 2000)
            || step
                .finished_at
                .is_some_and(|finished| finished < step.started_at)
            || (step.status == ToolStepStatus::Running) != step.finished_at.is_none()
            || (!allow_running && step.status == ToolStepStatus::Running)
        {
            return Err(invalid("工具步骤状态、时间或摘要无效"));
        }
    }
    Ok(())
}

fn local_memory_scope(
    state: &Snapshot,
    scene_id: &str,
    run_id: &str,
) -> Result<(String, MemorySource)> {
    let result = memory_scope(state, scene_id, run_id)?;
    let target = scene(state, scene_id)?;
    if !matches!(
        target
            .run
            .as_ref()
            .and_then(|r| r.memory_authority.as_ref()),
        Some(RunMemoryAuthority::Local { .. })
    ) {
        return Err(CoreError::MemoryDisabled);
    }
    Ok(result)
}

fn memory_scope(state: &Snapshot, scene_id: &str, run_id: &str) -> Result<(String, MemorySource)> {
    let target = scene(state, scene_id)?;
    require_running(target, run_id)?;
    let agent = state
        .agents
        .iter()
        .find(|agent| agent.id == target.agent_id)
        .ok_or_else(|| not_found("Agent", &target.agent_id))?;
    if !agent.memory_enabled {
        return Err(CoreError::MemoryDisabled);
    }
    let authority = target
        .run
        .as_ref()
        .and_then(|r| r.memory_authority.as_ref());
    let current = match authority {
        Some(RunMemoryAuthority::Local { selection_revision }) => {
            agent.memory_provider.binding_id.is_none()
                && *selection_revision == agent.memory_provider.revision
        }
        Some(RunMemoryAuthority::External {
            selection_revision,
            grant,
            ..
        }) => {
            agent.memory_provider.binding_id.as_deref() == Some(grant.binding_id.as_str())
                && *selection_revision == agent.memory_provider.revision
        }
        None => false,
    };
    if !current {
        return Err(CoreError::MemoryDisabled);
    }
    let message = target
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::User && message.run_id.as_deref() == Some(run_id)
        })
        .ok_or_else(|| invalid("运行缺少用户消息"))?;
    Ok((
        agent.id.clone(),
        MemorySource {
            scene_id: scene_id.into(),
            message_id: message.id.clone(),
        },
    ))
}

fn validate_memories(state: &Snapshot) -> Result<()> {
    unique(
        state.memories.iter().map(|memory| memory.id.as_str()),
        "记忆",
    )?;
    for memory in &state.memories {
        if !state.agents.iter().any(|agent| agent.id == memory.agent_id) {
            return Err(invalid("记忆关联的 Agent 不存在"));
        }
        bounded_text(&memory.text, 2000, "记忆")?;
        if memory.revision == 0 || memory.updated_at < memory.created_at {
            return Err(invalid("记忆修订号或时间无效"));
        }
        if let Some(source) = &memory.source {
            let valid = state
                .scenes
                .iter()
                .find(|scene| scene.id == source.scene_id && scene.agent_id == memory.agent_id)
                .is_some_and(|scene| {
                    scene.messages.iter().any(|message| {
                        message.id == source.message_id && message.role == MessageRole::User
                    })
                });
            if !valid {
                return Err(invalid("记忆来源必须属于同一 Agent 的现存用户消息"));
            }
        }
    }
    Ok(())
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn touch(scene: &mut Scene) {
    scene.updated_at = now().max(scene.updated_at);
}
fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::Invalid(message.into())
}
fn not_found(kind: &'static str, id: &str) -> CoreError {
    CoreError::NotFound {
        kind,
        id: id.into(),
    }
}

fn unique<'a>(ids: impl Iterator<Item = &'a str>, kind: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        if id.trim().is_empty() || !seen.insert(id) {
            return Err(invalid(format!("{kind} ID 为空或重复")));
        }
    }
    Ok(())
}

fn validate_connection(connection: &Connection) -> Result<()> {
    let url = connection.base_url.trim();
    if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(invalid("连接地址须使用 HTTP 或 HTTPS"));
    }
    if url.contains(['@', '?', '#']) || url.chars().any(char::is_whitespace) {
        return Err(invalid(
            "连接地址不能包含凭据、查询参数、片段或空白；密钥应存入系统凭据库",
        ));
    }
    Ok(())
}

fn validate_asset(asset: &Asset) -> Result<()> {
    if asset.id.trim().is_empty() || asset.path.trim().is_empty() || asset.name.trim().is_empty() {
        return Err(invalid("素材缺少 ID、名称或路径"));
    }
    if asset.width == Some(0) || asset.height == Some(0) {
        return Err(invalid("素材尺寸必须大于零"));
    }
    if asset.kind == AssetKind::Text
        && (asset.width.is_some()
            || asset.height.is_some()
            || asset.origin_x.is_some()
            || asset.origin_y.is_some()
            || asset.scale_factor.is_some())
    {
        return Err(invalid("文本素材不能包含图像尺寸或屏幕坐标"));
    }
    if asset
        .scale_factor
        .is_some_and(|s| !s.is_finite() || s <= 0.0)
    {
        return Err(invalid("无效素材缩放比例"));
    }
    Ok(())
}

fn validate_background(asset: &Asset) -> Result<()> {
    validate_asset(asset)?;
    if asset.kind != AssetKind::Image || asset.width.is_none() || asset.height.is_none() {
        return Err(invalid("场景背景必须是带像素尺寸的图片"));
    }
    Ok(())
}

fn validate_region_image(asset: &Asset) -> Result<()> {
    validate_background(asset)?;
    let width = asset.width.unwrap();
    let height = asset.height.unwrap();
    if Uuid::parse_str(&asset.id).is_err()
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32 * 1024 * 1024
        || asset.origin_x.is_some_and(|value| value != 0)
        || asset.origin_y.is_some_and(|value| value != 0)
        || asset.scale_factor.is_some_and(|value| value != 1.)
    {
        return Err(invalid(
            "替换图片标识或像素尺寸无效；单边最多 16384 像素，总计最多 3200 万像素",
        ));
    }
    Ok(())
}

fn validate_attachments(assets: &[Asset]) -> Result<()> {
    if assets.len() > 6 {
        return Err(invalid("每条消息最多包含六个附件"));
    }
    unique(assets.iter().map(|a| a.id.as_str()), "消息附件")?;
    for asset in assets {
        validate_asset(asset)?;
        if !matches!(
            asset.kind,
            AssetKind::Image | AssetKind::Text | AssetKind::Html | AssetKind::Svg
        ) {
            return Err(invalid("运行附件只支持图片、文本、HTML 或 SVG"));
        }
    }
    Ok(())
}

fn validate_bounds(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    max_width: f64,
    max_height: f64,
) -> Result<()> {
    if ![x, y, width, height].iter().all(|v| v.is_finite())
        || x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > max_width + 0.000001
        || y + height > max_height + 0.000001
    {
        return Err(invalid("区域或素材位置超出有效坐标范围"));
    }
    Ok(())
}

fn read_stored_snapshot(db: &SqlConnection, migrate_connections: bool) -> Result<(Snapshot, i64)> {
    let count: i64 = db.query_row("SELECT count(*) FROM app_state", [], |row| row.get(0))?;
    if count != 1 {
        return Err(invalid("场景存储必须包含且只包含一份应用状态"));
    }
    let (stored_version, payload, revision): (u32, String, i64) = db.query_row(
        "SELECT schema_version, payload, revision FROM app_state WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if stored_version != SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema(stored_version as u64));
    }
    if revision < 0 {
        return Err(invalid("无效存储修订号"));
    }
    let value: serde_json::Value = serde_json::from_str(&payload)?;
    let body_version = value
        .get("schemaVersion")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| invalid("缺少有效 schemaVersion"))?;
    if body_version != SCHEMA_VERSION as u64 {
        return Err(CoreError::UnsupportedSchema(body_version));
    }
    let has_connections = value.get("connections").is_some();
    if has_connections != value.get("defaultConnectionId").is_some()
        || (!has_connections && !migrate_connections)
    {
        return Err(invalid("连接目录字段缺失"));
    }
    let mut state: Snapshot = serde_json::from_value(value)?;
    if !has_connections {
        crate::connections::migrate_legacy(&mut state);
    }
    validate_snapshot(&state)?;
    Ok((state, revision))
}

fn validate_snapshot(state: &Snapshot) -> Result<()> {
    if state.schema_version != SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema(state.schema_version as u64));
    }
    unique(state.agents.iter().map(|a| a.id.as_str()), "Agent")?;
    unique(state.scenes.iter().map(|s| s.id.as_str()), "场景")?;
    if state.agents.is_empty() || state.scenes.is_empty() {
        return Err(invalid("缺少 Agent 或场景"));
    }
    if !state.scenes.iter().any(|s| s.id == state.active_scene_id) {
        return Err(invalid("活动场景不存在"));
    }
    validate_connection(&state.connection)?;
    crate::connections::validate(state)?;
    validate_memories(state)?;
    validate_mcp(state)?;
    for agent in &state.agents {
        if agent.name.trim().is_empty() {
            return Err(invalid("Agent 名称不能为空"));
        }
    }
    for target in &state.scenes {
        if let Some(link) = &target.blackboard_link {
            if link.item_id.is_empty() || link.parent_scene_id == target.id
                || !state.scenes.iter().any(|parent| parent.id == link.parent_scene_id && parent.blackboard_link.is_none())
                || state.scenes.iter().any(|other| other.id != target.id && other.blackboard_link.as_ref() == Some(link)) {
                return Err(invalid("黑板关联无效"));
            }
            blackboard_store::board_region(target)?;
        }
        if target.closed && (target.id == state.active_scene_id || is_running(target)) {
            return Err(invalid("已关闭会话不能是活动会话或继续运行"));
        }
        if target.frozen == (target.id == state.active_scene_id) {
            return Err(invalid("活动场景与冻结状态不一致"));
        }
        if !state.agents.iter().any(|a| a.id == target.agent_id) {
            return Err(invalid("场景关联的 Agent 不存在"));
        }
        if target.updated_at < target.created_at {
            return Err(invalid("场景更新时间早于创建时间"));
        }
        if target.title.trim().is_empty() {
            return Err(invalid("场景标题为空"));
        }
        unique(target.regions.iter().map(|r| r.id.as_str()), "区域")?;
        unique(target.items.iter().map(|i| i.id.as_str()), "素材项")?;
        unique(target.messages.iter().map(|m| m.id.as_str()), "消息")?;
        if target
            .conversation_start
            .is_some_and(|start| start > target.messages.len())
        {
            return Err(invalid("会话上下文起点无效"));
        }
        if let Some(start) = target
            .conversation_start
            .filter(|start| *start > 0 && *start < target.messages.len())
        {
            if target.messages[start].conversation_start != Some(start) {
                return Err(invalid("会话首条消息缺少上下文起点"));
            }
        }
        for (ordinal, message) in target.messages.iter().enumerate() {
            if message.reasoning.as_ref().is_some_and(|value| {
                message.role != MessageRole::Assistant || value.len() > 2 * 1024 * 1024
            }) {
                return Err(invalid("公开思考文字无效"));
            }
            if message.conversation_start.is_some_and(|start| {
                start == 0
                    || start != ordinal
                    || message.role != MessageRole::User
                    || start > target.conversation_start.unwrap_or(0)
            }) {
                return Err(invalid("消息会话起点无效"));
            }
            validate_steps(&message.tool_steps, false)?;
            if !message.tool_steps.is_empty()
                && (message.role != MessageRole::User || message.run_id.is_none())
            {
                return Err(invalid("工具历史必须关联用户运行消息"));
            }
            if let Some(attachments) = &message.attachments {
                validate_attachments(attachments)?;
            }
        }
        if let Some(background) = &target.background {
            validate_background(background)?;
        }
        for region in &target.regions {
            let background = target
                .background
                .as_ref()
                .ok_or_else(|| invalid("没有背景的场景不能包含截图区域"))?;
            validate_bounds(
                region.x,
                region.y,
                region.width,
                region.height,
                background.width.unwrap() as f64,
                background.height.unwrap() as f64,
            )?;
            if let Some(source) = &region.image_override {
                validate_region_image(source)?;
                if source.id == background.id || region.drawing_revision == 0 {
                    return Err(invalid("替换图片身份或修订号无效"));
                }
            }
            crate::drawing::validate(region, Some(background))?;
            crate::ocr::validate_region(region, background)?;
            if let Some(translation) = &region.translation {
                validate_asset(&translation.overlay)?;
            }
            crate::translation::validate_region(region, background)?;
        }
        crate::geometry_history::validate(target)?;
        for item in &target.items {
            validate_asset(&item.asset)?;
            crate::blackboard_objects::valid_link(state, target, item)?;
            if ![item.x, item.y, item.width, item.height]
                .iter()
                .all(|v| v.is_finite())
                || item.x.abs() > 16.
                || item.y.abs() > 16.
                || item.width <= 0.
                || item.height <= 0.
                || item.width > 8.
                || item.height > 8.
            {
                return Err(invalid("素材位置或尺寸无效"));
            }
            crate::video_edit::validate(item)?;
            crate::video_annotations::validate_item(item)?;
        }
        let mut references = HashSet::new();
        for reference in &target.refs {
            let exists = match reference.kind {
                ReferenceKind::Region => target.regions.iter().any(|r| r.id == reference.id),
                ReferenceKind::Item => target.items.iter().any(|i| i.id == reference.id),
            };
            let key = (
                matches!(reference.kind, ReferenceKind::Region),
                reference.id.as_str(),
            );
            if !exists || !references.insert(key) {
                return Err(invalid("引用不存在或重复"));
            }
        }
        if let Some(run) = &target.run {
            validate_steps(&run.steps, run.status == RunStatus::Running)?;
            if run.id.trim().is_empty() {
                return Err(invalid("运行 ID 为空"));
            }
            let user = target.messages.iter().any(|m| {
                m.role == MessageRole::User && m.run_id.as_deref() == Some(run.id.as_str())
            });
            let assistant = target.messages.iter().any(|m| {
                m.role == MessageRole::Assistant && m.run_id.as_deref() == Some(run.id.as_str())
            });
            if !user || (run.status == RunStatus::Completed) != assistant {
                return Err(invalid("运行结算与消息不一致"));
            }
            let user = target
                .messages
                .iter()
                .find(|m| {
                    m.role == MessageRole::User && m.run_id.as_deref() == Some(run.id.as_str())
                })
                .expect("checked user message");
            if (run.status == RunStatus::Running && !user.tool_steps.is_empty())
                || (run.status != RunStatus::Running && user.tool_steps != run.steps)
            {
                return Err(invalid("工具历史与运行结算不一致"));
            }
        }
    }
    Ok(())
}

impl Store {
    pub fn video_source(&self, scene_id: &str, item_id: &str) -> Result<VideoSourceView> {
        crate::video_edit::source(&self.state, scene_id, item_id)
    }

    pub fn video_export_matches(&self, origin: &VideoExportOrigin) -> bool {
        crate::video_edit::export_matches(&self.state, origin)
    }

    pub fn set_video_range(
        &mut self,
        target: &VideoTarget,
        verified: &VerifiedVideoSource,
        from: Option<VideoRange>,
        to: Option<VideoRange>,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        if !crate::video_edit::set(&mut next, target, verified, from, to)? {
            self.ensure_current_revision()?;
            return Ok(self.snapshot());
        }
        touch(scene_mut(&mut next, &target.scene_id)?);
        self.persist(next)
    }

    pub fn undo_video_range(
        &mut self,
        target: &VideoTarget,
        expected_operation_id: &str,
        from: Option<VideoRange>,
    ) -> Result<Snapshot> {
        self.replay_video_range(target, expected_operation_id, from, false)
    }

    pub fn redo_video_range(
        &mut self,
        target: &VideoTarget,
        expected_operation_id: &str,
        from: Option<VideoRange>,
    ) -> Result<Snapshot> {
        self.replay_video_range(target, expected_operation_id, from, true)
    }

    fn replay_video_range(
        &mut self,
        target: &VideoTarget,
        expected_operation_id: &str,
        from: Option<VideoRange>,
        redo: bool,
    ) -> Result<Snapshot> {
        let mut next = self.state.clone();
        crate::video_edit::replay(&mut next, target, expected_operation_id, from, redo)?;
        touch(scene_mut(&mut next, &target.scene_id)?);
        self.persist(next)
    }
}
