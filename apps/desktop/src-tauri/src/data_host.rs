// SPDX-License-Identifier: MPL-2.0
use crate::{
    data_storage::{self as storage, DataBucket, DataCategory, Inventory},
    storage_host, Host,
};
use mewu_core::{HistoryCleanupPlan, Store};
use serde::Serialize;
use std::{path::Path, sync::atomic::Ordering, time::SystemTime};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataUsage {
    pub files: DataBucket,
    pub screenshots: DataBucket,
    pub conversations: DataBucket,
    pub database_bytes: u64,
    pub can_clean: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCleanupReceipt {
    removed_count: u64,
    removed_bytes: u64,
    failed_count: u64,
    compacted: bool,
    usage: DataUsage,
}
fn database_bytes(root: &Path) -> Result<u64, String> {
    ["spaces.db", "spaces.db-wal", "spaces.db-shm"]
        .iter()
        .try_fold(0, |total, name| {
            match std::fs::symlink_metadata(root.join(name)) {
                Ok(meta) => {
                    crate::asset_locations::ordinary(&meta, false)
                        .map_err(|_| "数据库文件不能是链接")?;
                    Ok(total + meta.len())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(total),
                Err(_) => Err("无法读取数据库占用".into()),
            }
        })
}
fn inspect(
    app: &AppHandle,
    store: &Store,
    owner: usize,
) -> Result<(DataUsage, Inventory, HistoryCleanupPlan), String> {
    let host = app.state::<Host>();
    let plan = store.history_cleanup_plan().map_err(|e| e.to_string())?;
    let mut references = store.storage_assets();
    references.extend(crate::pin_host::storage_assets(app)?);
    let inventory = storage::inventory(&host.assets, &references, SystemTime::now(), owner)?;
    let conversations = DataBucket {
        category: DataCategory::Conversations,
        count: plan.count,
        bytes: plan.bytes,
        cleanable_count: plan.cleanable_count,
        cleanable_bytes: plan.cleanable_bytes,
        token: storage::token(&(owner, &host.root, &plan))?,
    };
    Ok((
        DataUsage {
            files: inventory.files.clone(),
            screenshots: inventory.screenshots.clone(),
            conversations,
            database_bytes: database_bytes(&host.root)?,
            can_clean: host.exit.accepts_new_operations()
                && !host.capturing.load(Ordering::Acquire)
                && crate::recording::ensure_idle(app).is_ok(),
        },
        inventory,
        plan,
    ))
}
#[tauri::command]
pub async fn get_data_usage(app: AppHandle, window: WebviewWindow) -> Result<DataUsage, String> {
    let owner = storage_host::owner(&window)?;
    storage_host::current_owner(&app, &window, owner)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    let permit = host.storage.maintenance()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let host = app.state::<Host>();
        let engine = host.lock()?;
        storage_host::current_owner(&app, &window, owner)?;
        Ok(inspect(&app, &engine.store, owner)?.0)
    })
    .await
    .map_err(|_| "数据统计中断".to_string())?
}
#[tauri::command]
pub async fn clean_data(
    app: AppHandle,
    window: WebviewWindow,
    category: DataCategory,
    expected_token: String,
) -> Result<DataCleanupReceipt, String> {
    if expected_token.len() != 64 || !expected_token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("清理范围无效，请刷新".into());
    }
    let owner = storage_host::owner(&window)?;
    storage_host::current_owner(&app, &window, owner)?;
    let host = app.state::<Host>();
    host.exit.ensure_new_operation()?;
    crate::recording::ensure_idle(&app)?;
    let permit = host.storage.maintenance()?;
    let transition =
        crate::SpaceTransition::try_begin(&host.capturing).ok_or("请先完成采集或空间切换")?;
    tauri::async_runtime::spawn_blocking(move || {
        let (_permit, _transition) = (permit, transition);
        let host = app.state::<Host>();
        let mut engine = host.lock()?;
        host.exit.ensure_new_operation()?;
        crate::recording::ensure_idle(&app)?;
        storage_host::current_owner(&app, &window, owner)?;
        let (before, inventory, plan) = inspect(&app, &engine.store, owner)?;
        let bucket = match category {
            DataCategory::Files => &before.files,
            DataCategory::Screenshots => &before.screenshots,
            DataCategory::Conversations => &before.conversations,
        };
        if bucket.token != expected_token {
            return Err("清理范围已变化，请刷新后重新确认".into());
        }
        let (mut removed_count, mut removed_bytes, mut failed_count, mut compacted) =
            (0, 0, 0, true);
        let mut snapshot = None;
        if category == DataCategory::Conversations && plan.cleanable_count > 0 {
            snapshot = Some(
                engine
                    .store
                    .clean_history(&plan)
                    .map_err(|e| e.to_string())?,
            );
            removed_count = plan.cleanable_count;
            // Commit survives a compacting failure (e.g. disk full). Report the
            // actual byte change and remaining database size, never roll back
            // only the renderer or pretend deleted data is still present.
            compacted = engine.store.compact_history_storage().is_ok();
            removed_bytes = before
                .database_bytes
                .saturating_sub(database_bytes(&host.root)?);
        } else {
            for (_, candidate) in inventory.candidates.iter().filter(|(c, _)| *c == category) {
                if host.exit.ensure_new_operation().is_err()
                    || storage_host::current_owner(&app, &window, owner).is_err()
                {
                    failed_count += 1;
                    continue;
                }
                match storage::remove(candidate) {
                    Ok(bytes) => {
                        removed_count += 1;
                        removed_bytes += bytes;
                    }
                    Err(_) => failed_count += 1,
                }
            }
        }
        let usage = inspect(&app, &engine.store, owner).map(|value| value.0);
        drop(engine);
        if let Some(snapshot) = snapshot {
            crate::publish(&app, &snapshot);
        }
        let mut usage = usage?;
        // The SpaceTransition remains held until this worker has fully settled.
        usage.can_clean =
            host.exit.accepts_new_operations() && crate::recording::ensure_idle(&app).is_ok();
        Ok(DataCleanupReceipt {
            removed_count,
            removed_bytes,
            failed_count,
            compacted,
            usage,
        })
    })
    .await
    .map_err(|_| "数据清理中断，请刷新占用".to_string())?
}
