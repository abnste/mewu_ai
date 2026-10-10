// SPDX-License-Identifier: MPL-2.0
//! Local memory provider: independent records, a rebuildable FTS5 index and
//! bounded query results. The Store owns authorization and the outer transaction.
//! https://sqlite.org/fts5.html
use crate::{
    CoreError, MemoryEntry, MemoryMutation, MemoryOrigin, MemoryPage, MemorySource, MemoryStats,
    Snapshot,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, CoreError>;
const COLUMNS: &str = "m.id,m.agent_id,m.text,m.revision,m.created_at,m.updated_at,m.source_scene_id,m.source_message_id,m.origin";
pub(crate) const RECALL_ITEMS: usize = 8;
const RECALL_CHARS: usize = 6000;
const RECALL_BYTES: usize = 24 * 1024;

fn sql_int(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid("记忆数值超出存储范围"))
}
fn unsigned(row: &Row<'_>, column: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
}

fn invalid(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}

pub(crate) fn query_error(error: CoreError) -> CoreError {
    if matches!(&error, CoreError::Database(rusqlite::Error::SqliteFailure(code, _)) if code.code == rusqlite::ErrorCode::OperationInterrupted)
    {
        CoreError::MemoryQueryTimedOut
    } else {
        error
    }
}

pub(crate) fn migrate(db: &Connection, state: &Snapshot) -> Result<()> {
    db.execute_batch("CREATE TABLE memory_records (
        rowid INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        agent_id TEXT NOT NULL,
        text TEXT NOT NULL CHECK(length(text) BETWEEN 1 AND 2000),
        revision INTEGER NOT NULL CHECK(revision > 0),
        created_at INTEGER NOT NULL CHECK(created_at >= 0),
        updated_at INTEGER NOT NULL CHECK(updated_at >= created_at),
        source_scene_id TEXT,
        source_message_id TEXT,
        origin TEXT NOT NULL CHECK(origin IN ('legacy_memory','legacy_reference','manual','conversation')),
        CHECK((source_scene_id IS NULL) = (source_message_id IS NULL))
    );
    CREATE INDEX memory_agent_order ON memory_records(agent_id,updated_at DESC,id);
    CREATE INDEX memory_agent_text ON memory_records(agent_id,text);
    CREATE TABLE memory_state (agent_id TEXT PRIMARY KEY, record_count INTEGER NOT NULL CHECK(record_count >= 0), revision INTEGER NOT NULL CHECK(revision >= 0));
    CREATE TABLE memory_migrations (name TEXT PRIMARY KEY, source_json TEXT NOT NULL);
    CREATE VIRTUAL TABLE memory_fts USING fts5(text,content='memory_records',content_rowid='rowid',tokenize='trigram');
    CREATE TRIGGER memory_insert AFTER INSERT ON memory_records BEGIN
        INSERT INTO memory_fts(rowid,text) VALUES(new.rowid,new.text);
        INSERT INTO memory_state(agent_id,record_count,revision) VALUES(new.agent_id,1,1)
        ON CONFLICT(agent_id) DO UPDATE SET record_count=record_count+1,revision=revision+1;
    END;
    CREATE TRIGGER memory_delete AFTER DELETE ON memory_records BEGIN
        INSERT INTO memory_fts(memory_fts,rowid,text) VALUES('delete',old.rowid,old.text);
        UPDATE memory_state SET record_count=record_count-1,revision=revision+1 WHERE agent_id=old.agent_id;
    END;
    CREATE TRIGGER memory_update AFTER UPDATE ON memory_records BEGIN
        INSERT INTO memory_fts(memory_fts,rowid,text) VALUES('delete',old.rowid,old.text);
        INSERT INTO memory_fts(rowid,text) VALUES(new.rowid,new.text);
        UPDATE memory_state SET revision=revision+1 WHERE agent_id=new.agent_id;
    END;")?;
    // A durable marker and the old fields are retained. Later opens never replay
    // this archive, including after a migrated record has been forgotten.
    db.execute("INSERT INTO memory_migrations(name,source_json) VALUES('snapshot-v1',?1)", [serde_json::to_string(&serde_json::json!({"memories":state.memories,"references":state.agents.iter().map(|a|serde_json::json!({"agentId":a.id,"text":a.memory})).collect::<Vec<_>>()}))?])?;
    for entry in &state.memories {
        insert(db, entry, "legacy_memory")?;
    }
    for agent in &state.agents {
        if agent.memory.trim().is_empty() {
            continue;
        }
        let mut chunk = String::new();
        let mut size = 0;
        for character in agent.memory.chars() {
            chunk.push(character);
            size += 1;
            if size == 2000 {
                insert_reference(db, &agent.id, std::mem::take(&mut chunk))?;
                size = 0;
            }
        }
        if !chunk.is_empty() {
            insert_reference(db, &agent.id, chunk)?;
        }
    }
    Ok(())
}

