// SPDX-License-Identifier: MPL-2.0
//! Text edits create new immutable assets. Source files and parent messages stay intact.
use crate::{Host, HostSnapshot};
use mewu_core::{Asset, AssetKind, Snapshot, Store};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};
struct NewText {
    path: PathBuf,
    adopted: bool,
}
impl Drop for NewText {
    fn drop(&mut self) {
        if !self.adopted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn save(
    store: &mut Store,
    root: &Path,
    scene_id: &str,
    item_id: &str,
    expected_asset_id: &str,
    text: &str,
) -> Result<Snapshot, String> {
    if text.len() > crate::assets::MAX_TEXT || text.contains('\0') {
        return Err("文本超出限制或包含无效字符".into());
    }
    let snapshot = store.snapshot();
    let item = snapshot
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .and_then(|s| s.items.iter().find(|i| i.id == item_id))
        .ok_or("文本对象已移除")?;
    if item.asset.id != expected_asset_id
        || item.asset.kind != AssetKind::Text
        || !item.asset.name.to_ascii_lowercase().ends_with(".txt")
    {
        return Err("文本对象已变化，请重新读取".into());
    }
    let _parents = crate::asset_locations::pin_ancestors(root).map_err(|_| "文本素材目录不可用")?;
    let _source = crate::asset_locations::resolve_path(Path::new(&item.asset.path), root)?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.join(format!("{id}.txt"));
    let mut file: File = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "无法保存文本")?;
    let mut output = NewText {
        path: path.clone(),
        adopted: false,
    };
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存文本")?;
    drop(file);
    let replacement = Asset {
        id,
        path: path.to_string_lossy().into_owned(),
        ..item.asset.clone()
    };
    let result = store
        .update_blackboard_text(scene_id, item_id, expected_asset_id, replacement)
        .map_err(|e| e.to_string())?;
    output.adopted = true;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mewu-board-text-{}", uuid::Uuid::new_v4()));
            assert!(
                path.is_absolute()
                    && !path
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with("c:\\hermes")
            );
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self
                .0
                .canonicalize()
                .unwrap()
                .starts_with(std::env::temp_dir().canonicalize().unwrap()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mewu-board-text-"));
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn board(f: &Fixture) -> (Store, String, String, Asset, String) {
        let mut store = Store::open_in_memory().unwrap();
        let parent = store.snapshot().active_scene_id;
        let id = uuid::Uuid::new_v4().to_string();
        let path = f.0.join(format!("{id}.txt"));
        std::fs::write(&path, "original\r\n中文").unwrap();
        let asset = Asset {
            id,
            name: "笔记.txt".into(),
            kind: AssetKind::Text,
            path: path.to_string_lossy().into_owned(),
            width: None,
            height: None,
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        store.add_asset(&parent, asset.clone()).unwrap();
        let bid = uuid::Uuid::new_v4().to_string();
        let background = Asset {
            id: bid.clone(),
            name: "黑板.png".into(),
            kind: AssetKind::Image,
            path: f
                .0
                .join(format!("{bid}.board.png"))
                .to_string_lossy()
                .into_owned(),
            width: Some(1280),
            height: Some(720),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        };
        let state = store
            .create_blackboard_document(&parent, background)
            .unwrap();
        let scene = state
            .scenes
            .iter()
            .find(|s| s.id == state.active_scene_id)
            .unwrap();
        let item = scene.items[0].id.clone();
        (store, scene.id.clone(), item, asset, parent)
    }
    #[test]
    fn editing_txt_creates_utf8_copy_preserves_source_parent_geometry_and_ai_reference() {
        let f = Fixture::new();
        let (mut store, scene, item, asset, parent) = board(&f);
        let prior = store
            .snapshot()
            .scenes
            .iter()
            .find(|s| s.id == scene)
            .unwrap()
            .items[0]
            .clone();
        let result = save(
            &mut store,
            &f.0,
            &scene,
            &item,
            &asset.id,
            "edited\n🙂公式 x²",
        )
        .unwrap();
        let current = &result.scenes.iter().find(|s| s.id == scene).unwrap().items[0];
        assert_ne!(current.asset.id, asset.id);
        assert_eq!(
            std::fs::read_to_string(&current.asset.path).unwrap(),
            "edited\n🙂公式 x²"
        );
        assert_eq!(
            std::fs::read_to_string(&asset.path).unwrap(),
            "original\r\n中文"
        );
        assert_eq!(
            result.scenes.iter().find(|s| s.id == parent).unwrap().items[0].asset,
            asset
        );
        assert_eq!(current.state, prior.state);
        assert_eq!(
            (current.x, current.y, current.width, current.height),
            (prior.x, prior.y, prior.width, prior.height)
        );
        assert!(store
            .storage_assets()
            .iter()
            .any(|a| a.id == current.asset.id));
        assert!(save(&mut store, &f.0, &scene, &item, &asset.id, "stale").is_err());
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 2);
    }
    #[test]
    fn failed_inactive_or_oversized_text_edit_keeps_draft_source_and_has_no_orphan_file() {
        let f = Fixture::new();
        let (mut store, scene, item, asset, _) = board(&f);
        assert!(save(
            &mut store,
            &f.0,
            &scene,
            &item,
            &asset.id,
            &"a".repeat(crate::assets::MAX_TEXT + 1)
        )
        .is_err());
        store
            .apply(mewu_core::SceneCommand::CloseScene {
                scene_id: scene.clone(),
            })
            .unwrap();
        let before = store.snapshot();
        assert!(save(&mut store, &f.0, &scene, &item, &asset.id, "rejected").is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_to_string(&asset.path).unwrap(),
            "original\r\n中文"
        );
    }
}
#[tauri::command]
pub async fn save_blackboard_text(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    item_id: String,
    expected_asset_id: String,
    text: String,
) -> Result<HostSnapshot, String> {
    if window.label() != "space" {
        return Err("此窗口不能编辑黑板文本".into());
    }
    let host = app.state::<Host>();
    host.exit.ensure_running()?;
    let permit = host.storage.maintenance()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let host = app.state::<Host>();
        host.exit.ensure_running()?;
        let mut engine = host.lock()?;
        host.exit.ensure_running()?;
        let snapshot = save(
            &mut engine.store,
            &host.assets,
            &scene_id,
            &item_id,
            &expected_asset_id,
            &text,
        )?;
        drop(engine);
        Ok(crate::publish(&app, &snapshot))
    })
    .await
    .map_err(|_| "文本保存中断".to_string())?
}
