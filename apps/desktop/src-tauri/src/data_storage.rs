// SPDX-License-Identifier: MPL-2.0
//! App-owned direct asset files only. Identity checks use handles, never a
//! renderer-supplied path. Recent outputs are protected while writers settle.
use crate::asset_locations as locations;
use mewu_core::Asset;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const SETTLE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    Files,
    Screenshots,
    Conversations,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataBucket {
    pub category: DataCategory,
    pub count: u64,
    pub bytes: u64,
    pub cleanable_count: u64,
    pub cleanable_bytes: u64,
    pub token: String,
}
#[derive(Clone, Debug, Serialize)]
struct Identity {
    name: String,
    bytes: u64,
    modified_nanos: u128,
    file_key: u64,
}
pub struct Candidate {
    path: PathBuf,
    identity: Identity,
}
pub struct Inventory {
    pub files: DataBucket,
    pub screenshots: DataBucket,
    pub candidates: Vec<(DataCategory, Candidate)>,
    _parents: Vec<File>,
}
pub fn token(value: &impl Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| "无法核验数据清理范围")?)
    ))
}
fn generated(name: &str) -> bool {
    let (id, rest) = name.split_once('.').unwrap_or((name, ""));
    uuid::Uuid::parse_str(id)
        .is_ok_and(|value| value.get_version_num() == 4 && value.to_string() == id)
        && rest.len() <= 128
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}
fn category(path: &Path) -> DataCategory {
    match path.extension().and_then(|v| v.to_str()) {
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif") => DataCategory::Screenshots,
        _ => DataCategory::Files,
    }
}
fn identity(file: &File, name: String) -> Result<Identity, String> {
    let metadata = file.metadata().map_err(|_| "无法读取文件占用")?;
    if locations::ordinary(&metadata, false).is_err() {
        return Err("素材文件不能是链接或重解析点".into());
    }
    let handle = same_file::Handle::from_file(file.try_clone().map_err(|_| "无法核验素材文件")?)
        .map_err(|_| "无法核验素材文件")?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    handle.hash(&mut hash);
    Ok(Identity {
        name,
        bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .map_err(|_| "无法核验素材时间")?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| "素材时间无效")?
            .as_nanos(),
        file_key: hash.finish(),
    })
}
pub fn inventory(
    root: &Path,
    references: &[Asset],
    now: SystemTime,
    owner: usize,
) -> Result<Inventory, String> {
    let parents = locations::pin_ancestors(root).map_err(|_| "素材目录不能是链接或重解析点")?;
    let mut used = HashSet::new();
    for asset in references {
        // Resolve migration aliases through the registered startup map. Missing
        // references fail closed instead of guessing an old path's filename.
        let path = locations::resolve_path(Path::new(&asset.path), root)?;
        used.insert(path.file_name().ok_or("素材文件名无效")?.to_os_string());
    }
    let empty = |category| DataBucket {
        category,
        count: 0,
        bytes: 0,
        cleanable_count: 0,
        cleanable_bytes: 0,
        token: String::new(),
    };
    let mut result = Inventory {
        files: empty(DataCategory::Files),
        screenshots: empty(DataCategory::Screenshots),
        candidates: Vec::new(),
        _parents: parents,
    };
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|_| "无法统计素材目录")? {
        let entry = entry.map_err(|_| "无法统计素材文件")?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !generated(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| "素材目录已变化，请刷新")?;
        if locations::ordinary(&metadata, false).is_err() {
            continue;
        }
        let file =
            locations::open_regular(&entry.path()).map_err(|_| "素材正在使用，请稍后刷新")?;
        let identity = identity(&file, name)?;
        let category = category(&entry.path());
        let eligible = !used.contains(&entry.file_name())
            && now
                .duration_since(metadata.modified().map_err(|_| "无法核验素材时间")?)
                .is_ok_and(|age| age >= SETTLE_AGE);
        entries.push((category, identity.clone(), eligible));
        let bucket = if category == DataCategory::Screenshots {
            &mut result.screenshots
        } else {
            &mut result.files
        };
        bucket.count += 1;
        bucket.bytes += identity.bytes;
        if eligible {
            bucket.cleanable_count += 1;
            bucket.cleanable_bytes += identity.bytes;
            result.candidates.push((
                category,
                Candidate {
                    path: entry.path(),
                    identity,
                },
            ));
        }
    }
    entries.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    for bucket in [&mut result.files, &mut result.screenshots] {
        bucket.token = token(&(
            owner,
            root,
            bucket.category,
            entries
                .iter()
                .filter(|(category, _, _)| *category == bucket.category)
                .collect::<Vec<_>>(),
        ))?;
    }
    Ok(result)
}
pub fn remove(candidate: &Candidate) -> Result<u64, String> {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .access_mode(0x8000_0000 | 0x0001_0000)
            .share_mode(0)
            .custom_flags(0x0020_0000)
            .open(&candidate.path)
            .map_err(|_| "文件正在使用或不可清理")?
    };
    #[cfg(not(windows))]
    let file = OpenOptions::new()
        .read(true)
        .open(&candidate.path)
        .map_err(|_| "文件不可清理")?;
    if token(&identity(&file, candidate.identity.name.clone())?)? != token(&candidate.identity)? {
        return Err("文件已变化，请刷新".into());
    }
    #[cfg(windows)]
    crate::table_staging::mark_delete(&file).map_err(|_| "文件正在使用或不可清理")?;
    #[cfg(not(windows))]
    fs::remove_file(&candidate.path).map_err(|_| "文件不可清理")?;
    drop(file);
    Ok(candidate.identity.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("mewu-data-cleanup-{}", uuid::Uuid::new_v4()));
            assert!(
                path.is_absolute()
                    && !path
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with("c:\\hermes")
            );
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self, suffix: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(format!("{}.{suffix}", uuid::Uuid::new_v4()));
            fs::write(&path, bytes).unwrap();
            path
        }
        fn old_time(&self) -> SystemTime {
            SystemTime::now() + Duration::from_secs(48 * 60 * 60)
        }
        fn scan(&self, assets: &[Asset]) -> Inventory {
            inventory(&self.0, assets, self.old_time(), 10).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let resolved = self.0.canonicalize().unwrap();
            assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            assert!(self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mewu-data-cleanup-"));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn asset(path: &Path) -> Asset {
        Asset {
            id: uuid::Uuid::new_v4().to_string(),
            name: "used.png".into(),
            kind: mewu_core::AssetKind::Image,
            path: path.to_string_lossy().into_owned(),
            width: Some(1),
            height: Some(1),
            origin_x: None,
            origin_y: None,
            scale_factor: None,
        }
    }
    #[test]
    fn inventory_counts_exact_bytes_and_only_direct_owned_unreferenced_files() {
        let f = Fixture::new();
        let used = f.file("png", b"1234");
        let unused = f.file("board-preview.png", b"123456");
        let document = f.file("pdf", b"abc");
        fs::write(f.0.join("unrelated.png"), b"user owned").unwrap();
        fs::create_dir(f.0.join("nested")).unwrap();
        fs::write(f.0.join("nested/keep.png"), b"keep").unwrap();
        let scan = f.scan(&[asset(&used)]);
        assert_eq!(
            (
                scan.screenshots.count,
                scan.screenshots.bytes,
                scan.screenshots.cleanable_count,
                scan.screenshots.cleanable_bytes
            ),
            (2, 10, 1, 6)
        );
        assert_eq!((scan.files.count, scan.files.bytes), (1, 3));
        for (_, candidate) in &scan.candidates {
            remove(candidate).unwrap();
        }
        assert!(used.exists());
        assert!(!unused.exists() && !document.exists());
        assert!(f.0.join("unrelated.png").exists() && f.0.join("nested/keep.png").exists());
    }
    #[test]
    fn recent_outputs_and_missing_or_outside_references_fail_closed() {
        let f = Fixture::new();
        let path = f.file("png", b"new");
        let scan = inventory(&f.0, &[], SystemTime::now(), 10).unwrap();
        assert_eq!(scan.screenshots.cleanable_count, 0);
        assert_eq!(scan.screenshots.bytes, 3);
        assert!(inventory(&f.0, &[asset(&f.0.join("missing.png"))], f.old_time(), 10).is_err());
        assert!(inventory(
            &f.0,
            &[asset(&f.0.parent().unwrap().join("outside.png"))],
            f.old_time(),
            10
        )
        .is_err());
        assert!(path.exists());
    }
    #[test]
    fn review_token_changes_for_owner_identity_size_or_new_reference() {
        let f = Fixture::new();
        let path = f.file("txt", b"old");
        let a = f.scan(&[]).files.token;
        assert_ne!(
            a,
            inventory(&f.0, &[], f.old_time(), 11).unwrap().files.token
        );
        assert_ne!(a, f.scan(&[asset(&path)]).files.token);
        fs::write(&path, b"changed").unwrap();
        assert_ne!(a, f.scan(&[]).files.token);
    }
    #[test]
    fn replacement_at_reviewed_path_is_never_deleted() {
        let f = Fixture::new();
        let path = f.file("pdf", b"original");
        let scan = f.scan(&[]);
        fs::rename(&path, f.0.join("kept-original")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        assert!(remove(&scan.candidates[0].1).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
    }
    #[test]
    fn a_reader_lease_blocks_deletion_of_media_in_use() {
        let f = Fixture::new();
        let path = f.file("mp4", b"movie");
        let scan = f.scan(&[]);
        let reader = locations::open_regular(&path).unwrap();
        #[cfg(windows)]
        {
            assert!(remove(&scan.candidates[0].1).is_err());
            assert!(path.exists());
        }
        drop(reader);
        assert_eq!(remove(&scan.candidates[0].1).unwrap(), 5);
        assert!(!path.exists());
    }
}