fn insert_reference(db: &Connection, agent_id: &str, text: String) -> Result<()> {
    let timestamp = crate::store::now();
    insert(
        db,
        &MemoryEntry {
            id: Uuid::new_v4().to_string(),
            agent_id: agent_id.into(),
            text,
            revision: 1,
            created_at: timestamp,
            updated_at: timestamp,
            source: None,
            origin: Some(MemoryOrigin::LegacyReference),
        },
        "legacy_reference",
    )
}

fn insert(db: &Connection, entry: &MemoryEntry, origin: &str) -> Result<()> {
    db.execute("INSERT INTO memory_records(id,agent_id,text,revision,created_at,updated_at,source_scene_id,source_message_id,origin) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![entry.id,entry.agent_id,entry.text,sql_int(entry.revision)?,sql_int(entry.created_at)?,sql_int(entry.updated_at)?,entry.source.as_ref().map(|s|s.scene_id.as_str()),entry.source.as_ref().map(|s|s.message_id.as_str()),origin])?;
    Ok(())
}

pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    let marker: i64 = db.query_row(
        "SELECT count(*) FROM memory_migrations WHERE name='snapshot-v1'",
        [],
        |r| r.get(0),
    )?;
    if marker != 1 {
        return Err(invalid("记忆迁移记录缺失，原数据未被覆盖"));
    }
    let mut sources =
        db.prepare("SELECT agent_id,source_scene_id,source_message_id FROM memory_records")?;
    let rows = sources.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (agent_id, scene_id, message_id) = row?;
        if !state.agents.iter().any(|a| a.id == agent_id) {
            return Err(invalid("记忆关联的 Agent 不存在"));
        }
        if let Some(scene_id) = scene_id {
            if !state.scenes.iter().any(|scene| {
                scene.id == scene_id
                    && scene.agent_id == agent_id
                    && scene.messages.iter().any(|m| {
                        Some(&m.id) == message_id.as_ref() && m.role == crate::MessageRole::User
                    })
            }) {
                return Err(invalid("记忆来源必须属于同一 Agent 的用户消息"));
            }
        } else if message_id.is_some() {
            return Err(invalid("记忆来源不完整"));
        }
    }
    let mismatch: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM memory_state s WHERE s.record_count != (SELECT count(*) FROM memory_records r WHERE r.agent_id=s.agent_id)) OR EXISTS(SELECT 1 FROM memory_records r WHERE NOT EXISTS(SELECT 1 FROM memory_state s WHERE s.agent_id=r.agent_id))",[],|r|r.get(0))?;
    if mismatch {
        return Err(invalid("记忆统计与记录不一致，原数据未被覆盖"));
    }
    Ok(())
}

