// SPDX-License-Identifier: MPL-2.0
//! Executable plugin registry, independent of native feature records and scenes.
use crate::extension_market::{self, Entry};
use mewu_extension::{ExtensionManifest, ExtensionPackage, ServiceRegistration};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Source {
    Local { path: String },
    Github { entry: Entry },
}
impl Source {
    fn same_origin(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local { path: a }, Self::Local { path: b }) => a == b,
            (Self::Github { entry: a }, Self::Github { entry: b }) => {
                a.repository.eq_ignore_ascii_case(&b.repository)
            }
            _ => false,
        }
    }
    fn verify(&self, bytes: &[u8]) -> Result<ExtensionPackage, String> {
        match self {
            Self::Local { .. } => extension_market::decode_package(bytes),
            Self::Github { entry } => extension_market::verify_package(entry, bytes),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub manifest: ExtensionManifest,
    pub revision: u64,
    pub state: String,
    pub source: Source,
    pub sha256: String,
    pub services: Vec<ServiceRegistration>,
    pub has_rollback: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_error: Option<String>,
    pub unsupported_services: BTreeMap<String, String>,
}
pub(crate) fn region_platform_supported(platform: &str) -> bool {
    matches!(platform, "windows-x86_64" | "darwin-aarch64" | "linux-x86_64")
}
pub(crate) fn service_platform_error(service: &ServiceRegistration, platform: &str) -> Option<&'static str> {
    let Ok(platforms) = mewu_extension::service_platforms(service) else { return Some("插件平台声明无效") };
    if !mewu_extension::plugin_platform_supported(platforms.as_deref(), platform) {
        return Some("此功能不支持当前系统");
    }
    if service.metadata.as_ref().and_then(|m| m.get("requiresRegion")).and_then(Value::as_bool) == Some(true)
        && !region_platform_supported(platform) {
        return Some("当前系统暂不支持区域操作");
    }
    None
}
impl Record {
    pub(crate) fn ensure_supported(&self) -> Result<(), String> {
        ensure_platform(&self.manifest, &self.source, mewu_extension::current_plugin_platform())
    }
}
fn ensure_platform(manifest: &ExtensionManifest, source: &Source, platform: &str) -> Result<(), String> {
    let catalog = match source { Source::Github { entry } => Some(entry.platforms.as_slice()), _ => None };
    if !mewu_extension::plugin_platform_supported(manifest.platforms.as_deref(), platform)
        || !mewu_extension::plugin_platform_supported(catalog, platform) {
        return Err("此插件不支持当前系统".into());
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub agent_id: String,
    pub kind: String,
    pub plugin_id: Option<String>,
    pub service_id: Option<String>,
    pub revision: u64,
}
pub struct Store {
    db: Connection,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        let mut db =
            Connection::open(root.join("extensions.db")).map_err(|_| "无法打开扩展注册表")?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|_| "无法配置扩展注册表")?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| "扩展注册表版本无效")?;
        if !(0..=2).contains(&version) {
            return Err("扩展注册表由更新版本创建".into());
        }
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "无法迁移扩展注册表")?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS extensions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,state TEXT NOT NULL,manifest TEXT NOT NULL,source TEXT NOT NULL,sha256 TEXT NOT NULL,archive BLOB NOT NULL,services TEXT NOT NULL DEFAULT '[]',previous_archive BLOB);
          CREATE TABLE IF NOT EXISTS extension_values(plugin_id TEXT NOT NULL,scope TEXT NOT NULL,key TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(plugin_id,scope,key));
          CREATE TABLE IF NOT EXISTS extension_bindings(agent_id TEXT NOT NULL,kind TEXT NOT NULL,plugin_id TEXT NOT NULL,service_id TEXT NOT NULL,PRIMARY KEY(agent_id,kind));").map_err(|_| "无法初始化扩展注册表")?;
        if version < 2 {
            tx.execute_batch(
                "ALTER TABLE extensions ADD COLUMN previous_source TEXT;
                ALTER TABLE extensions ADD COLUMN previous_sha256 TEXT;
                ALTER TABLE extension_bindings ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
                PRAGMA user_version=2;",
            )
            .map_err(|_| "无法迁移扩展注册表")?;
        }
        tx.commit().map_err(|_| "无法提交扩展注册表迁移")?;
        Ok(Self { db })
    }
    fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
        fn parse<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
            serde_json::from_str(&text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        }
        let revision: i64 = row.get(1)?;
        let state: String = row.get(2)?;
        if revision <= 0 || !["enabled", "disabled", "removed"].contains(&state.as_str()) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut record = Record {
            manifest: parse(row.get(0)?)?,
            revision: revision as u64,
            state,
            source: parse(row.get(3)?)?,
            sha256: row.get(4)?,
            services: parse(row.get(5)?)?,
            has_rollback: row.get(6)?,
            platform_error: None,
            unsupported_services: BTreeMap::new(),
        };
        record.platform_error = record.ensure_supported().err();
        for service in &record.services {
            if let Some(reason) = service_platform_error(service, mewu_extension::current_plugin_platform()) {
                record.unsupported_services.insert(service.id.clone(), reason.into());
            }
        }
        Ok(record)
    }
    pub fn list(&self) -> Result<Vec<Record>, String> {
        let mut query = self.db.prepare("SELECT manifest,revision,state,source,sha256,services,previous_archive IS NOT NULL AND previous_source IS NOT NULL AND previous_sha256 IS NOT NULL FROM extensions ORDER BY id").map_err(|_| "无法读取扩展")?;
        let result = query
            .query_map([], Self::row)
            .map_err(|_| "无法读取扩展")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "扩展记录损坏".into());
        result
    }
    pub fn get_optional(&self, id: &str) -> Result<Option<Record>, String> {
        self.db.query_row("SELECT manifest,revision,state,source,sha256,services,previous_archive IS NOT NULL AND previous_source IS NOT NULL AND previous_sha256 IS NOT NULL FROM extensions WHERE id=?1", [id], Self::row).optional().map_err(|_| "扩展记录损坏".into())
    }
    pub fn get(&self, id: &str) -> Result<Record, String> {
        self.get_optional(id)?.ok_or("扩展未安装".into())
    }
    pub fn package(&self, id: &str, revision: u64) -> Result<ExtensionPackage, String> {
        let record = self.get(id)?;
        record.ensure_supported()?;
        if record.revision != revision || record.state != "enabled" {
            return Err("扩展已停用或版本已变化".into());
        }
        let bytes: Vec<u8> = self
            .db
            .query_row("SELECT archive FROM extensions WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(|_| "无法读取扩展安装包")?;
        if format!("{:x}", Sha256::digest(&bytes)) != record.sha256 {
            return Err("扩展安装包完整性验证失败".into());
        }
        let package = record.source.verify(&bytes)?;
        if package.manifest != record.manifest {
            return Err("扩展清单与安装包不一致".into());
        }
        Ok(package)
    }
    pub fn validate_dependencies(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if !mewu_extension::plugin_platform_supported(manifest.platforms.as_deref(), mewu_extension::current_plugin_platform()) {
            return Err("此插件不支持当前系统".into());
        }
        for (id, requirement) in &manifest.dependencies {
            let dependency = self.get(id)?;
            dependency.ensure_supported()?;
            let requirement =
                semver::VersionReq::parse(requirement).map_err(|_| "依赖版本规范无效")?;
            let version = semver::Version::parse(&dependency.manifest.version)
                .map_err(|_| "已安装依赖版本损坏")?;
            if dependency.state != "enabled" || !requirement.matches(&version) {
                return Err(format!("插件依赖不可用：{id}"));
            }
        }
        Ok(())
    }
    pub fn install(
        &mut self,
        package: &ExtensionPackage,
        bytes: &[u8],
        source: &Source,
        expected: Option<u64>,
    ) -> Result<Record, String> {
        self.install_internal(package, bytes, source, expected, false)
    }
    fn install_internal(
        &mut self,
        package: &ExtensionPackage,
        bytes: &[u8],
        source: &Source,
        expected: Option<u64>,
        rollback: bool,
    ) -> Result<Record, String> {
        if source.verify(bytes)? != *package {
            return Err("预览内容与安装包不一致".into());
        }
        ensure_platform(&package.manifest, source, mewu_extension::current_plugin_platform())?;
        let old = self.get_optional(&package.manifest.id)?;
        if old.as_ref().map(|r| r.revision) != expected {
            return Err("扩展安装状态已变化，请重新预览".into());
        }
        let digest = format!("{:x}", Sha256::digest(bytes));
        if let Some(old) = &old {
            // Removal cannot transfer a stable identity or its retained data.
            if !old.source.same_origin(source) {
                return Err("插件标识已绑定原仓库，更换来源请使用不同标识".into());
            }
            let previous =
                semver::Version::parse(&old.manifest.version).map_err(|_| "已安装版本损坏")?;
            let next =
                semver::Version::parse(&package.manifest.version).map_err(|_| "插件版本无效")?;
            if next == previous && old.sha256 != digest {
                return Err("同一版本的插件内容不能改变".into());
            }
            if next < previous && !rollback {
                return Err("请使用回滚操作安装旧版本".into());
            }
            if next == previous && old.sha256 == digest && old.state != "removed" {
                return Ok(old.clone());
            }
        }
        let revision = old
            .as_ref()
            .map_or(Some(1), |r| r.revision.checked_add(1))
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or("扩展版本计数溢出")?;
        let state = old
            .as_ref()
            .filter(|r| r.state != "removed")
            .map_or("enabled", |r| r.state.as_str());
        self.db.execute("INSERT INTO extensions(id,revision,state,manifest,source,sha256,archive,services,previous_archive,previous_source,previous_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,'[]',NULL,NULL,NULL)
          ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,state=excluded.state,manifest=excluded.manifest,source=excluded.source,sha256=excluded.sha256,previous_archive=extensions.archive,previous_source=extensions.source,previous_sha256=extensions.sha256,archive=excluded.archive,services='[]'", params![package.manifest.id, revision as i64, state, serde_json::to_string(&package.manifest).map_err(|_| "清单无效")?, serde_json::to_string(source).map_err(|_| "来源无效")?, digest, bytes]).map_err(|_| "无法保存扩展安装")?;
        self.get(&package.manifest.id)
    }
    pub fn set_state(&mut self, id: &str, revision: u64, state: &str) -> Result<Record, String> {
        if !["enabled", "disabled", "removed"].contains(&state) {
            return Err("扩展状态无效".into());
        }
        let old = self.get(id)?;
        if state == "enabled" { old.ensure_supported()?; }
        if old.revision != revision {
            return Err("扩展版本已变化".into());
        }
        if old.state == "removed" && state != "removed" {
            return Err("插件已卸载，请重新安装".into());
        }
        if old.state == state {
            return Ok(old);
        }
        if revision >= i64::MAX as u64 {
            return Err("扩展版本计数溢出".into());
        }
        if self
            .db
            .execute(
                "UPDATE extensions SET state=?3,revision=revision+1 WHERE id=?1 AND revision=?2",
                params![id, revision as i64, state],
            )
            .map_err(|_| "无法保存扩展状态")?
            != 1
        {
            return Err("扩展版本已变化".into());
        }
        self.get(id)
    }
    pub fn set_services(
        &mut self,
        id: &str,
        revision: u64,
        services: &[ServiceRegistration],
    ) -> Result<(), String> {
        if revision > i64::MAX as u64 || services.len() > mewu_extension::MAX_SERVICES {
            return Err("扩展服务目录无效".into());
        }
        if self
            .db
            .execute(
                "UPDATE extensions SET services=?3 WHERE id=?1 AND revision=?2 AND state='enabled'",
                params![
                    id,
                    revision as i64,
                    serde_json::to_string(services).map_err(|_| "服务目录无效")?
                ],
            )
            .map_err(|_| "无法保存扩展服务")?
            != 1
        {
            return Err("扩展已停用".into());
        }
        Ok(())
    }

    /// Compensate exactly the failed activation attempt. A newer installation,
    /// explicit disable/remove, or another successful state transition wins.
    /// Keep current/rollback archives and provider data available for recovery.
    pub(crate) fn fail_activation(
        &mut self,
        id: &str,
        revision: u64,
    ) -> Result<Option<Record>, String> {
        if revision == 0 || revision > i64::MAX as u64 {
            return Err("扩展版本无效".into());
        }
        let changed = self.db.execute(
            "UPDATE extensions SET state='disabled', services='[]', revision=CASE WHEN revision<9223372036854775807 THEN revision+1 ELSE revision END WHERE id=?1 AND revision=?2 AND state='enabled'",
            params![id, revision as i64],
        ).map_err(|_| "无法保存扩展激活失败后的停用状态")?;
        if changed == 0 {
            return Ok(None);
        }
        self.get(id).map(Some)
    }
    pub fn rollback(&mut self, id: &str, revision: u64) -> Result<Record, String> {
        let old = self.get(id)?;
        if old.revision != revision || old.state == "removed" {
            return Err("扩展版本已变化或已卸载".into());
        }
        let (bytes, source, digest): (Vec<u8>, String, String) = self.db.query_row("SELECT previous_archive,previous_source,previous_sha256 FROM extensions WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|_| "没有可验证的上一版本")?;
        if digest != format!("{:x}", Sha256::digest(&bytes)) {
            return Err("回滚包完整性验证失败".into());
        }
        let source: Source = serde_json::from_str(&source).map_err(|_| "上一版本来源无效")?;
        let package = source.verify(&bytes)?;
        if package.manifest.id != id {
            return Err("回滚包身份不一致".into());
        }
        self.install_internal(&package, &bytes, &source, Some(revision), true)
    }
    pub fn value(&self, plugin: &str, scope: &str, key: &str) -> Result<Value, String> {
        validate_key(key, false)?;
        let text: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM extension_values WHERE plugin_id=?1 AND scope=?2 AND key=?3",
                params![plugin, scope, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| "无法读取插件数据")?;
        text.map_or(Ok(Value::Null), |s| {
            serde_json::from_str(&s).map_err(|_| "插件数据无效".into())
        })
    }
    pub fn set_value(
        &mut self,
        plugin: &str,
        scope: &str,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        validate_key(key, false)?;
        let text = serde_json::to_string(value).map_err(|_| "插件数据无效")?;
        if text.len() > 1024 * 1024 {
            return Err("单项插件数据超过 1 MiB".into());
        }
        self.db.execute("INSERT INTO extension_values VALUES(?1,?2,?3,?4) ON CONFLICT(plugin_id,scope,key) DO UPDATE SET value=excluded.value", params![plugin, scope, key, text]).map_err(|_| "无法保存插件数据")?;
        Ok(())
    }
    pub fn delete_value(&mut self, plugin: &str, scope: &str, key: &str) -> Result<(), String> {
        validate_key(key, false)?;
        self.db
            .execute(
                "DELETE FROM extension_values WHERE plugin_id=?1 AND scope=?2 AND key=?3",
                params![plugin, scope, key],
            )
            .map_err(|_| "无法删除插件数据")?;
        Ok(())
    }
    pub fn compare_and_set(
        &mut self,
        plugin: &str,
        scope: &str,
        key: &str,
        expected: &Value,
        value: &Value,
    ) -> Result<bool, String> {
        validate_key(key, false)?;
        let text = serde_json::to_string(value).map_err(|_| "插件数据无效")?;
        let previous = serde_json::to_string(expected).map_err(|_| "插件数据无效")?;
        if text.len() > 1024 * 1024 || previous.len() > 1024 * 1024 {
            return Err("单项插件数据超过 1 MiB".into());
        }
        let count=if expected.is_null(){self.db.execute("INSERT INTO extension_values(plugin_id,scope,key,value) VALUES(?1,?2,?3,?4) ON CONFLICT(plugin_id,scope,key) DO NOTHING",params![plugin,scope,key,text])}
            else{self.db.execute("UPDATE extension_values SET value=?4 WHERE plugin_id=?1 AND scope=?2 AND key=?3 AND value=?5",params![plugin,scope,key,text,previous])}.map_err(|_|"无法比较并保存插件数据")?;
        Ok(count == 1)
    }
    pub fn keys(
        &self,
        plugin: &str,
        scope: &str,
        prefix: &str,
        cursor: &str,
    ) -> Result<Value, String> {
        validate_key(prefix, true)?;
        validate_key(cursor, true)?;
        // Literal prefix comparison: '%'/'_' never become LIKE operators.
        let mut query = self.db.prepare("SELECT key FROM extension_values WHERE plugin_id=?1 AND scope=?2 AND key>?3 AND substr(key,1,length(?4))=?4 ORDER BY key LIMIT 101").map_err(|_| "无法读取插件数据")?;
        let mut keys = query
            .query_map(params![plugin, scope, cursor, prefix], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| "无法读取插件数据")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "插件数据无效")?;
        let more = keys.len() > 100;
        if more {
            keys.pop();
        }
        Ok(serde_json::json!({"nextCursor":if more {keys.last().cloned()}else{None},"keys":keys}))
    }
    fn binding_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
        let revision: i64 = row.get(4)?;
        if revision <= 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let plugin: String = row.get(2)?;
        let service: String = row.get(3)?;
        if plugin.is_empty() != service.is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Binding {
            agent_id: row.get(0)?,
            kind: row.get(1)?,
            plugin_id: if plugin.is_empty() {
                None
            } else {
                Some(plugin)
            },
            service_id: if service.is_empty() {
                None
            } else {
                Some(service)
            },
            revision: revision as u64,
        })
    }
    pub fn bindings(&self) -> Result<Vec<Binding>, String> {
        let mut query = self.db.prepare("SELECT agent_id,kind,plugin_id,service_id,revision FROM extension_bindings ORDER BY agent_id,kind").map_err(|_| "无法读取扩展绑定")?;
        let result = query
            .query_map([], Self::binding_row)
            .map_err(|_| "无法读取扩展绑定")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "扩展绑定损坏".into());
        result
    }
    pub fn binding(&self, agent: &str, kind: &str) -> Result<Option<Binding>, String> {
        self.db.query_row("SELECT agent_id,kind,plugin_id,service_id,revision FROM extension_bindings WHERE agent_id=?1 AND kind=?2", params![agent,kind],Self::binding_row).optional().map_err(|_| "扩展绑定损坏".into())
    }
    pub fn set_binding(
        &mut self,
        agent: &str,
        kind: &str,
        plugin: Option<&str>,
        service: Option<&str>,
        expected: Option<u64>,
    ) -> Result<(), String> {
        if !["agent", "memory"].contains(&kind) || plugin.is_some() != service.is_some() {
            return Err("扩展绑定无效".into());
        }
        let current = self.binding(agent, kind)?.map_or(0, |b| b.revision);
        if expected.is_some_and(|r| r != current) {
            return Err("扩展绑定已变化，请重新加载".into());
        }
        let next = current
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or("扩展绑定版本溢出")?;
        self.db.execute("INSERT INTO extension_bindings(agent_id,kind,plugin_id,service_id,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(agent_id,kind) DO UPDATE SET plugin_id=excluded.plugin_id,service_id=excluded.service_id,revision=excluded.revision",params![agent,kind,plugin.unwrap_or(""),service.unwrap_or(""),next as i64]).map_err(|_| "无法保存扩展绑定")?;
        Ok(())
    }
}
fn validate_key(key: &str, empty: bool) -> Result<(), String> {
    if (!empty && key.is_empty()) || key.len() > 512 || key.chars().any(char::is_control) {
        return Err("插件数据键无效".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn with_store(test: impl FnOnce(&mut Store, &std::path::Path)) {
        let directory =
            std::env::temp_dir().join(format!("mewu-extension-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let mut store = Store::open(&directory).unwrap();
        test(&mut store, &directory);
        drop(store);
        for name in [
            "extensions.db",
            "extensions.db-wal",
            "extensions.db-shm",
            "extensions.db-journal",
        ] {
            let _ = std::fs::remove_file(directory.join(name));
        }
        let _ = std::fs::remove_dir(directory);
    }
    fn package(version: &str, repository: &str) -> (ExtensionPackage, Vec<u8>, Source) {
        let manifest:ExtensionManifest=serde_json::from_value(serde_json::json!({"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"community.store-demo","version":version,"name":"Store","description":"","license":"MPL-2.0","entry":"index.js"})).unwrap();
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        zip.start_file("mewu-plugin.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        zip.start_file("index.js", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(
            b"export default ctx=>ctx.commands.register({id:'read',title:'Read'},()=>42);",
        )
        .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let package = extension_market::decode_package(&bytes).unwrap();
        let source = Source::Github {
            entry: Entry {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                description: manifest.description.clone(),
                icon: manifest.icon.clone(),
                tags: vec![],
                repository: repository.into(),
                version: version.into(),
                release_tag: format!("v{version}"),
                asset_name: "plugin.zip".into(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
                host_api: 2,
                platforms: vec!["any".into()],
                manifest_path: None,
            },
        };
        (package, bytes, source)
    }
    #[test]
    fn github_rollback_preserves_the_previous_release_identity_and_digest() {
        with_store(|store, _| {
            let (first, bytes1, source1) = package("1.0.0", "example/repo");
            let a = store.install(&first, &bytes1, &source1, None).unwrap();
            let (next, bytes2, source2) = package("2.0.0", "example/repo");
            let b = store
                .install(&next, &bytes2, &source2, Some(a.revision))
                .unwrap();
            assert!(b.has_rollback);
            assert!(store
                .install(&first, &bytes1, &source1, Some(b.revision))
                .is_err());
            let c = store.rollback(&first.manifest.id, b.revision).unwrap();
            assert_eq!(c.manifest.version, "1.0.0");
            let Source::Github { entry } = c.source else {
                panic!()
            };
            assert_eq!(entry.version, "1.0.0");
            assert_eq!(entry.sha256, format!("{:x}", Sha256::digest(&bytes1)));
            assert_eq!(
                store.package(&first.manifest.id, c.revision).unwrap(),
                first
            );
        });
    }

    #[test]
    fn platform_guards_cover_catalog_local_install_enable_run_and_dependency_without_destroying_data() {
        with_store(|store, _| {
            let current = mewu_extension::current_plugin_platform();
            let other = mewu_extension::PLUGIN_PLATFORMS.into_iter().find(|p| *p != current).unwrap();
            let (portable, bytes, source) = package("1.0.0", "example/repo");
            let mut foreign_catalog = source.clone();
            if let Source::Github { entry } = &mut foreign_catalog { entry.platforms = vec![other.into()]; }
            assert!(store.install(&portable, &bytes, &foreign_catalog, None).is_err());
            assert!(store.list().unwrap().is_empty());
            let mut restricted = portable.clone(); restricted.manifest.platforms = Some(vec![other.into()]);
            assert!(ensure_platform(&restricted.manifest, &Source::Local { path:"plugin.zip".into() }, current).is_err());
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            archive.start_file("mewu-plugin.json", zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(&serde_json::to_vec(&restricted.manifest).unwrap()).unwrap();
            for (name, body) in &restricted.modules { archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap(); archive.write_all(body.as_bytes()).unwrap(); }
            let local_bytes = archive.finish().unwrap().into_inner();
            assert!(store.install(&restricted, &local_bytes, &Source::Local { path:"plugin.zip".into() }, None).is_err());
            assert!(store.list().unwrap().is_empty());
            let installed = store.install(&portable, &bytes, &source, None).unwrap();
            store.set_value(&installed.manifest.id, "agent", "remember", &serde_json::json!(42)).unwrap();
            // Model a valid existing installation moved from another OS: same
            // untouched archive and manifest, but its verified catalog restricts it.
            store.db.execute("UPDATE extensions SET source=?2,state='disabled' WHERE id=?1", params![installed.manifest.id, serde_json::to_string(&foreign_catalog).unwrap()]).unwrap();
            let foreign = store.get(&installed.manifest.id).unwrap();
            assert!(foreign.platform_error.is_some());
            assert!(store.set_state(&foreign.manifest.id, foreign.revision, "enabled").is_err());
            assert!(store.package(&foreign.manifest.id, foreign.revision).is_err());
            let mut consumer = portable.manifest.clone(); consumer.id="independent.consumer".into();
            consumer.dependencies.insert(foreign.manifest.id.clone(), "^1".into());
            assert!(store.validate_dependencies(&consumer).is_err());
            assert_eq!(store.get(&foreign.manifest.id).unwrap().revision, foreign.revision);
            assert_eq!(store.value(&foreign.manifest.id, "agent", "remember").unwrap(), 42);
            let (updated, updated_bytes, updated_source) = package("2.0.0", "example/repo");
            let next = store.install(&updated, &updated_bytes, &updated_source, Some(foreign.revision)).unwrap();
            assert!(store.rollback(&next.manifest.id, next.revision).is_err());
            assert_eq!(store.get(&next.manifest.id).unwrap().revision, next.revision);
            store.set_state(&next.manifest.id, next.revision, "removed").unwrap();
            assert_eq!(store.value(&foreign.manifest.id, "agent", "remember").unwrap(), 42);
        });
    }
    #[test]
    fn unsupported_native_region_service_does_not_disable_portable_ui_agent_or_memory() {
        for kind in ["command", "tool", "agent", "memory", "ui", "renderer"] {
            let mut service: ServiceRegistration = serde_json::from_value(serde_json::json!({"kind":kind,"id":"independent","title":"Independent"})).unwrap();
            for platform in mewu_extension::PLUGIN_PLATFORMS { assert!(service_platform_error(&service, platform).is_none()); }
            service.metadata = Some(serde_json::json!({"requiresRegion":true}));
            assert!(service_platform_error(&service, "windows-x86_64").is_none());
            assert!(service_platform_error(&service, "darwin-aarch64").is_none());
            assert!(service_platform_error(&service, "linux-x86_64").is_none());
            assert!(!region_platform_supported("linux-aarch64"));
            assert_eq!(service_platform_error(&service, "linux-aarch64"), Some("此功能不支持当前系统"));
            service.metadata = Some(serde_json::json!({"platforms":["darwin-aarch64"]}));
            assert!(service_platform_error(&service, "darwin-aarch64").is_none());
            assert!(service_platform_error(&service, "windows-x86_64").is_some());
        }
    }
    #[test]
    fn removed_identity_cannot_be_taken_over_and_disabled_updates_stay_disabled() {
        with_store(|store, _| {
            let (first, bytes1, source1) = package("1.0.0", "example/repo");
            let a = store.install(&first, &bytes1, &source1, None).unwrap();
            let a = store
                .set_state(&a.manifest.id, a.revision, "disabled")
                .unwrap();
            let (next, bytes2, source2) = package("2.0.0", "example/repo");
            let b = store
                .install(&next, &bytes2, &source2, Some(a.revision))
                .unwrap();
            assert_eq!(b.state, "disabled");
            let b = store
                .set_state(&b.manifest.id, b.revision, "removed")
                .unwrap();
            assert!(store
                .set_state(&b.manifest.id, b.revision, "enabled")
                .is_err());
            let (other, bytes3, source3) = package("3.0.0", "attacker/repo");
            assert!(store
                .install(&other, &bytes3, &source3, Some(b.revision))
                .is_err());
            assert_eq!(store.get(&b.manifest.id).unwrap().state, "removed");
        });
    }
    #[test]
    fn corrupt_registry_is_not_treated_as_a_new_install() {
        with_store(|store, _| {
            let (first, bytes, source) = package("1.0.0", "example/repo");
            let a = store.install(&first, &bytes, &source, None).unwrap();
            store
                .db
                .execute(
                    "UPDATE extensions SET manifest='invalid JSON' WHERE id=?1",
                    [&a.manifest.id],
                )
                .unwrap();
            assert!(store.get_optional(&a.manifest.id).is_err());
            assert!(store.install(&first, &bytes, &source, None).is_err());
        });
    }
    #[test]
    fn scoped_pagination_retrieves_all_history_and_prefix_is_literal() {
        with_store(|store, _| {
            for index in 0..235 {
                store
                    .set_value(
                        "p",
                        "a",
                        &format!("memory%{index:04}"),
                        &serde_json::json!(index),
                    )
                    .unwrap();
            }
            store
                .set_value("p", "b", "memory%0000", &serde_json::json!("other agent"))
                .unwrap();
            store
                .set_value("p", "a", "memoryX0000", &Value::Null)
                .unwrap();
            let mut cursor = String::new();
            let mut count = 0;
            loop {
                let page = store.keys("p", "a", "memory%", &cursor).unwrap();
                count += page["keys"].as_array().unwrap().len();
                let Some(next) = page["nextCursor"].as_str() else {
                    break;
                };
                cursor = next.into();
            }
            assert_eq!(count, 235);
        });
    }
    #[test]
    fn storage_compare_and_set_distinguishes_absent_from_a_stored_null() {
        with_store(|store, _| {
            assert!(store
                .compare_and_set("p", "a", "sequence", &Value::Null, &serde_json::json!(1))
                .unwrap());
            assert!(!store
                .compare_and_set("p", "a", "sequence", &Value::Null, &serde_json::json!(1))
                .unwrap());
            assert!(store
                .compare_and_set(
                    "p",
                    "a",
                    "sequence",
                    &serde_json::json!(1),
                    &serde_json::json!(2)
                )
                .unwrap());
            assert!(!store
                .compare_and_set(
                    "p",
                    "a",
                    "sequence",
                    &serde_json::json!(1),
                    &serde_json::json!(3)
                )
                .unwrap());
            store
                .set_value("p", "a", "stored-null", &Value::Null)
                .unwrap();
            assert!(!store
                .compare_and_set("p", "a", "stored-null", &Value::Null, &serde_json::json!(1))
                .unwrap());
        });
    }
    #[test]
    fn binding_clear_retains_a_revision_tombstone_and_rejects_stale_updates() {
        with_store(|store, _| {
            store
                .set_binding("agent", "memory", Some("p"), Some("memory"), Some(0))
                .unwrap();
            let binding = store.binding("agent", "memory").unwrap().unwrap();
            assert_eq!(binding.revision, 1);
            store
                .set_binding("agent", "memory", None, None, Some(1))
                .unwrap();
            let binding = store.binding("agent", "memory").unwrap().unwrap();
            assert_eq!(binding.revision, 2);
            assert!(binding.plugin_id.is_none());
            assert!(store
                .set_binding("agent", "memory", Some("other"), Some("memory"), Some(1))
                .is_err());
            assert!(store
                .set_binding("agent", "memory", None, Some("memory"), None)
                .is_err());
        });
    }
    #[test]
    fn two_sqlite_connections_have_exactly_one_absent_compare_and_set_winner() {
        with_store(|_, root| {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let mut handles = vec![];
            for value in [1, 2] {
                let root = root.to_owned();
                let barrier = barrier.clone();
                handles.push(std::thread::spawn(move || {
                    let mut store = Store::open(&root).unwrap();
                    barrier.wait();
                    store
                        .compare_and_set(
                            "plugin",
                            "agent",
                            "sequence",
                            &Value::Null,
                            &serde_json::json!(value),
                        )
                        .unwrap()
                }));
            }
            assert_eq!(
                handles
                    .into_iter()
                    .map(|h| usize::from(h.join().unwrap()))
                    .sum::<usize>(),
                1
            );
        });
    }
    #[test]
    fn dependencies_require_an_enabled_compatible_installed_version() {
        with_store(|store, _| {
            let (dependency, bytes, source) = package("1.0.0", "example/repo");
            let installed = store.install(&dependency, &bytes, &source, None).unwrap();
            let mut caller = dependency.manifest.clone();
            caller.id = "community.caller".into();
            caller
                .dependencies
                .insert(dependency.manifest.id.clone(), "^1.0".into());
            store.validate_dependencies(&caller).unwrap();
            let disabled = store
                .set_state(&installed.manifest.id, installed.revision, "disabled")
                .unwrap();
            assert!(store.validate_dependencies(&caller).is_err());
            let enabled = store
                .set_state(&installed.manifest.id, disabled.revision, "enabled")
                .unwrap();
            let (next, bytes, source) = package("2.0.0", "example/repo");
            store
                .install(&next, &bytes, &source, Some(enabled.revision))
                .unwrap();
            assert!(store.validate_dependencies(&caller).is_err());
        });
    }
    #[test]
    fn installed_archive_tampering_is_rejected_before_activation() {
        with_store(|store, _| {
            let (package, bytes, source) = package("1.0.0", "example/repo");
            let installed = store.install(&package, &bytes, &source, None).unwrap();
            store
                .db
                .execute(
                    "UPDATE extensions SET archive=x'0001' WHERE id=?1",
                    [&installed.manifest.id],
                )
                .unwrap();
            assert!(store
                .package(&installed.manifest.id, installed.revision)
                .is_err());
        });
    }

    #[test]
    fn failed_activation_disables_and_clears_services_but_preserves_recoverable_package_and_data() {
        with_store(|store, _| {
            let (package, bytes, source) = package("1.0.0", "example/repo");
            let installed = store.install(&package, &bytes, &source, None).unwrap();
            let services = vec![serde_json::from_value(
                serde_json::json!({"kind":"command","id":"read","title":"Read"}),
            )
            .unwrap()];
            store
                .set_services(&installed.manifest.id, installed.revision, &services)
                .unwrap();
            store
                .set_value(
                    &installed.manifest.id,
                    "agent",
                    "memory",
                    &serde_json::json!({"content":"keep"}),
                )
                .unwrap();
            let failed = store
                .fail_activation(&installed.manifest.id, installed.revision)
                .unwrap()
                .unwrap();
            assert_eq!(failed.state, "disabled");
            assert_eq!(failed.revision, installed.revision + 1);
            assert!(failed.services.is_empty());
            assert_eq!(failed.sha256, installed.sha256);
            assert_eq!(failed.manifest, installed.manifest);
            assert_eq!(
                serde_json::to_value(&failed.source).unwrap(),
                serde_json::to_value(&installed.source).unwrap()
            );
            assert_eq!(
                store
                    .value(&installed.manifest.id, "agent", "memory")
                    .unwrap(),
                serde_json::json!({"content":"keep"})
            );
            assert!(store
                .package(&installed.manifest.id, failed.revision)
                .is_err());
            let enabled = store
                .set_state(&installed.manifest.id, failed.revision, "enabled")
                .unwrap();
            assert_eq!(
                store
                    .package(&installed.manifest.id, enabled.revision)
                    .unwrap(),
                package
            );
        });
    }

    #[test]
    fn failed_updated_activation_retains_verified_previous_release_for_rollback() {
        with_store(|store, _| {
            let (first, bytes1, source1) = package("1.0.0", "example/repo");
            let a = store.install(&first, &bytes1, &source1, None).unwrap();
            let (second, bytes2, source2) = package("2.0.0", "example/repo");
            let b = store
                .install(&second, &bytes2, &source2, Some(a.revision))
                .unwrap();
            let failed = store
                .fail_activation(&b.manifest.id, b.revision)
                .unwrap()
                .unwrap();
            assert!(failed.has_rollback);
            let rolled = store.rollback(&b.manifest.id, failed.revision).unwrap();
            assert_eq!(rolled.state, "disabled");
            assert_eq!(rolled.manifest.version, "1.0.0");
            assert_eq!(rolled.sha256, format!("{:x}", Sha256::digest(&bytes1)));
            assert_eq!(
                serde_json::to_value(&rolled.source).unwrap(),
                serde_json::to_value(&source1).unwrap()
            );
            let enabled = store
                .set_state(&rolled.manifest.id, rolled.revision, "enabled")
                .unwrap();
            assert_eq!(
                store
                    .package(&enabled.manifest.id, enabled.revision)
                    .unwrap(),
                first
            );
        });
    }

    #[test]
    fn stale_activation_failure_never_overwrites_new_install_disable_or_removal() {
        with_store(|store, _| {
            let (first, bytes1, source1) = package("1.0.0", "example/repo");
            let a = store.install(&first, &bytes1, &source1, None).unwrap();
            let (second, bytes2, source2) = package("2.0.0", "example/repo");
            let b = store
                .install(&second, &bytes2, &source2, Some(a.revision))
                .unwrap();
            let before = serde_json::to_value(&b).unwrap();
            assert!(store
                .fail_activation(&a.manifest.id, a.revision)
                .unwrap()
                .is_none());
            assert_eq!(
                serde_json::to_value(store.get(&b.manifest.id).unwrap()).unwrap(),
                before
            );
            let disabled = store
                .set_state(&b.manifest.id, b.revision, "disabled")
                .unwrap();
            assert!(store
                .fail_activation(&b.manifest.id, b.revision)
                .unwrap()
                .is_none());
            assert_eq!(
                store.get(&b.manifest.id).unwrap().revision,
                disabled.revision
            );
            let removed = store
                .set_state(&b.manifest.id, disabled.revision, "removed")
                .unwrap();
            assert!(store
                .fail_activation(&b.manifest.id, b.revision)
                .unwrap()
                .is_none());
            assert_eq!(
                serde_json::to_value(store.get(&b.manifest.id).unwrap()).unwrap(),
                serde_json::to_value(&removed).unwrap()
            );
        });
    }
}
