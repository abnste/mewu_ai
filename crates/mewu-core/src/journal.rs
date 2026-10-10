// SPDX-License-Identifier: MPL-2.0
//! Durable execution observations. This module never executes a tool or a request.
use crate::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) type Result<T> = std::result::Result<T, CoreError>;
pub(crate) const TEXT_LIMIT: usize = 2 * 1024 * 1024;
pub(crate) const TOOL_LIMIT: usize = 128 * 1024;
pub(crate) const PROJECTION_LIMIT: usize = 256 * 1024;
pub(crate) fn bad(message: &str) -> CoreError {
    CoreError::Invalid(message.into())
}
pub(crate) fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn validate_binding(binding: &ToolBinding) -> Result<()> {
    let name =
        |s: &str| !s.is_empty() && s.chars().count() <= 128 && !s.chars().any(char::is_control);
    match binding {
        ToolBinding::VideoAnnotation {
            plugin_id,
            plugin_revision,
            contribution_id,
            name: n,
            target_manifest_sha256,
        } => {
            crate::visual_annotations::validate_grant(&VisualAnnotationGrant {
                plugin_id: plugin_id.clone(),
                plugin_revision: *plugin_revision,
                contribution_id: contribution_id.clone(),
            })?;
            if n != VIDEO_ANNOTATION_TOOL || !hash_valid(target_manifest_sha256) {
                return Err(bad("视频批注回执身份无效"));
            }
        }
        ToolBinding::VisualAnnotation {
            plugin_id,
            plugin_revision,
            contribution_id,
            name: n,
            target_manifest_sha256,
        } => {
            crate::visual_annotations::validate_grant(&VisualAnnotationGrant {
                plugin_id: plugin_id.clone(),
                plugin_revision: *plugin_revision,
                contribution_id: contribution_id.clone(),
            })?;
            if n != VISUAL_ANNOTATION_TOOL || !hash_valid(target_manifest_sha256) {
                return Err(bad("批注回执身份无效"));
            }
        }
        ToolBinding::Mcp {
            server_id,
            revision,
            name: n,
            advertised_alias,
        } => {
            uuid(server_id)?;
            if *revision == 0 || !name(n) || !name(advertised_alias) {
                return Err(bad("MCP 回执身份无效"));
            }
        }
        ToolBinding::Local { name: n } | ToolBinding::Rejected { advertised_name: n } => {
            if !name(n) {
                return Err(bad("工具回执名称无效"));
            }
        }
        ToolBinding::ExternalMemory {
            binding_id,
            binding_revision,
            plugin_id,
            plugin_revision,
            contribution_id,
            name: n,
        } => {
            uuid(binding_id)?;
            if *binding_revision == 0
                || *plugin_revision == 0
                || !name(plugin_id)
                || !name(contribution_id)
                || !name(n)
            {
                return Err(bad("记忆回执身份无效"));
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_references(references: &[EvidenceReference]) -> Result<()> {
    if references.len() > 64 {
        return Err(bad("工具来源过多"));
    }
    for reference in references {
        match reference {
            EvidenceReference::LocalMemory { id, revision } => {
                uuid(id)?;
                if *revision == 0 {
                    return Err(bad("记忆来源版本无效"));
                }
            }
            EvidenceReference::ExternalMemory {
                binding_id,
                evidence_id,
                operation_id,
                revision,
            } => {
                uuid(binding_id)?;
                uuid(evidence_id)?;
                if let Some(id) = operation_id {
                    uuid(id)?;
                }
                if *revision == 0 {
                    return Err(bad("记忆来源版本无效"));
                }
            }
            EvidenceReference::History {
                scene_id,
                message_id,
            } => {
                uuid(scene_id)?;
                uuid(message_id)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn uuid(s: &str) -> Result<()> {
    match uuid::Uuid::parse_str(s) {
        Ok(id) if id.to_string() == s => Ok(()),
        _ => Err(bad("执行记录 ID 无效")),
    }
}
pub(crate) fn encode<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn number(n: u64) -> Result<i64> {
    i64::try_from(n).map_err(|_| bad("执行记录版本溢出"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Record {
    pub id: String,
    pub scene: String,
    pub agent: String,
    pub user: String,
    pub kind: RecordedRunKind,
    pub status: RecordedRunStatus,
    pub revision: u64,
    pub checkpoint: u64,
    pub created: u64,
    pub updated: u64,
    pub connection_id: String,
    pub connection_revision: u64,
    pub provenance: Option<ContinuationProvenance>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Event {
    pub summary: JournalEntrySummary,
    // Exact byte contribution when this JSON body appears inside the quoted
    // evidence string. Private derived metadata; verified against body on open.
    pub projection_content_bytes: u64,
    pub binding: Option<ToolBinding>,
    pub arguments_sha256: Option<String>,
    pub references: Vec<EvidenceReference>,
    pub nonce: String,
    pub request_sha256: Option<String>,
    pub not_sent: Option<NotSentReason>,
    pub unknown: Option<NoResponseReason>,
}

pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE agent_run_records(id TEXT PRIMARY KEY,scene_id TEXT NOT NULL,created_at INTEGER NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),data TEXT NOT NULL CHECK(length(CAST(data AS BLOB))<=2097152));
CREATE INDEX agent_runs_scene ON agent_run_records(scene_id,created_at DESC,id);
CREATE TABLE agent_run_events(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 64),metadata TEXT NOT NULL CHECK(length(CAST(metadata AS BLOB))<=65536),content TEXT NOT NULL CHECK(length(CAST(content AS BLOB))<=12587008),UNIQUE(run_id,sequence));
CREATE INDEX agent_events_run ON agent_run_events(run_id,sequence);")?;
    Ok(())
}
pub(crate) fn record(db: &Connection, id: &str) -> Result<Option<Record>> {
    let s: Option<String> = db
        .query_row(
            "SELECT data FROM agent_run_records WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
}
pub(crate) fn required(db: &Connection, id: &str) -> Result<Record> {
    record(db, id)?.ok_or(CoreError::JournalConflict)
}
pub(crate) fn event(db: &Connection, id: &str) -> Result<(String, Event, JournalContent)> {
    let (run, meta, body): (String, String, String) = db
        .query_row(
            "SELECT run_id,metadata,content FROM agent_run_events WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(CoreError::JournalConflict)?;
    Ok((
        run,
        serde_json::from_str(&meta)?,
        serde_json::from_str(&body)?,
    ))
}
pub(crate) fn events(db: &Connection, run: &str) -> Result<Vec<Event>> {
    let mut stmt =
        db.prepare("SELECT metadata FROM agent_run_events WHERE run_id=?1 ORDER BY sequence")?;
    let rows = stmt.query_map([run], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
pub(crate) fn save_record(db: &Connection, r: &Record) -> Result<()> {
    db.execute(
        "UPDATE agent_run_records SET revision=?1,data=?2 WHERE id=?3",
        params![number(r.revision)?, encode(r)?, r.id],
    )?;
    Ok(())
}
pub(crate) fn save_event(db: &Connection, e: &Event, body: &JournalContent) -> Result<()> {
    db.execute(
        "UPDATE agent_run_events SET metadata=?1,content=?2 WHERE id=?3",
        params![encode(e)?, encode(body)?, e.summary.id],
    )?;
    Ok(())
}
pub(crate) fn bump(db: &Connection, r: &mut Record) -> Result<ReceiptCommit> {
    r.revision = r
        .revision
        .checked_add(1)
        .ok_or(CoreError::JournalConflict)?;
    r.updated = stamp().max(r.updated);
    r.checkpoint = 0;
    for e in events(db, &r.id)? {
        if matches!(
            e.summary.phase,
            ReceiptPhase::Prepared | ReceiptPhase::Dispatched
        ) {
            break;
        }
        r.checkpoint = e.summary.sequence;
    }
    save_record(db, r)?;
    Ok(ReceiptCommit {
        journal_revision: r.revision,
        checkpoint_seq: r.checkpoint,
    })
}
pub(crate) fn create_event(
    db: &Connection,
    r: &Record,
    kind: ReceiptKind,
    round: u32,
    label: String,
    call: Option<String>,
    binding: Option<ToolBinding>,
    arguments: Option<String>,
    request: Option<String>,
) -> Result<(Event, DispatchLease)> {
    let sequence: i64 = db.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_run_events WHERE run_id=?1",
        [&r.id],
        |row| row.get(0),
    )?;
    if sequence > 64 {
        return Err(bad("执行记录超过单次运行预算"));
    }
    let event_id = uuid::Uuid::new_v4().to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let s = JournalEntrySummary {
        id: event_id.clone(),
        sequence: sequence as u64,
        kind,
        phase: ReceiptPhase::Prepared,
        round,
        tool_call_id: call,
        label,
        started_at: stamp(),
        finished_at: None,
        returned_error: None,
        content_bytes: 0,
        content_sha256: None,
        content_unavailable: Some(ContentUnavailable::NotReceived),
        response_kind: None,
        observed_encoding: None,
        observed_bytes: None,
        observed_sha256: None,
        selectable: false,
    };
    let body = JournalContent::Omitted {
        reason: ContentUnavailable::NotReceived,
    };
    let e = Event {
        summary: s,
        projection_content_bytes: projection_content_bytes(&body)?,
        binding,
        arguments_sha256: arguments,
        references: vec![],
        nonce: nonce.clone(),
        request_sha256: request,
        not_sent: None,
        unknown: None,
    };
    db.execute(
        "INSERT INTO agent_run_events(id,run_id,sequence,metadata,content) VALUES(?1,?2,?3,?4,?5)",
        params![event_id, r.id, sequence, encode(&e)?, encode(&body)?],
    )?;
    Ok((
        e,
        DispatchLease {
            run_id: r.id.clone(),
            entry_id: event_id,
            nonce,
        },
    ))
}
pub(crate) fn leased(
    db: &Connection,
    lease: &DispatchLease,
) -> Result<(Record, Event, JournalContent)> {
    let (run, e, body) = event(db, &lease.entry_id)?;
    if run != lease.run_id || e.nonce != lease.nonce {
        return Err(CoreError::JournalConflict);
    }
    Ok((required(db, &run)?, e, body))
}
pub(crate) fn set_content(e: &mut Event, body: &JournalContent) -> Result<()> {
    let bytes = serde_json::to_vec(body)?;
    e.projection_content_bytes = projection_content_bytes(body)?;
    e.summary.content_bytes = bytes.len() as u64;
    e.summary.content_sha256 = Some(digest(&bytes));
    e.summary.content_unavailable = match body {
        JournalContent::Omitted { reason } => Some(reason.clone()),
        _ => None,
    };
    e.summary.selectable = matches!(body,JournalContent::Text{text} if !text.trim().is_empty())
        || matches!(body, JournalContent::Json { .. });
    Ok(())
}

fn projection_content_bytes(body: &JournalContent) -> Result<u64> {
    // The envelope embeds body as JSON, then the complete envelope is encoded
    // as a JSON string. Exclude the outer string's own two quote bytes.
    Ok((serde_json::to_vec(&encode(body)?)?.len() - 2) as u64)
}
pub(crate) fn sync_step(state: &mut Snapshot, r: &Record, e: &Event) -> Result<()> {
    if e.summary.kind != ReceiptKind::Tool {
        return Ok(());
    }
    let scene = state
        .scenes
        .iter_mut()
        .find(|s| s.id == r.scene)
        .ok_or(CoreError::JournalConflict)?;
    // Event identity, not provider call ID, also represents rejected duplicate IDs.
    let name = match e.binding.as_ref() {
        Some(ToolBinding::Mcp {
            advertised_alias, ..
        }) => advertised_alias.clone(),
        Some(
            ToolBinding::Local { name }
            | ToolBinding::ExternalMemory { name, .. }
            | ToolBinding::VisualAnnotation { name, .. }
            | ToolBinding::VideoAnnotation { name, .. },
        ) => name.clone(),
        Some(ToolBinding::Rejected { advertised_name }) => advertised_name.clone(),
        None => "tool".into(),
    };
    let status = match e.summary.phase {
        ReceiptPhase::Prepared | ReceiptPhase::Dispatched => ToolStepStatus::Running,
        ReceiptPhase::Unknown => ToolStepStatus::Unknown,
        ReceiptPhase::NotSent => ToolStepStatus::Failed,
        ReceiptPhase::Responded => {
            if matches!(
                e.summary.response_kind,
                Some(ToolResponseKind::Task | ToolResponseKind::InputRequired)
            ) {
                ToolStepStatus::Unknown
            } else if e.summary.returned_error == Some(true)
                || matches!(e.summary.response_kind, Some(ToolResponseKind::RpcError))
            {
                ToolStepStatus::Failed
            } else {
                ToolStepStatus::Completed
            }
        }
    };
    let note = match status {
        ToolStepStatus::Unknown => Some("结果未知".into()),
        ToolStepStatus::Failed if e.summary.phase == ReceiptPhase::NotSent => Some("未执行".into()),
        _ => None,
    };
    let step = ToolStep {
        id: e.summary.id.clone(),
        name,
        label: e.summary.label.clone(),
        status,
        started_at: e.summary.started_at / 1000,
        finished_at: e.summary.finished_at.map(|n| n / 1000),
        summary: note,
    };
    let set = |steps: &mut Vec<ToolStep>| {
        if let Some(old) = steps.iter_mut().find(|s| s.id == step.id) {
            *old = step.clone()
        } else {
            steps.push(step.clone())
        }
    };
    let live = scene.run.as_mut().filter(|run| run.id == r.id);
    if let Some(run) = live {
        set(&mut run.steps);
    }
    if r.status != RecordedRunStatus::Running {
        let user = scene
            .messages
            .iter_mut()
            .find(|m| m.id == r.user)
            .ok_or(CoreError::JournalConflict)?;
        set(&mut user.tool_steps);
    }
    Ok(())
}
pub(crate) fn on_snapshot_change(
    db: &Connection,
    old: &Snapshot,
    next: &mut Snapshot,
) -> Result<()> {
    let changes: Vec<(String, Run)> = next
        .scenes
        .iter()
        .filter_map(|s| s.run.clone().map(|r| (s.id.clone(), r)))
        .collect();
    for (scene_id, run) in changes {
        let previous = old
            .scenes
            .iter()
            .find(|s| s.id == scene_id)
            .and_then(|s| s.run.as_ref());
        let mut r = record(db, &run.id)?;
        if r.is_none() && previous.is_none_or(|p| p.id != run.id) {
            let s = next
                .scenes
                .iter()
                .find(|s| s.id == scene_id)
                .ok_or(CoreError::JournalConflict)?;
            let user = s
                .messages
                .iter()
                .find(|m| m.run_id.as_deref() == Some(&run.id) && m.role == MessageRole::User)
                .ok_or(CoreError::JournalConflict)?;
            let t = stamp();
            let created = Record {
                id: run.id.clone(),
                scene: s.id.clone(),
                agent: s.agent_id.clone(),
                user: user.id.clone(),
                kind: run.kind.clone(),
                status: RecordedRunStatus::Running,
                revision: 1,
                checkpoint: 0,
                created: t,
                updated: t,
                connection_id: run
                    .connection_id
                    .clone()
                    .ok_or(CoreError::ConnectionUnselected)?,
                connection_revision: run
                    .connection_revision
                    .ok_or(CoreError::ConnectionUnselected)?,
                provenance: None,
            };
            db.execute("INSERT INTO agent_run_records(id,scene_id,created_at,revision,data) VALUES(?1,?2,?3,1,?4)",params![created.id,created.scene,number(t)?,encode(&created)?])?;
            r = Some(created);
        }
        let Some(mut r) = r else { continue };
        if run.status != RunStatus::Running && r.status == RecordedRunStatus::Running {
            r.status = match run.status {
                RunStatus::Completed => RecordedRunStatus::Completed,
                RunStatus::Canceled => RecordedRunStatus::Canceled,
                _ => {
                    if run.error.as_deref() == Some("上次运行已中断") {
                        RecordedRunStatus::Interrupted
                    } else {
                        RecordedRunStatus::Failed
                    }
                }
            };
            for mut e in events(db, &r.id)? {
                if matches!(
                    e.summary.phase,
                    ReceiptPhase::Prepared | ReceiptPhase::Dispatched
                ) {
                    if e.summary.phase == ReceiptPhase::Prepared {
                        e.summary.phase = ReceiptPhase::NotSent;
                        e.not_sent = Some(NotSentReason::RunEnded)
                    } else {
                        e.summary.phase = ReceiptPhase::Unknown;
                        e.unknown = Some(NoResponseReason::Canceled)
                    }
                    e.summary.finished_at = Some(stamp().max(e.summary.started_at));
                    let (_, _, body) = event(db, &e.summary.id)?;
                    save_event(db, &e, &body)?;
                }
                sync_step(next, &r, &e)?;
            }
            bump(db, &mut r)?;
        }
    }
    Ok(())
}

pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    // Stream records and bodies; no global payload/ID Vec and no per-row lookup.
    let mut records = db.prepare(
        "SELECT r.id,r.scene_id,r.created_at,r.revision,r.data,source.data FROM agent_run_records r LEFT JOIN agent_run_records source ON source.id=json_extract(r.data,'$.provenance.sourceRunId') ORDER BY r.id",
    )?;
    let mut rows = records.query([])?;
    while let Some(row) = rows.next()? {
        let text: String = row.get(4)?;
        if text.len() > 2 * 1024 * 1024 {
            return Err(bad("执行记录元数据过大"));
        }
        let r: Record = serde_json::from_str(&text)?;
        uuid(&r.id)?;
        uuid(&r.user)?;
        uuid(&r.scene)?;
        uuid(&r.agent)?;
        uuid(&r.connection_id)?;
        if row.get::<_, String>(0)? != r.id
            || row.get::<_, String>(1)? != r.scene
            || row.get::<_, i64>(2)? != number(r.created)?
            || row.get::<_, i64>(3)? != number(r.revision)?
            || r.revision == 0
            || r.connection_revision == 0
            || r.updated < r.created
        {
            return Err(bad("执行记录身份无效"));
        }
        let scene = state
            .scenes
            .iter()
            .find(|s| s.id == r.scene && s.agent_id == r.agent)
            .ok_or_else(|| bad("执行记录来源缺失"))?;
        if !scene.messages.iter().any(|m| {
            m.id == r.user && m.role == MessageRole::User && m.run_id.as_deref() == Some(&r.id)
        }) {
            return Err(bad("执行记录原始问题缺失"));
        }
        if r.kind == RecordedRunKind::Continuation && r.provenance.is_none() {
            return Err(bad("续答来源缺失"));
        }
        if let Some(p) = &r.provenance {
            if r.kind != RecordedRunKind::Continuation
                || p.source_run_id == r.id
                || p.projection_version != 1
                || p.source_revision == 0
                || !hash_valid(&p.projection_sha256)
                || p.selected_event_ids.is_empty()
                || p.selected_event_ids.len() > 64
            {
                return Err(bad("续答来源无效"));
            }
            uuid(&p.source_run_id)?;
            let mut selected = std::collections::HashSet::new();
            for id in &p.selected_event_ids {
                uuid(id)?;
                if !selected.insert(id) {
                    return Err(bad("续答证据重复"));
                }
            }
            let mut previous = None;
            for id in &p.original_message_ids {
                let position = scene
                    .messages
                    .iter()
                    .position(|m| m.id == *id)
                    .ok_or_else(|| bad("续答历史来源缺失"))?;
                if previous.is_some_and(|n| position <= n) {
                    return Err(bad("续答历史来源顺序无效"));
                }
                previous = Some(position);
            }
            let source: Record = serde_json::from_str(
                &row.get::<_, Option<String>>(5)?
                    .ok_or_else(|| bad("续答原始运行缺失"))?,
            )?;
            let end = scene
                .messages
                .iter()
                .position(|m| m.id == source.user)
                .ok_or_else(|| bad("续答原始问题缺失"))?;
            let range = crate::store::original_conversation_range(scene, end)?;
            let original = &scene.messages[range];
            if p.original_message_ids.len() != original.len()
                || !p
                    .original_message_ids
                    .iter()
                    .zip(original)
                    .all(|(id, m)| *id == m.id)
            {
                return Err(bad("续答必须保留原消息前缀"));
            }
        }
    }
    let mut query=db.prepare("SELECT e.id,e.sequence,e.metadata,e.content,r.data FROM agent_run_events e LEFT JOIN agent_run_records r ON r.id=e.run_id ORDER BY e.run_id,e.sequence")?;
    let mut rows = query.query([])?;
    let mut last = String::new();
    let mut seq = 0;
    let mut prefix = 0;
    let mut blocked = false;
    let mut expected = 0;
    while let Some(row) = rows.next()? {
        let r: Record = serde_json::from_str(
            &row.get::<_, Option<String>>(4)?
                .ok_or_else(|| bad("执行回执没有运行"))?,
        )?;
        if last != r.id {
            if !last.is_empty() && prefix != expected {
                return Err(bad("执行记录检查点不连续"));
            }
            last = r.id.clone();
            seq = 0;
            prefix = 0;
            blocked = false;
            expected = r.checkpoint;
        }
        seq += 1;
        let meta: String = row.get(2)?;
        let raw: String = row.get(3)?;
        if meta.len() > 65536 || raw.len() > 6 * TEXT_LIMIT + 4096 {
            return Err(bad("执行回执过大"));
        }
        let e: Event = serde_json::from_str(&meta)?;
        let body: JournalContent = serde_json::from_str(&raw)?;
        if e.projection_content_bytes != projection_content_bytes(&body)? {
            return Err(bad("执行正文投影长度不匹配"));
        }
        if let Some(binding) = &e.binding {
            validate_binding(binding)?;
        }
        validate_references(&e.references)?;
        uuid(&e.summary.id)?;
        uuid(&e.nonce)?;
        if row.get::<_, String>(0)? != e.summary.id
            || row.get::<_, i64>(1)? != seq
            || e.summary.sequence != seq as u64
            || seq > 64
            || e.summary.round >= 6
        {
            return Err(bad("执行回执序号无效"));
        }
        let pending = matches!(
            e.summary.phase,
            ReceiptPhase::Prepared | ReceiptPhase::Dispatched
        );
        if pending != e.summary.finished_at.is_none()
            || e.summary
                .finished_at
                .is_some_and(|n| n < e.summary.started_at)
            || (pending && r.status != RecordedRunStatus::Running)
        {
            return Err(bad("执行回执状态无效"));
        }
        blocked |= pending;
        if !blocked {
            prefix = seq as u64;
        }
        if e.summary.kind == ReceiptKind::Tool
            && (e.binding.is_none()
                || e.summary.tool_call_id.as_ref().is_none_or(|id| {
                    id.is_empty() || id.len() > 200 || id.chars().any(char::is_control)
                })
                || e.arguments_sha256.is_none())
        {
            return Err(bad("工具回执缺少身份"));
        }
        if e.summary.kind == ReceiptKind::Model
            && (e.binding.is_some()
                || e.summary.tool_call_id.is_some()
                || e.request_sha256.is_none())
        {
            return Err(bad("模型回执类型不匹配"));
        }
        if e.arguments_sha256.as_ref().is_some_and(|s| !hash_valid(s))
            || e.request_sha256.as_ref().is_some_and(|s| !hash_valid(s))
        {
            return Err(bad("执行记录摘要无效"));
        }
        if let Some(hash) = &e.summary.content_sha256 {
            if !hash_valid(hash)
                || *hash != digest(raw.as_bytes())
                || e.summary.content_bytes != raw.len() as u64
            {
                return Err(bad("执行记录正文摘要不匹配"));
            }
        } else if !matches!(
            body,
            JournalContent::Omitted {
                reason: ContentUnavailable::NotReceived
            }
        ) || e.summary.content_bytes != 0
        {
            return Err(bad("执行记录缺少正文摘要"));
        }
        if e.summary.phase == ReceiptPhase::Responded && e.summary.content_sha256.is_none() {
            return Err(bad("已返回回执缺少正文标记"));
        }
        let unavailable = match &body {
            JournalContent::Omitted { reason } => Some(reason.clone()),
            _ => None,
        };
        let selectable = matches!(&body,JournalContent::Text{text} if !text.trim().is_empty())
            || matches!(&body, JournalContent::Json { .. });
        if e.summary.content_unavailable != unavailable || e.summary.selectable != selectable {
            return Err(bad("执行正文投影标记无效"));
        }
        if e.summary.kind == ReceiptKind::Tool && e.summary.phase == ReceiptPhase::Responded {
            if e.summary.response_kind.is_none()
                || e.summary.observed_bytes.is_none()
                || e.summary
                    .observed_sha256
                    .as_ref()
                    .is_none_or(|s| !hash_valid(s))
                || e.summary
                    .observed_encoding
                    .as_ref()
                    .is_none_or(|s| s.is_empty() || s.len() > 100)
            {
                return Err(bad("工具响应缺少观察证据"));
            }
        }
        if e.summary.kind == ReceiptKind::Model
            && !matches!(
                body,
                JournalContent::Text { .. } | JournalContent::Omitted { .. }
            )
        {
            return Err(bad("模型记录不能含工具 JSON"));
        }
        if e.summary.kind == ReceiptKind::Tool && matches!(body, JournalContent::Text { .. }) {
            return Err(bad("工具记录类型无效"));
        }
        match body {
            JournalContent::Text { text } if text.len() > TEXT_LIMIT => {
                return Err(bad("模型记录正文过大"))
            }
            JournalContent::Json { value } if serde_json::to_vec(&value)?.len() > TOOL_LIMIT => {
                return Err(bad("工具记录正文过大"))
            }
            _ => {}
        }
    }
    if !last.is_empty() && prefix != expected {
        return Err(bad("执行记录检查点不连续"));
    }
    let bad_count:i64=db.query_row("SELECT count(*) FROM agent_run_records r WHERE json_extract(r.data,'$.checkpoint')>0 AND NOT EXISTS(SELECT 1 FROM agent_run_events e WHERE e.run_id=r.id)",[],|r|r.get(0))?;
    if bad_count != 0 {
        return Err(bad("空执行记录检查点无效"));
    }
    let excessive:i64=db.query_row("SELECT count(*) FROM (SELECT run_id FROM agent_run_events GROUP BY run_id HAVING sum(json_extract(metadata,'$.summary.kind')='model')>6 OR sum(json_extract(metadata,'$.summary.kind')='tool')>48)",[],|r|r.get(0))?;
    if excessive != 0 {
        return Err(bad("执行轮次超过预算"));
    }
    let invalid_provenance:i64=db.query_row("SELECT count(*) FROM agent_run_records r LEFT JOIN agent_run_records source ON source.id=json_extract(r.data,'$.provenance.sourceRunId') WHERE json_extract(r.data,'$.kind')='continuation' AND (source.id IS NULL OR source.scene_id!=r.scene_id OR json_extract(source.data,'$.agent')!=json_extract(r.data,'$.agent') OR json_extract(source.data,'$.kind')='continuation' OR json_extract(r.data,'$.provenance.sourceRevision')>source.revision OR json_extract(r.data,'$.provenance.checkpointSeq')>json_extract(source.data,'$.checkpoint'))",[],|r|r.get(0))?;
    if invalid_provenance != 0 {
        return Err(bad("续答原始运行身份不匹配"));
    }
    let foreign_event:i64=db.query_row("SELECT count(*) FROM agent_run_records r JOIN json_each(json_extract(r.data,'$.provenance.selectedEventIds')) selection LEFT JOIN agent_run_events e ON e.id=selection.value AND e.run_id=json_extract(r.data,'$.provenance.sourceRunId') WHERE e.id IS NULL OR json_extract(e.metadata,'$.summary.selectable')!=1",[],|r|r.get(0))?;
    if foreign_event != 0 {
        return Err(bad("续答证据不属于原始运行"));
    }
    Ok(())
}
