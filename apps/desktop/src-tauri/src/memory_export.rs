// SPDX-License-Identifier: MPL-2.0
use mewu_core::ExportView;
use serde::{
    ser::{SerializeMap, SerializeSeq},
    Serialize, Serializer,
};
use std::{io::Write, path::Path};

pub fn write_to_path(database: &Path, path: &Path) -> Result<(), String> {
    let view = ExportView::open(database).map_err(|_| "无法读取导出数据")?;
    let parent = path.parent().ok_or("导出位置无效")?;
    let temporary = parent.join(format!(".mewu-export-{}.tmp", uuid::Uuid::new_v4()));
    let file = std::fs::File::create_new(&temporary).map_err(|_| "无法保存导出文件")?;
    let result = (|| {
        let mut writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &Export(&view))
            .map_err(|_| "无法导出会话和记忆")?;
        drop(view);
        writer.flush().map_err(|_| "无法保存导出文件")?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|_| "无法保存导出文件")?;
        drop(writer);
        std::fs::rename(&temporary, path).map_err(|_| "无法替换导出文件")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Explicit user export streams memory pages. Normal window snapshots never
/// hydrate the complete memory store, even as it grows over many conversations.
pub struct Export<'a>(&'a ExportView);
impl Serialize for Export<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(6))?;
        let snapshot = self.0.snapshot();
        map.serialize_entry("schemaVersion", &snapshot.schema_version)?;
        map.serialize_entry("agents", &snapshot.agents)?;
        map.serialize_entry("scenes", &snapshot.scenes)?;
        map.serialize_entry("mcpServers", &snapshot.mcp_servers)?;
        map.serialize_entry("memories", &Entries(self.0))?;
        map.serialize_entry("externalMemories", &ExternalEntries(self.0))?;
        map.end()
    }
}

struct ExternalEntries<'a>(&'a ExportView);
impl Serialize for ExternalEntries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        for agent in &self.0.snapshot().agents {
            let mut binding_cursor = None;
            loop {
                let bindings = self
                    .0
                    .memory_provider_status(&agent.id, binding_cursor.as_deref(), 25)
                    .map_err(|_| serde::ser::Error::custom("无法导出记忆来源"))?;
                for binding in bindings.bindings {
                    let mut cursor = None;
                    loop {
                        let page = self
                            .0
                            .external_memory_page(&agent.id, &binding.id, "", cursor.as_deref(), 25)
                            .map_err(|_| serde::ser::Error::custom("无法导出外接记忆"))?;
                        for summary in page.entries {
                            let detail = self
                                .0
                                .external_memory_entry(&agent.id, &binding.id, &summary.id)
                                .map_err(|_| serde::ser::Error::custom("无法导出记忆原文"))?
                                .ok_or_else(|| serde::ser::Error::custom("记忆来源缺失"))?;
                            sequence.serialize_element(&serde_json::json!({
                                "agentId": agent.id, "bindingId": binding.id,
                                "providerId": binding.provider_id, "evidence": detail,
                            }))?;
                        }
                        cursor = page.next_cursor;
                        if cursor.is_none() {
                            break;
                        }
                    }
                }
                binding_cursor = bindings.next_cursor;
                if binding_cursor.is_none() {
                    break;
                }
            }
        }
        sequence.end()
    }
}
struct Entries<'a>(&'a ExportView);
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        for agent in &self.0.snapshot().agents {
            let mut cursor = None;
            loop {
                let page = self
                    .0
                    .memory_page(&agent.id, "", cursor.as_deref(), 25)
                    .map_err(|_| serde::ser::Error::custom("无法导出记忆"))?;
                for entry in &page.entries {
                    sequence.serialize_element(entry)?;
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
        sequence.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mewu_core::{SceneCommand, Store};

    #[test]
    fn explicit_export_includes_every_page_without_putting_memories_in_window_snapshots() {
        let root = std::env::temp_dir().join(format!("mewu-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let database = root.join("spaces.db");
        let mut store = Store::open(&database).unwrap();
        let agent_id = store.snapshot().agents[0].id.clone();
        for index in 0..80 {
            store
                .apply(SceneCommand::SaveMemory {
                    agent_id: agent_id.clone(),
                    id: None,
                    text: format!("Memory {index}"),
                    expected_revision: None,
                })
                .unwrap();
        }
        let snapshot = store.snapshot();
        assert!(snapshot.memories.is_empty());
        let target = root.join("export.json");
        std::fs::write(&target, "previous export").unwrap();
        write_to_path(&database, &target).unwrap();
        let value: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(&target).unwrap()).unwrap();
        assert_eq!(value["memories"].as_array().unwrap().len(), 80);
        assert!(value.get("connection").is_none());
        drop(store);
        for name in ["export.json", "spaces.db", "spaces.db-wal", "spaces.db-shm"] {
            let _ = std::fs::remove_file(root.join(name));
        }
        std::fs::remove_dir(root).unwrap();
    }
}