pub(crate) fn stats(db: &Connection, state: &Snapshot) -> Result<Vec<MemoryStats>> {
    let mut statement = db.prepare("SELECT agent_id,record_count,revision FROM memory_state")?;
    let rows = statement.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, (unsigned(r, 1)?, unsigned(r, 2)?)))
    })?;
    let values: HashMap<_, _> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(state
        .agents
        .iter()
        .map(|agent| {
            let (count, revision) = values.get(&agent.id).copied().unwrap_or_default();
            MemoryStats {
                agent_id: agent.id.clone(),
                count,
                revision,
            }
        })
        .collect())
}

fn read(row: &Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let source_scene: Option<String> = row.get(6)?;
    let source_message: Option<String> = row.get(7)?;
    let origin: String = row.get(8)?;
    let origin = match origin.as_str() {
        "legacy_memory" => MemoryOrigin::LegacyMemory,
        "legacy_reference" => MemoryOrigin::LegacyReference,
        "manual" => MemoryOrigin::Manual,
        "conversation" => MemoryOrigin::Conversation,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(MemoryEntry {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        text: row.get(2)?,
        revision: unsigned(row, 3)?,
        created_at: unsigned(row, 4)?,
        updated_at: unsigned(row, 5)?,
        source: source_scene
            .zip(source_message)
            .map(|(scene_id, message_id)| MemorySource {
                scene_id,
                message_id,
            }),
        origin: Some(origin),
    })
}

pub(crate) fn mutate(
    db: &Connection,
    agent_id: &str,
    mutation: MemoryMutation,
    source: Option<MemorySource>,
) -> Result<()> {
    match mutation {
        MemoryMutation::Save {
            id,
            text,
            expected_revision,
        } => {
            let text = text.trim();
            if text.is_empty() || text.chars().count() > 2000 {
                return Err(invalid("每条记忆需为 1 至 2000 字"));
            }
            let duplicate: Option<String> = db
                .query_row(
                    "SELECT id FROM memory_records WHERE agent_id=?1 AND text=?2 LIMIT 1",
                    params![agent_id, text],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = id {
                if duplicate.as_ref().is_some_and(|existing| existing != &id) {
                    return Err(invalid("该 Agent 已有相同内容的记忆"));
                }
                let current: Option<(u64,u64)> = db.query_row("SELECT revision,updated_at FROM memory_records WHERE agent_id=?1 AND id=?2",params![agent_id,id],|r|Ok((unsigned(r,0)?,unsigned(r,1)?))).optional()?;
                let (revision, updated_at) = current.ok_or_else(|| CoreError::NotFound {
                    kind: "记忆",
                    id: id.clone(),
                })?;
                if expected_revision != Some(revision) {
                    return Err(CoreError::MemoryConflict);
                }
                let revision = revision
                    .checked_add(1)
                    .filter(|r| *r <= i64::MAX as u64)
                    .ok_or_else(|| invalid("记忆修订号已达上限"))?;
                db.execute("UPDATE memory_records SET text=?1,revision=?2,updated_at=?3,source_scene_id=?4,source_message_id=?5,origin=?6 WHERE id=?7 AND agent_id=?8",params![text,sql_int(revision)?,sql_int(crate::store::now().max(updated_at))?,source.as_ref().map(|s|s.scene_id.as_str()),source.as_ref().map(|s|s.message_id.as_str()),if source.is_some(){"conversation"}else{"manual"},id,agent_id])?;
            } else {
                if expected_revision.is_some() {
                    return Err(CoreError::MemoryConflict);
                }
                if duplicate.is_some() {
                    return Ok(());
                }
                let timestamp = crate::store::now();
                let origin = if source.is_some() {
                    "conversation"
                } else {
                    "manual"
                };
                insert(
                    db,
                    &MemoryEntry {
                        id: Uuid::new_v4().to_string(),
                        agent_id: agent_id.into(),
                        text: text.into(),
                        revision: 1,
                        created_at: timestamp,
                        updated_at: timestamp,
                        source,
                        origin: None,
                    },
                    origin,
                )?;
            }
        }
        MemoryMutation::Delete {
            id,
            expected_revision,
        } => {
            let current: Option<u64> = db
                .query_row(
                    "SELECT revision FROM memory_records WHERE agent_id=?1 AND id=?2",
                    params![agent_id, id],
                    |r| unsigned(r, 0),
                )
                .optional()?;
            let revision = current.ok_or_else(|| CoreError::NotFound {
                kind: "记忆",
                id: id.clone(),
            })?;
            if revision != expected_revision {
                return Err(CoreError::MemoryConflict);
            }
            db.execute(
                "DELETE FROM memory_records WHERE agent_id=?1 AND id=?2",
                params![agent_id, id],
            )?;
        }
    }
    Ok(())
}

struct QueryDeadline<'a>(&'a Connection);
impl<'a> QueryDeadline<'a> {
    fn new(db: &'a Connection) -> Result<Self> {
        let started = Instant::now();
        db.progress_handler(
            2000,
            Some(move || started.elapsed() > Duration::from_millis(250)),
        )?;
        Ok(Self(db))
    }
}
impl Drop for QueryDeadline<'_> {
    fn drop(&mut self) {
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    agent: String,
    query: String,
    revision: u64,
    updated: u64,
    id: String,
}
fn query_text(query: &str) -> Result<&str> {
    let query = query.trim();
    if query.chars().count() > 200 || query.contains('\0') {
        return Err(invalid("记忆查询最多 200 字，不能包含空字符"));
    }
    Ok(query)
}
fn quote(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

pub(crate) fn page(
    db: &Connection,
    agent_id: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<MemoryPage> {
    // Stats, cursor revision and rows must describe one SQLite read snapshot,
    // even when another process commits between these individual SELECTs.
    let transaction = db.unchecked_transaction()?;
    page_in_transaction(&transaction, agent_id, query, cursor, limit)
}

/// Reuse an already-pinned transaction for streaming a consistent export.
pub(crate) fn page_in_transaction(
    db: &Connection,
    agent_id: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<MemoryPage> {
    if db.is_autocommit() {
        return Err(invalid("记忆分页缺少读取事务"));
    }
    if !(1..=25).contains(&limit) {
        return Err(invalid("每页需为 1 至 25 条"));
    }
    let query = query_text(query)?;
    let _deadline = QueryDeadline::new(db)?;
    let (record_count, revision) = db
        .query_row(
            "SELECT record_count,revision FROM memory_state WHERE agent_id=?1",
            [agent_id],
            |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
        )
        .optional()?
        .unwrap_or_default();
    let after = cursor
        .map(|text| {
            if text.len() > 4096 {
                return Err(invalid("记忆分页位置无效"));
            }
            let after: Cursor =
                serde_json::from_str(text).map_err(|_| invalid("记忆分页位置无效"))?;
            if after.agent != agent_id || after.query != query || after.revision != revision {
                return Err(CoreError::MemoryConflict);
            }
            Ok(after)
        })
        .transpose()?;
    let filter = if query.is_empty() {
        "1"
    } else if query.chars().count() >= 3 {
        "m.rowid IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?2)"
    } else {
        "instr(lower(m.text),lower(?2))>0"
    };
    let search = if query.chars().count() >= 3 {
        quote(query)
    } else {
        query.to_string()
    };
    // Numeric placeholders are deliberately shared by the static predicate.
    let filter = if query.is_empty() {
        "(?2 = '')"
    } else {
        filter
    };
    let total: u64 = if query.is_empty() {
        record_count
    } else {
        db.query_row(
            &format!("SELECT count(*) FROM memory_records m WHERE m.agent_id=?1 AND {filter}"),
            params![agent_id, search],
            |r| unsigned(r, 0),
        )?
    };
    let after_filter = if after.is_some() {
        "m.updated_at<=?3 AND (m.updated_at<?3 OR m.id>?4)"
    } else {
        "?3 IS NULL AND ?4 IS NULL"
    };
    let mut statement=db.prepare(&format!("SELECT {COLUMNS} FROM memory_records m WHERE m.agent_id=?1 AND {filter} AND ({after_filter}) ORDER BY m.updated_at DESC,m.id LIMIT ?5"))?;
    let rows = statement.query_map(
        params![
            agent_id,
            search,
            after.as_ref().map(|a| sql_int(a.updated)).transpose()?,
            after.as_ref().map(|a| a.id.as_str()),
            limit as i64 + 1
        ],
        read,
    )?;
    let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let more = entries.len() > limit;
    entries.truncate(limit);
    let next_cursor = if more {
        let last = entries.last().expect("nonempty page");
        Some(serde_json::to_string(&Cursor {
            agent: agent_id.into(),
            query: query.into(),
            revision,
            updated: last.updated_at,
            id: last.id.clone(),
        })?)
    } else {
        None
    };
    Ok(MemoryPage {
        entries,
        total,
        revision,
        next_cursor,
    })
}

pub(crate) fn entry(db: &Connection, agent_id: &str, id: &str) -> Result<Option<MemoryEntry>> {
    if id.len() > 200 {
        return Err(invalid("记忆标识无效"));
    }
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM memory_records m WHERE m.agent_id=?1 AND m.id=?2"),
            params![agent_id, id],
            read,
        )
        .optional()?)
}

// Trigram recall is lexical, not semantic. Expand a natural-language question
// into bounded OR terms; callers may also ask with a short, exact phrase.
fn recall_expression(query: &str) -> String {
    let mut terms = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < 3 {
            continue;
        }
        if word.is_ascii() {
            terms.push(quote(word));
        } else {
            for chunk in chars.windows(3) {
                terms.push(quote(&chunk.iter().collect::<String>()));
                if terms.len() >= 48 {
                    break;
                }
            }
        }
        if terms.len() >= 48 {
            break;
        }
    }
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        quote(query)
    } else {
        terms.join(" OR ")
    }
}

