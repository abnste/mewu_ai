// SPDX-License-Identifier: MPL-2.0
//! Online community index and executable packages. Index data never executes.
use futures_util::StreamExt;
use mewu_extension::{ExtensionManifest, ExtensionPackage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    time::Duration,
};

pub const DEFAULT_CATALOG: &str =
    "https://raw.githubusercontent.com/abnste/mewu-plugins/main/mewu-market.json";
pub const MAX_ARCHIVE: usize = 16 * 1024 * 1024;
pub const MAX_CATALOG: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<mewu_extension::ExtensionIcon>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub repository: String,
    pub version: String,
    pub release_tag: String,
    pub asset_name: String,
    pub sha256: String,
    pub size: u64,
    pub host_api: u32,
    pub platforms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub plugins: Vec<Entry>,
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
pub fn validate_entry(entry: &Entry) -> Result<(), String> {
    if entry.icon.as_ref().is_some_and(|icon| !mewu_extension::valid_extension_icon(icon)) {
        return Err("插件图标无效".into());
    }
    let pieces: Vec<_> = entry.repository.split('/').collect();
    if pieces.len() != 2
        || !pieces.iter().all(|p| segment(p))
        || entry.id.to_ascii_lowercase().starts_with("mewu.")
        || entry.id.len() > 128
        || !segment(&entry.id)
        || entry.name.trim().is_empty()
        || entry.name.len() > 128
        || entry.description.len() > 4000
        || entry.name.chars().any(char::is_control)
        || entry.description.contains('\0')
        || semver::Version::parse(&entry.version).is_err()
        || entry.host_api != 2
        || !segment(&entry.release_tag)
        || !segment(&entry.asset_name)
        || !entry.asset_name.ends_with(".zip")
        || entry.sha256.len() != 64
        || !entry
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || entry.size == 0
        || entry.size > MAX_ARCHIVE as u64
        || !mewu_extension::valid_plugin_platforms(&entry.platforms)
        || entry.tags.len() > 64
        || entry.tags.iter().any(|s| !segment(s))
        || entry.tags.iter().collect::<BTreeSet<_>>().len() != entry.tags.len()
        || entry
            .manifest_path
            .as_deref()
            .is_some_and(|p| p != "mewu-plugin.json")
    {
        return Err("社区目录条目格式或来源无效".into());
    }
    Ok(())
}
pub fn parse_entry(bytes: &[u8]) -> Result<Entry, String> {
    if bytes.len() > mewu_extension::MAX_MANIFEST_BYTES {
        return Err("社区目录条目过大".into());
    }
    let entry: Entry =
        serde_json::from_slice(bytes).map_err(|_| "社区条目 JSON 无效或含重复字段")?;
    validate_entry(&entry)?;
    Ok(entry)
}
pub fn parse_catalog(bytes: &[u8]) -> Result<Catalog, String> {
    if bytes.len() > MAX_CATALOG {
        return Err("社区目录过大".into());
    }
    let catalog: Catalog =
        serde_json::from_slice(bytes).map_err(|_| "社区目录 JSON 无效或含重复字段")?;
    if catalog.schema_version != 2 || catalog.plugins.len() > 2048 {
        return Err("社区目录版本或数量无效".into());
    }
    let mut ids = BTreeSet::new();
    for entry in &catalog.plugins {
        validate_entry(entry)?;
        if !ids.insert(&entry.id) {
            return Err("社区插件标识重复".into());
        }
    }
    Ok(catalog)
}
pub fn parse_manifest(bytes: &[u8]) -> Result<ExtensionManifest, String> {
    if bytes.len() > mewu_extension::MAX_MANIFEST_BYTES {
        return Err("插件清单过大".into());
    }
    let manifest: ExtensionManifest =
        serde_json::from_slice(bytes).map_err(|_| "插件清单 JSON 无效或含重复字段")?;
    // Run the same runtime contract, using a harmless entry placeholder for pure metadata validation.
    let modules = BTreeMap::from([(manifest.entry.clone(), String::new())]);
    mewu_extension::validate_package(&manifest, &modules).map_err(|e| e.to_string())?;
    if manifest.id.to_ascii_lowercase().starts_with("mewu.") {
        return Err("社区插件不能替换宿主内置插件".into());
    }
    Ok(manifest)
}
pub fn decode_package(bytes: &[u8]) -> Result<ExtensionPackage, String> {
    if bytes.is_empty() || bytes.len() > MAX_ARCHIVE {
        return Err("插件安装包过大或为空".into());
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "插件 ZIP 无效")?;
    if zip.len() > mewu_extension::MAX_MODULES + 1 {
        return Err("插件文件过多".into());
    }
    let mut names = BTreeSet::new();
    let mut modules = BTreeMap::new();
    let mut manifest = None;
    let mut total = 0usize;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|_| "无法读取插件文件")?;
        let name = file.name().to_owned();
        if file.is_dir()
            || file.enclosed_name().is_none()
            || name.contains(['\\', ':', '\0'])
            || name.starts_with('/')
            || name
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || !names.insert(name.to_ascii_lowercase())
            || file
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            return Err("插件包含重复、链接或不安全路径".into());
        }
        let limit = if name == "mewu-plugin.json" {
            mewu_extension::MAX_MANIFEST_BYTES
        } else if name.ends_with(".js") || name.ends_with(".mjs") {
            mewu_extension::MAX_MODULE_BYTES
        } else {
            return Err("可执行插件包只能包含清单和 JavaScript 模块".into());
        };
        if file.size() > limit as u64 {
            return Err("插件文件过大".into());
        }
        let mut raw = Vec::new();
        file.by_ref()
            .take(limit as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "插件解压失败")?;
        total = total.checked_add(raw.len()).ok_or("插件过大")?;
        if raw.len() > limit
            || total > mewu_extension::MAX_PACKAGE_BYTES + mewu_extension::MAX_MANIFEST_BYTES
        {
            return Err("插件展开内容过大".into());
        }
        if name == "mewu-plugin.json" {
            manifest = Some(parse_manifest(&raw)?)
        } else {
            modules.insert(
                name,
                String::from_utf8(raw).map_err(|_| "插件模块不是 UTF-8")?,
            );
        }
    }
    let manifest = manifest.ok_or("插件缺少根目录 mewu-plugin.json")?;
    mewu_extension::validate_package(&manifest, &modules).map_err(|e| e.to_string())?;
    let package=ExtensionPackage { manifest, modules };
    mewu_extension::validate_module_graph(&package).map_err(|e|e.to_string())?;
    Ok(package)
}
pub fn verify_package(entry: &Entry, bytes: &[u8]) -> Result<ExtensionPackage, String> {
    validate_entry(entry)?;
    if bytes.len() as u64 != entry.size || format!("{:x}", Sha256::digest(bytes)) != entry.sha256 {
        return Err("插件安装包与已发布的大小或 SHA-256 不符".into());
    }
    let package = decode_package(bytes)?;
    if package.manifest.id != entry.id
        || package.manifest.version != entry.version
        || package.manifest.host_api != entry.host_api
    {
        return Err("插件清单与社区条目身份不一致".into());
    }
    if package.manifest.icon.as_ref().is_some_and(|icon| entry.icon.as_ref().is_some_and(|advertised| advertised != icon)) {
        return Err("目录图标与插件声明不一致".into());
    }
    if mewu_extension::PLUGIN_PLATFORMS.into_iter().any(|platform|
        mewu_extension::plugin_platform_supported(Some(&entry.platforms), platform)
            && !mewu_extension::plugin_platform_supported(package.manifest.platforms.as_deref(), platform)) {
        return Err("社区条目的平台范围超出插件声明".into());
    }
    Ok(package)
}
fn client() -> Result<reqwest::Client, String> {
    crate::network_policy::apply(reqwest::Client::builder(), None)?
        .user_agent("Mewu/1.0 extension-market-v2")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 5 {
                return attempt.error("too many redirects");
            }
            if attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "raw.githubusercontent.com"
                            | "objects.githubusercontent.com"
                            | "release-assets.githubusercontent.com"
                    )
                )
            {
                attempt.follow()
            } else {
                attempt.error("untrusted redirect")
            }
        }))
        .build()
        .map_err(|_| "无法创建社区下载连接".into())
}
async fn download(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|_| "无法连接 GitHub 插件市场")?;
    if !response.status().is_success() {
        return Err(format!("GitHub 返回 HTTP {}", response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("下载内容过大".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(part) = stream.next().await {
        let part = part.map_err(|_| "插件下载中断")?;
        if bytes.len().saturating_add(part.len()) > limit {
            return Err("下载内容过大".into());
        }
        bytes.extend_from_slice(&part);
    }
    Ok(bytes)
}
pub async fn get_catalog() -> Result<Catalog, String> {
    parse_catalog(&download(DEFAULT_CATALOG, MAX_CATALOG).await?)
}
pub async fn get_package(entry: &Entry) -> Result<(ExtensionPackage, Vec<u8>), String> {
    validate_entry(entry)?;
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        entry.repository, entry.release_tag, entry.asset_name
    );
    let bytes = download(&url, MAX_ARCHIVE).await?;
    let package = verify_package(entry, &bytes)?;
    Ok((package, bytes))
}
pub fn supports_platform(entry: &Entry) -> bool {
    mewu_extension::plugin_platform_supported(Some(&entry.platforms), mewu_extension::current_plugin_platform())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        Entry {
            id: "community.example".into(),
            name: "Example".into(),
            description: "Example".into(),
            icon: None,
            tags: vec![],
            repository: "abnste/mewu-plugins".into(),
            version: "1.0.0".into(),
            release_tag: "example-v1.0.0".into(),
            asset_name: "example.zip".into(),
            sha256: "0".repeat(64),
            size: 1,
            host_api: 2,
            platforms: vec!["any".into()],
            manifest_path: None,
        }
    }
    #[test]
    fn rejects_duplicate_identity_and_unsafe_release_sources() {
        let e = entry();
        validate_entry(&e).unwrap();
        let c = Catalog {
            schema_version: 2,
            plugins: vec![e.clone(), e.clone()],
        };
        assert!(parse_catalog(&serde_json::to_vec(&c).unwrap()).is_err());
        for field in ["../x", "x/y", "x%2Fy"] {
            let mut e = e.clone();
            e.release_tag = field.into();
            assert!(validate_entry(&e).is_err());
        }
    }
    #[test]
    fn rejects_archive_mismatch_before_loading_code() {
        let e = entry();
        assert!(verify_package(&e, b"x").is_err());
    }
    #[test]
    fn rejects_duplicate_json_fields() {
        assert!(parse_catalog(br#"{"schemaVersion":2,"schemaVersion":2,"plugins":[]}"#).is_err());
    }
}
