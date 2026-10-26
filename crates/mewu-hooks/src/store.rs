// SPDX-License-Identifier: MPL-2.0
use crate::{Event, Result, Rule, MAX_RULES};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub rules: Vec<Rule>,
    pub runs: Vec<Run>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub rule_id: String,
    pub rule_revision: u64,
    pub event_id: String,
    pub event_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub completed_actions: usize,
    pub error: Option<String>,
}
pub struct Store {
    db: Connection,
}
fn db(_: rusqlite::Error) -> String {
    "事件触发器存储不可用".into()
}
const RUN_SELECT:&str="SELECT id,rule_id,rule_revision,event_id,event_name,started_at,finished_at,status,completed_actions,error FROM hook_runs";
fn uint(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let v: i64 = r.get(index)?;
    v.try_into()
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, v))
}
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    Ok(Run {
        id: r.get(0)?,
        rule_id: r.get(1)?,
        rule_revision: uint(r, 2)?,
        event_id: r.get(3)?,
        event_name: r.get(4)?,
        started_at: r.get(5)?,
        finished_at: r.get(6)?,
        status: r.get(7)?,
        completed_actions: uint(r, 8)? as usize,
        error: r.get(9)?,
    })
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_connection(Connection::open(path).map_err(db)?)
    }
    pub fn memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory().map_err(db)?)
    }
    fn from_connection(db_: Connection) -> Result<Self> {
        db_.busy_timeout(Duration::from_secs(2)).map_err(db)?;
        let version: i64 = db_
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(db)?;
        if version > 2 {
            return Err("事件触发器数据来自更新的软件版本".into());
        }
        db_.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS hook_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL);
            INSERT OR IGNORE INTO hook_meta VALUES(1,0);
            CREATE TABLE IF NOT EXISTS hook_rules(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS hook_runs(id TEXT PRIMARY KEY,rule_id TEXT NOT NULL,rule_revision INTEGER NOT NULL,event_id TEXT NOT NULL,event_name TEXT NOT NULL,started_at INTEGER NOT NULL,finished_at INTEGER,status TEXT NOT NULL,completed_actions INTEGER NOT NULL DEFAULT 0,error TEXT,UNIQUE(rule_id,event_id));
            CREATE INDEX IF NOT EXISTS hook_runs_started ON hook_runs(started_at DESC);
            CREATE TABLE IF NOT EXISTS hook_imports(source TEXT PRIMARY KEY);
            PRAGMA user_version=2; COMMIT;").map_err(db)?;
        // A prior process's uncertain effects must never be replayed on startup.
        db_.execute("UPDATE hook_runs SET status='interrupted',finished_at=?1,error='上次运行中断，未自动重试' WHERE status='running'",[chrono::Utc::now().timestamp_millis()]).map_err(db)?;
        Ok(Self { db: db_ })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let revision = self.revision()?;
        let rules = self.rules()?;
        let mut q = self
            .db
            .prepare(&format!(
                "{RUN_SELECT} ORDER BY started_at DESC,rowid DESC LIMIT 100"
            ))
            .map_err(db)?;
        let runs = q
            .query_map([], row)
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(Snapshot {
            revision,
            rules,
            runs,
        })
    }
    pub fn revision(&self) -> Result<u64> {
        self.db
            .query_row(
                "SELECT revision FROM hook_meta WHERE singleton=1",
                [],
                |r| uint(r, 0),
            )
            .map_err(db)
    }
    pub fn rules(&self) -> Result<Vec<Rule>> {
        let mut q = self
            .db
            .prepare("SELECT body FROM hook_rules ORDER BY rowid")
            .map_err(db)?;
        let rows = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db)?;
        if rows.len() > MAX_RULES || rows.iter().map(String::len).sum::<usize>() > 2 * 1024 * 1024 {
            return Err("事件触发器规则超出限制".into());
        }
        rows.iter()
            .map(|s| {
                let r: Rule = serde_json::from_str(s).map_err(|_| "规则数据无效")?;
                r.validate()?;
                Ok(r)
            })
            .collect()
    }
    pub fn rule(&self, id: &str) -> Result<Option<Rule>> {
        let s: Option<String> = self
            .db
            .query_row("SELECT body FROM hook_rules WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db)?;
        s.map(|s| serde_json::from_str(&s).map_err(|_| "规则数据无效".into()))
            .transpose()
    }
    /// Import a former settings source exactly once. Imported entries are normal
    /// editable/removable rules; the durable marker prevents resurrection after
    /// deletion. Configuration and marker commit together, including an explicit
    /// empty/disabled source. Existing user rules always take precedence.
    pub fn imported(&self, source: &str) -> Result<bool> {
        self.db
            .query_row(
                "SELECT 1 FROM hook_imports WHERE source=?1",
                [source],
                |r| r.get::<_, i32>(0),
            )
            .optional()
            .map(|v| v.is_some())
            .map_err(db)
    }
    pub fn import_once(&mut self, source: &str, rules: Vec<Rule>) -> Result<bool> {
        if source.is_empty()
            || source.len() > 120
            || !source
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err("迁移来源无效".into());
        }
        for rule in &rules {
            rule.validate()?;
            if rule.revision != 0 {
                return Err("导入规则版本无效".into());
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if tx
            .query_row(
                "SELECT 1 FROM hook_imports WHERE source=?1",
                [source],
                |r| r.get::<_, i32>(0),
            )
            .optional()
            .map_err(db)?
            .is_some()
        {
            return Ok(false);
        }
        let mut existing = {
            let mut q = tx.prepare("SELECT body FROM hook_rules").map_err(db)?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(db)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db)?;
            rows.into_iter()
                .map(|s| serde_json::from_str::<Rule>(&s).map_err(|_| "规则数据无效".to_string()))
                .collect::<Result<Vec<_>>>()?
        };
        let mut inserted = false;
        for mut rule in rules {
            if existing.iter().any(|r| r.id == rule.id) {
                return Err("导入规则标识重复".into());
            }
            if rule.enabled&&existing.iter().any(|r|r.enabled&&matches!((&r.trigger,&rule.trigger),(crate::Trigger::Shortcut{accelerator:a},crate::Trigger::Shortcut{accelerator:b})if a==b)){continue;}
            if existing.len() >= MAX_RULES {
                return Err("事件触发器规则超出限制".into());
            }
            rule.revision = 1;
            let body = serde_json::to_string(&rule).map_err(|_| "规则格式无效")?;
            let bytes = existing.iter().try_fold(body.len(), |sum, r| {
                serde_json::to_vec(r)
                    .map(|s| sum + s.len())
                    .map_err(|_| "规则格式无效")
            })?;
            if bytes > 2 * 1024 * 1024 {
                return Err("规则配置总大小超过 2 MiB".into());
            }
            tx.execute(
                "INSERT INTO hook_rules VALUES(?1,1,?2)",
                params![rule.id, body],
            )
            .map_err(db)?;
            existing.push(rule);
            inserted = true;
        }
        tx.execute("INSERT INTO hook_imports VALUES(?1)", [source])
            .map_err(db)?;
        if inserted {
            tx.execute(
                "UPDATE hook_meta SET revision=revision+1 WHERE singleton=1",
                [],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(true)
    }
    pub fn save(&mut self, expected: u64, mut rule: Rule) -> Result<Snapshot> {
        rule.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let revision: u64 = tx
            .query_row(
                "SELECT revision FROM hook_meta WHERE singleton=1",
                [],
                |r| uint(r, 0),
            )
            .map_err(db)?;
        if revision != expected {
            return Err("规则已变更，请重新载入".into());
        }
        let old: Option<u64> = tx
            .query_row(
                "SELECT revision FROM hook_rules WHERE id=?1",
                [&rule.id],
                |r| uint(r, 0),
            )
            .optional()
            .map_err(db)?;
        if old.unwrap_or(0) != rule.revision {
            return Err("这条规则已变更，请重新载入".into());
        }
        if old.is_none() {
            let n: i64 = tx
                .query_row("SELECT count(*) FROM hook_rules", [], |r| r.get(0))
                .map_err(db)?;
            if n >= MAX_RULES as i64 {
                return Err("规则数量已达到上限".into());
            }
        }
        // A hotkey has one owner. Native registration checks the same semantic
        // accelerator against core/OS bindings before calling this transaction.
        if let crate::Trigger::Shortcut { accelerator } = &rule.trigger {
            let rows: Vec<String> = {
                let mut q = tx
                    .prepare("SELECT body FROM hook_rules WHERE id<>?1")
                    .map_err(db)?;
                let v = q
                    .query_map([&rule.id], |r| r.get(0))
                    .map_err(db)?
                    .collect::<std::result::Result<Vec<String>, _>>()
                    .map_err(db)?;
                v
            };
            for s in rows {
                let r: Rule = serde_json::from_str(&s).map_err(|_| "规则数据无效")?;
                if rule.enabled
                    && r.enabled
                    && matches!(r.trigger,crate::Trigger::Shortcut{accelerator:ref a}if a==accelerator)
                {
                    return Err("此快捷键已用于另一条规则".into());
                }
            }
        }
        rule.revision = rule
            .revision
            .checked_add(1)
            .filter(|n| *n < i64::MAX as u64)
            .ok_or("规则版本过大")?;
        let body = serde_json::to_string(&rule).map_err(|_| "规则格式无效")?;
        let retained: i64 = tx
            .query_row(
                "SELECT coalesce(sum(length(CAST(body AS BLOB))),0) FROM hook_rules WHERE id<>?1",
                [&rule.id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if retained.saturating_add(body.len() as i64) > 2 * 1024 * 1024 {
            return Err("规则配置总大小超过 2 MiB".into());
        }
        tx.execute("INSERT INTO hook_rules VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,body=excluded.body",params![rule.id,rule.revision as i64,body]).map_err(db)?;
        tx.execute(
            "UPDATE hook_meta SET revision=revision+1 WHERE singleton=1",
            [],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        self.snapshot()
    }
    pub fn remove(&mut self, expected: u64, id: &str, rule_revision: u64) -> Result<Snapshot> {
        crate::valid_uuid(id)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let revision: u64 = tx
            .query_row(
                "SELECT revision FROM hook_meta WHERE singleton=1",
                [],
                |r| uint(r, 0),
            )
            .map_err(db)?;
        if revision != expected {
            return Err("规则已变更，请重新载入".into());
        }
        if rule_revision >= i64::MAX as u64 {
            return Err("规则版本无效".into());
        }
        if tx
            .execute(
                "DELETE FROM hook_rules WHERE id=?1 AND revision=?2",
                params![id, rule_revision as i64],
            )
            .map_err(db)?
            != 1
        {
            return Err("这条规则已变更或删除".into());
        }
        tx.execute(
            "UPDATE hook_meta SET revision=revision+1 WHERE singleton=1",
            [],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        self.snapshot()
    }
    pub fn claim(
        &mut self,
        rule: &Rule,
        event: &Event,
        now: i64,
        manual: bool,
    ) -> Result<Option<Run>> {
        event.validate()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        rule.validate()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT body FROM hook_rules WHERE id=?1 AND revision=?2",
                params![rule.id, rule.revision as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        let Some(current) = current else {
            return Ok(None);
        };
        let current: Rule = serde_json::from_str(&current).map_err(|_| "规则数据无效")?;
        if !current.enabled {
            return Ok(None);
        }
        let active: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM hook_runs WHERE rule_id=?1 AND status='running')",
                [&rule.id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if active {
            return Ok(None);
        }
        let last: Option<i64> = tx
            .query_row(
                "SELECT max(started_at) FROM hook_runs WHERE rule_id=?1",
                [&rule.id],
                |r| r.get(0),
            )
            .map_err(db)?;
        if !manual && last.is_some_and(|last| now.saturating_sub(last) < rule.cooldown_ms as i64) {
            return Ok(None);
        }
        let run = Run {
            id: uuid::Uuid::new_v4().to_string(),
            rule_id: rule.id.clone(),
            rule_revision: rule.revision,
            event_id: event.id.clone(),
            event_name: event.name.clone(),
            started_at: now,
            finished_at: None,
            status: "running".into(),
            completed_actions: 0,
            error: None,
        };
        let n=tx.execute("INSERT OR IGNORE INTO hook_runs(id,rule_id,rule_revision,event_id,event_name,started_at,status) VALUES(?1,?2,?3,?4,?5,?6,'running')",params![run.id,run.rule_id,run.rule_revision as i64,run.event_id,run.event_name,now]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok((n == 1).then_some(run))
    }
    pub fn progress(&mut self, id: &str, completed: usize) -> Result<()> {
        if completed > 16 {
            return Err("动作计数无效".into());
        }
        self.db
            .execute(
                "UPDATE hook_runs SET completed_actions=?2 WHERE id=?1 AND status='running'",
                params![id, completed as i64],
            )
            .map_err(db)?;
        Ok(())
    }
    pub fn finish(&mut self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        if !["completed", "failed", "canceled", "skipped", "interrupted"].contains(&status) {
            return Err("运行状态无效".into());
        }
        self.db.execute("UPDATE hook_runs SET status=?2,finished_at=?3,error=?4 WHERE id=?1 AND status='running'",params![id,status,chrono::Utc::now().timestamp_millis(),error.map(|s|s.chars().take(240).collect::<String>())]).map_err(db)?;
        self.db.execute("DELETE FROM hook_runs WHERE status<>'running' AND id NOT IN(SELECT id FROM hook_runs ORDER BY started_at DESC,rowid DESC LIMIT 500)",[]).map_err(db)?;
        Ok(())
    }
}