pub(crate) fn recall(db: &Connection, agent_id: &str, query: &str) -> Result<Vec<MemoryEntry>> {
    let query = query_text(query)?;
    let _deadline = QueryDeadline::new(db)?;
    let (sql, search) = if query.is_empty() {
        (format!("SELECT {COLUMNS} FROM memory_records m WHERE m.agent_id=?1 AND ?2='' ORDER BY m.updated_at DESC,m.id LIMIT 32"),String::new())
    } else if query.chars().count() < 3 {
        (format!("SELECT {COLUMNS} FROM memory_records m WHERE m.agent_id=?1 AND instr(lower(m.text),lower(?2))>0 ORDER BY m.updated_at DESC,m.id LIMIT 32"),query.into())
    } else {
        (format!("SELECT {COLUMNS} FROM memory_records m JOIN memory_fts f ON f.rowid=m.rowid WHERE m.agent_id=?1 AND memory_fts MATCH ?2 ORDER BY bm25(memory_fts),m.updated_at DESC,m.id LIMIT 32"),recall_expression(query))
    };
    let mut statement = db.prepare(&sql)?;
    let rows = statement.query_map(params![agent_id, search], read)?;
    let mut output = Vec::new();
    let mut chars = 0;
    let mut bytes = 2; // JSON array brackets, with one comma for each later item.
    for row in rows {
        let entry = row?;
        let length = entry.text.chars().count();
        let encoded = serde_json::to_vec(&entry)?.len() + usize::from(!output.is_empty());
        if chars + length > RECALL_CHARS || bytes + encoded > RECALL_BYTES {
            continue;
        }
        chars += length;
        bytes += encoded;
        output.push(entry);
        if output.len() == RECALL_ITEMS {
            break;
        }
    }
    Ok(output)
}
