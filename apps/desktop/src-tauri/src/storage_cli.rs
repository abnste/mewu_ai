// SPDX-License-Identifier: MPL-2.0
//! Read-only updater query, executed before single-instance/Tauri initialization.
use crate::storage_root::{self, StoragePaths};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn paths(identifier: &str) -> Result<StoragePaths, String> {
    if identifier.is_empty() || identifier.contains(['/', '\\']) {
        return Err("应用标识无效".into());
    }
    Ok(StoragePaths {
        bootstrap: dirs::config_dir()
            .ok_or("配置目录不可用")?
            .join(format!("{identifier}-storage")),
        legacy_root: dirs::data_dir()
            .ok_or("用户数据目录不可用")?
            .join(identifier),
        default_root: dirs::home_dir().ok_or("用户目录不可用")?.join(".mewu"),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    root: PathBuf,
    generation: u64,
    phase: &'static str,
    pending_id: Option<String>,
    bootstrap: PathBuf,
}

fn unmoved_candidate(root: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("无法确认数据目录".into()),
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("数据目录不是普通目录".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("数据目录不能是链接".into());
                }
            }
        }
    }
    for entry in fs::read_dir(root).map_err(|_| "无法检查数据目录")? {
        let entry = entry.map_err(|_| "无法检查数据目录")?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".mewu-migrated-")
        {
            return Err("数据目录已迁移，存储定位信息缺失".into());
        }
    }
    match fs::symlink_metadata(root.join("spaces.db")) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("会话数据库不能是链接".into());
                }
            }
            Ok(true)
        }
        Ok(_) => Err("会话数据库无效".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("无法检查会话数据库".into()),
    }
}

fn info(paths: &StoragePaths) -> Result<Info, String> {
    if let Some(state) = storage_root::readonly_state(paths).map_err(|error| error.to_string())? {
        let phase = match state.phase {
            storage_root::StoragePhase::Idle => "idle",
            storage_root::StoragePhase::Committed => "committed",
            storage_root::StoragePhase::RolledBack => "rolled_back",
            storage_root::StoragePhase::Requested => "requested",
            storage_root::StoragePhase::Copying => "copying",
            storage_root::StoragePhase::Verified => "verified",
            storage_root::StoragePhase::Activated => "activated",
            storage_root::StoragePhase::NeedsRecovery => "needs_recovery",
            storage_root::StoragePhase::Failed => "failed",
        };
        return Ok(Info {
            root: state.root,
            generation: state.generation,
            phase,
            pending_id: state.pending_id,
            bootstrap: paths.bootstrap.clone(),
        });
    }
    let legacy = unmoved_candidate(&paths.legacy_root)?;
    let default = unmoved_candidate(&paths.default_root)?;
    if legacy && default {
        return Err("检测到两个数据目录，无法确定当前数据".into());
    }
    Ok(Info {
        root: if legacy {
            paths.legacy_root.clone()
        } else {
            paths.default_root.clone()
        },
        generation: 0,
        phase: "idle",
        pending_id: None,
        bootstrap: paths.bootstrap.clone(),
    })
}

pub fn write_info(paths: &StoragePaths, output: &Path) -> Result<(), String> {
    // Unknown pre-existing output is never overwritten, even on a failed query.
    let bytes = serde_json::to_vec(&info(paths)?).map_err(|_| "无法生成存储信息")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "无法创建存储信息文件")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "无法保存存储信息".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_locator_query_never_creates_business_storage_and_rejects_moved_source() {
        let base = std::env::temp_dir().join(format!("mewu-storage-cli-{}", uuid::Uuid::new_v4()));
        let paths = StoragePaths {
            bootstrap: base.join("bootstrap"),
            legacy_root: base.join("old"),
            default_root: base.join("new"),
        };
        assert_eq!(info(&paths).unwrap().root, paths.default_root);
        assert!(!base.exists());
        fs::create_dir_all(&paths.legacy_root).unwrap();
        fs::write(paths.legacy_root.join("spaces.db"), b"synthetic").unwrap();
        assert_eq!(info(&paths).unwrap().root, paths.legacy_root);
        fs::write(
            paths.legacy_root.join(".mewu-migrated-synthetic.json"),
            b"{}",
        )
        .unwrap();
        assert!(info(&paths).is_err());
        assert!(!paths.bootstrap.exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn paths_match_tauri_configuration_and_output_is_create_new() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(config["app"]["appDirectoriesOverride"].is_null());
        let identifier = config["identifier"].as_str().unwrap();
        let actual = paths(identifier).unwrap();
        assert_eq!(
            actual.legacy_root,
            dirs::data_dir().unwrap().join(identifier)
        );
        let base =
            std::env::temp_dir().join(format!("mewu-storage-output-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let paths = StoragePaths {
            bootstrap: base.join("b"),
            legacy_root: base.join("l"),
            default_root: base.join("d"),
        };
        let output = base.join("info.json");
        write_info(&paths, &output).unwrap();
        let before = fs::read(&output).unwrap();
        assert!(write_info(&paths, &output).is_err());
        assert_eq!(fs::read(&output).unwrap(), before);
        fs::remove_dir_all(base).unwrap();
    }
}
