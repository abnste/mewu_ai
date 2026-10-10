// SPDX-License-Identifier: MPL-2.0
//! Durable external-memory ledger. The existing Store owns every transaction.
use crate::*;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64
}
pub(crate) fn queue_evidence(
    db: &Connection,
    b: &Binding,
    event: &str,
    source: EvidenceSource,
    text: Option<String>,
    blocked: Option<MemoryReason>,
) -> Result<WriteReceipt> {
    use sha2::{Digest, Sha256};
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM external_memory_evidence WHERE binding_id=?1 AND event_key=?2",
            params![b.id, event],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        let e = evidence(db, &id)?.ok_or(CoreError::MemoryWorkConflict)?;
        let hash = text
            .as_ref()
            .map(|t| format!("{:x}", Sha256::digest(t.as_bytes())));
        if e.hash != hash || encode(&e.source)? != encode(&source)? {
            return Err(invalid("同一记忆请求不能替换原文或来源"));
        }
        return write_receipt(db, &id);
    }
    // This is an active outbound-work budget, not a limit on historical audit
    // records. Retired/suspended routes and forgotten payloads cannot be sent.
    let pending:(i64,i64)=db.query_row("SELECT count(*),coalesce(sum(e.payload_bytes),0) FROM external_memory_outbox o JOIN external_memory_evidence e ON e.id=o.evidence_id JOIN external_memory_bindings b ON b.id=o.binding_id WHERE o.kind='retain' AND o.state IN('queued','dispatching','accepted','unknown') AND b.enabled=1 AND b.status='ready' AND e.state!='suppressed'",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let mut blocked = blocked;
    let bytes = text.as_ref().map_or(0, String::len);
    if bytes > MAX_PAYLOAD {
        blocked = Some(MemoryReason::EvidenceTooLarge)
    }
    if pending.0 >= MAX_PENDING || pending.1.saturating_add(bytes as i64) > MAX_PENDING_BYTES {
        blocked = Some(MemoryReason::QueueFull)
    }
    // Explicit calls return failure with input still owned by the caller. Only
    // completed-turn sync can retain a blocked source event without its payload.
    if blocked.is_some() && source.origin != EvidenceOrigin::CompletedTurn {
        return Err(invalid("记忆待处理队列已满或内容过大，尚未发送"));
    }
    let text = if blocked.is_none() { text } else { None };
    let hash = text
        .as_ref()
        .map(|t| format!("{:x}", Sha256::digest(t.as_bytes())));
    let id = Uuid::new_v4().to_string();
    let document_id = format!("mewu-evidence-{id}");
    let time = now_ms();
    let state = if blocked.is_some() {
        "blocked"
    } else {
        "queued"
    };
    db.execute("INSERT INTO external_memory_evidence(id,binding_id,revision,event_key,document_id,source_json,text,payload_hash,payload_bytes,state,reason_json,created_at,updated_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",params![id,b.id,event,document_id,encode(&source)?,text,hash,text.as_ref().map_or(0,String::len) as i64,state,blocked.as_ref().map(encode).transpose()?,integer(time)?])?;
    if blocked.is_none() {
        enqueue(
            db,
            &b.grant(),
            Some(&id),
            "retain",
            "retain",
            hash.as_deref(),
        )?;
    }
    write_receipt(db, &id)
}
pub(crate) fn enqueue(
    db: &Connection,
    grant: &MemoryAuthorityGrant,
    evidence: Option<&str>,
    kind: &str,
    action: &str,
    hash: Option<&str>,
) -> Result<String> {
    if let Some(evidence) = evidence {
        if let Some(id) = db
            .query_row(
                "SELECT id FROM external_memory_outbox WHERE evidence_id=?1 AND kind=?2",
                params![evidence, kind],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }
    }
    let id = Uuid::new_v4().to_string();
    db.execute("INSERT INTO external_memory_outbox(id,binding_id,evidence_id,revision,grant_json,kind,action,state,lease,payload_hash,next_attempt_at) VALUES(?1,?2,?3,1,?4,?5,?6,'queued',NULL,?7,0)",params![id,grant.binding_id,evidence,encode(grant)?,kind,action,hash])?;
    db.execute(
        "UPDATE external_memory_bindings SET ledger_revision=ledger_revision+1 WHERE id=?1",
        [&grant.binding_id],
    )?;
    Ok(id)
}
pub(crate) fn write_receipt(db: &Connection, id: &str) -> Result<WriteReceipt> {
    let e = evidence(db, id)?.ok_or_else(|| invalid("记忆证据不存在"))?;
    let operation_id = db
        .query_row(
            "SELECT id FROM external_memory_outbox WHERE evidence_id=?1 AND kind='retain'",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(WriteReceipt {
        evidence_id: e.id,
        revision: e.revision,
        operation_id,
        state: e.state,
        reason: e.reason,
    })
}
pub(crate) fn on_snapshot_change(db: &Connection, old: &Snapshot, next: &Snapshot) -> Result<()> {
    for agent in &old.agents {
        if agent.memory_enabled
            && !next
                .agents
                .iter()
                .any(|a| a.id == agent.id && a.memory_enabled)
        {
            let ids: Vec<String> = db
                .prepare("SELECT id FROM external_memory_bindings WHERE agent_id=?1")?
                .query_map([&agent.id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in ids {
                block_unclaimed(db, &id, MemoryReason::AuthorityChanged)?;
            }
        }
    }
    for scene in &next.scenes {
        let Some(run) = scene
            .run
            .as_ref()
            .filter(|r| r.status == RunStatus::Completed)
        else {
            continue;
        };
        if !old.scenes.iter().any(|s| {
            s.id == scene.id
                && s.run
                    .as_ref()
                    .is_some_and(|r| r.id == run.id && r.status == RunStatus::Running)
        }) {
            continue;
        }
        let Some(RunMemoryAuthority::External {
            selection_revision,
            grant,
            completed_turn_sync: true,
        }) = &run.memory_authority
        else {
            continue;
        };
        let b = binding(db, &grant.binding_id)?;
        if check_grant(&b, grant).is_err()
            || !usable(next, &b)
            || selection(next, &b.agent_id)?.revision != *selection_revision
        {
            continue;
        }
        let user = scene
            .messages
            .iter()
            .find(|m| m.role == MessageRole::User && m.run_id.as_deref() == Some(&run.id))
            .ok_or_else(|| invalid("记忆来源缺少用户消息"))?;
        let assistant = scene
            .messages
            .iter()
            .find(|m| m.role == MessageRole::Assistant && m.run_id.as_deref() == Some(&run.id))
            .ok_or_else(|| invalid("记忆来源缺少完成消息"))?;
        let oversized = user.text.len().saturating_add(assistant.text.len()) > MAX_PAYLOAD;
        let text = if oversized {
            None
        } else {
            Some(serde_json::to_string(
                &serde_json::json!({"version":1,"occurredAt":assistant.created_at,"parts":[{"role":"user","text":user.text},{"role":"assistant","text":assistant.text}]}),
            )?)
        };
        queue_evidence(
            db,
            &b,
            &format!("turn:{}", run.id),
            EvidenceSource {
                origin: EvidenceOrigin::CompletedTurn,
                scene_id: Some(scene.id.clone()),
                user_message_id: Some(user.id.clone()),
                assistant_message_id: Some(assistant.id.clone()),
                run_id: Some(run.id.clone()),
            },
            text,
            oversized.then_some(MemoryReason::EvidenceTooLarge),
        )?;
    }
    Ok(())
}
pub(crate) struct Operation {
    pub id: String,
    pub binding_id: String,
    pub evidence_id: Option<String>,
    pub revision: u64,
    pub grant: MemoryAuthorityGrant,
    pub kind: String,
    pub action: String,
    pub state: String,
    pub lease: Option<String>,
    pub hash: Option<String>,
    pub remote_state: Option<String>,
    pub due: u64,
    pub attempts: u64,
}
fn operation_row(r: &Row<'_>) -> rusqlite::Result<Operation> {
    Ok(Operation {
        id: r.get(0)?,
        binding_id: r.get(1)?,
        evidence_id: r.get(2)?,
        revision: unsigned(r, 3)?,
        grant: json(r.get(4)?, 4)?,
        kind: r.get(5)?,
        action: r.get(6)?,
        state: r.get(7)?,
        lease: r.get(8)?,
        hash: r.get(9)?,
        remote_state: r.get(10)?,
        due: unsigned(r, 11)?,
        attempts: unsigned(r, 12)?,
    })
}
const OP_COLS:&str="id,binding_id,evidence_id,revision,grant_json,kind,action,state,lease,payload_hash,remote_state,next_attempt_at,attempts";
pub(crate) fn operation(db: &Connection, id: &str) -> Result<Operation> {
    db.query_row(
        &format!("SELECT {OP_COLS} FROM external_memory_outbox WHERE id=?1"),
        [id],
        operation_row,
    )
    .optional()?
    .ok_or(CoreError::MemoryWorkConflict)
}
fn work_authorized(state: &Snapshot, b: &Binding, op: &Operation) -> bool {
    if !b.enabled || b.status == BindingStatus::Retired || b.status == BindingStatus::Suspended {
        return false;
    }
    if op.action == "create_bank" {
        return b.status == BindingStatus::Provisioning && op.grant == b.grant();
    }
    if op.action == "inspect_bank" {
        return b.status == BindingStatus::Provisioning;
    }
    if op.action == "retain" {
        usable(state, b)
            && op.grant == b.grant()
            && (b.policy.explicit_retain || b.policy.completed_turn_sync)
    } else {
        b.status == BindingStatus::Ready
            && state
                .agents
                .iter()
                .any(|a| a.id == b.agent_id && a.memory_enabled)
    }
}
pub(crate) fn lease_authorized(
    db: &Connection,
    state: &Snapshot,
    lease: &MemoryWorkLease,
    grant: &MemoryAuthorityGrant,
) -> Result<bool> {
    let op = operation(db, &lease.operation_id)?;
    if op.revision != lease.revision
        || op.lease.as_deref() != Some(&lease.nonce)
        || op.binding_id != lease.binding_id
        || op.action != lease.action
        || op.state != "dispatching"
    {
        return Ok(false);
    }
    let b = binding(db, &op.binding_id)?;
    if check_grant(&b, grant).is_err() || !work_authorized(state, &b, &op) {
        return Ok(false);
    }
    if op.action == "retain" {
        let Some(e) = op
            .evidence_id
            .as_deref()
            .map(|id| evidence(db, id))
            .transpose()?
            .flatten()
        else {
            return Ok(false);
        };
        if e.state == EvidenceState::Suppressed || forgotten(db, &e)?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) fn pending(
    db: &Connection,
    state: &Snapshot,
    time: u64,
    limit: usize,
) -> Result<Vec<PendingMemoryWork>> {
    if !(1..=64).contains(&limit) {
        return Err(invalid("后台记忆批量参数无效"));
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut query=db.prepare(&format!("SELECT {OP_COLS} FROM external_memory_outbox WHERE state IN('queued','accepted','unknown') AND lease IS NULL AND next_attempt_at<=?1 ORDER BY next_attempt_at,id"))?;
    for op in query.query_map([integer(time)?], operation_row)? {
        let op = op?;
        let b = binding(db, &op.binding_id)?;
        if work_authorized(state, &b, &op) && seen.insert(b.id.clone()) {
            result.push(PendingMemoryWork {
                operation_id: op.id,
                revision: op.revision,
                grant: b.grant(),
                next_attempt_at: op.due,
            });
            if result.len() == limit {
                break;
            }
        }
    }
    Ok(result)
}
pub(crate) fn claim(
    db: &Connection,
    state: &Snapshot,
    id: &str,
    expected: u64,
    grant: &MemoryAuthorityGrant,
    time: u64,
) -> Result<Option<MemoryDispatch>> {
    let op = operation(db, id)?;
    let b = binding(db, &op.binding_id)?;
    check_grant(&b, grant)?;
    if op.revision != expected {
        return Err(CoreError::MemoryWorkConflict);
    }
    if op.lease.is_some()
        || op.due > time
        || !["queued", "accepted", "unknown"].contains(&op.state.as_str())
        || !work_authorized(state, &b, &op)
    {
        return Ok(None);
    }
    let busy:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_outbox WHERE binding_id=?1 AND lease IS NOT NULL)",[&b.id],|r|r.get(0))?;
    if busy {
        return Ok(None);
    }
    let e = op
        .evidence_id
        .as_ref()
        .map(|id| evidence(db, id))
        .transpose()?
        .flatten();
    if let Some(e) = &e {
        if op.action == "retain"
            && (e.state == EvidenceState::Suppressed || forgotten(db, e)?.is_some())
        {
            return Ok(None);
        }
    }
    let action = match op.action.as_str() {
        "create_bank" => MemoryWorkAction::CreateBank,
        "inspect_bank" => MemoryWorkAction::InspectBank,
        "retain" => {
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            MemoryWorkAction::Retain {
                document_id: e.document_id.clone(),
                operation_id: op.id.clone(),
                attributed_text: e.text.clone().ok_or(CoreError::MemoryWorkConflict)?,
                payload_sha256: e.hash.clone().ok_or(CoreError::MemoryWorkConflict)?,
            }
        }
        "poll_retain" => {
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            MemoryWorkAction::PollRetain {
                document_id: e.document_id.clone(),
                operation_id: op.id.clone(),
            }
        }
        "cancel_retain" => {
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            let retain: String = db.query_row(
                "SELECT id FROM external_memory_outbox WHERE evidence_id=?1 AND kind='retain'",
                [&e.id],
                |r| r.get(0),
            )?;
            MemoryWorkAction::CancelRetain {
                document_id: e.document_id.clone(),
                operation_id: retain,
            }
        }
        "delete_document" => {
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            MemoryWorkAction::DeleteDocument {
                document_id: e.document_id.clone(),
            }
        }
        "inspect_document" => {
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            MemoryWorkAction::InspectDocument {
                document_id: e.document_id.clone(),
            }
        }
        _ => return Err(CoreError::MemoryWorkConflict),
    };
    let nonce = Uuid::new_v4().to_string();
    let revision = bump(op.revision)?;
    db.execute("UPDATE external_memory_outbox SET state='dispatching',lease=?1,revision=?2,attempts=attempts+1 WHERE id=?3",params![nonce,integer(revision)?,op.id])?;
    Ok(Some(MemoryDispatch {
        lease: MemoryWorkLease {
            operation_id: op.id,
            nonce,
            revision,
            binding_id: b.id.clone(),
            action: op.action,
        },
        grant: b.grant(),
        route: b.route(),
        action,
    }))
}
pub(crate) fn record(
    db: &Connection,
    lease: MemoryWorkLease,
    observation: MemoryWorkObservation,
    time: u64,
) -> Result<MemoryWorkReceipt> {
    let op = operation(db, &lease.operation_id)?;
    if op.revision != lease.revision
        || op.lease.as_deref() != Some(&lease.nonce)
        || op.binding_id != lease.binding_id
        || op.action != lease.action
    {
        return Err(CoreError::MemoryWorkConflict);
    }
    let b = binding(db, &op.binding_id)?;
    let e = op
        .evidence_id
        .as_ref()
        .map(|id| evidence(db, id))
        .transpose()?
        .flatten();
    let mut state = "unknown".to_owned();
    let mut action = op.action.clone();
    let mut remote = op.remote_state.clone();
    let mut reason = None;
    let mut evidence_state = None;
    let bank_ok = |id: &str| -> Result<()> {
        if id == b.bank_id {
            Ok(())
        } else {
            Err(CoreError::MemoryWorkConflict)
        }
    };
    let doc_ok = |id: &str| -> Result<()> {
        if e.as_ref().is_some_and(|e| e.document_id == id) {
            Ok(())
        } else {
            Err(CoreError::MemoryWorkConflict)
        }
    };
    let operation_ok = |id: &str| -> Result<()> {
        if op.kind == "retain" && id == op.id {
            return Ok(());
        }
        if op.kind == "delete" {
            let original: Option<String> = db
                .query_row(
                    "SELECT id FROM external_memory_outbox WHERE evidence_id=?1 AND kind='retain'",
                    [op.evidence_id.as_deref()],
                    |r| r.get(0),
                )
                .optional()?;
            if original.as_deref() == Some(id) {
                return Ok(());
            }
        }
        Err(CoreError::MemoryWorkConflict)
    };
    match observation {
        MemoryWorkObservation::BankReady {
            bank_id,
            service_version,
            observations_enabled,
        } => {
            bank_ok(&bank_id)?;
            if op.kind != "create"
                || !["create_bank", "inspect_bank"].contains(&op.action.as_str())
                || service_version != "0.10.3"
                || observations_enabled
            {
                return Err(CoreError::MemoryWorkConflict);
            }
            db.execute("UPDATE external_memory_bindings SET status=CASE WHEN status='provisioning' THEN 'ready' ELSE status END,ledger_revision=ledger_revision+1 WHERE id=?1",[&b.id])?;
            state = "completed".into();
        }
        MemoryWorkObservation::BankAbsent { bank_id } => {
            bank_ok(&bank_id)?;
            if op.action != "inspect_bank" {
                return Err(CoreError::MemoryWorkConflict);
            }
            action = "inspect_bank".into();
            reason = Some(MemoryReason::OutcomeUnknown);
        }
        MemoryWorkObservation::RetainAccepted {
            bank_id,
            operation_id,
        } => {
            bank_ok(&bank_id)?;
            operation_ok(&operation_id)?;
            if op.action != "retain" {
                return Err(CoreError::MemoryWorkConflict);
            }
            state = "accepted".into();
            action = "poll_retain".into();
            evidence_state = Some("accepted");
        }
        MemoryWorkObservation::Operation {
            bank_id,
            operation_id,
            state: observed,
        } => {
            bank_ok(&bank_id)?;
            operation_ok(&operation_id)?;
            if !["poll_retain", "cancel_retain"].contains(&op.action.as_str()) {
                return Err(CoreError::MemoryWorkConflict);
            }
            remote = Some(enum_text(&observed)?);
            if op.kind == "delete" {
                if observed == RemoteOperationState::Completed {
                    db.execute("UPDATE external_memory_outbox SET state='completed',remote_state='completed',revision=revision+1 WHERE evidence_id=?1 AND kind='retain' AND lease IS NULL",[op.evidence_id.as_deref()])?;
                }
                action = "delete_document".into();
                state = "queued".into();
            } else {
                action = "poll_retain".into();
                match observed {
                    RemoteOperationState::Completed => {
                        state = "completed".into();
                        evidence_state = Some("committed")
                    }
                    RemoteOperationState::Pending | RemoteOperationState::Processing => {
                        state = "accepted".into();
                        evidence_state = Some("accepted")
                    }
                    RemoteOperationState::Failed | RemoteOperationState::Cancelled => {
                        state = "failed".into();
                        evidence_state = Some("failed")
                    }
                    RemoteOperationState::NotFound => {
                        reason = Some(MemoryReason::OutcomeUnknown);
                        evidence_state = Some("unknown")
                    }
                }
            }
        }
        MemoryWorkObservation::CancelIntentObserved {
            bank_id,
            operation_id,
        } => {
            bank_ok(&bank_id)?;
            operation_ok(&operation_id)?;
            if op.action != "cancel_retain" {
                return Err(CoreError::MemoryWorkConflict);
            }
            state = "queued".into();
            action = "delete_document".into();
        }
        MemoryWorkObservation::DocumentDeleteObserved {
            bank_id,
            document_id,
            absent,
        } => {
            bank_ok(&bank_id)?;
            doc_ok(&document_id)?;
            if op.action != "delete_document" || op.kind != "delete" {
                return Err(CoreError::MemoryWorkConflict);
            }
            let _ = absent;
            state = "queued".into();
            action = "inspect_document".into();
        }
        MemoryWorkObservation::DocumentObserved {
            bank_id,
            document_id,
            present,
        } => {
            bank_ok(&bank_id)?;
            doc_ok(&document_id)?;
            if op.action != "inspect_document" || op.kind != "delete" {
                return Err(CoreError::MemoryWorkConflict);
            }
            let e = e.as_ref().ok_or(CoreError::MemoryWorkConflict)?;
            let quiet:bool=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM external_memory_outbox WHERE evidence_id=?1 AND kind='retain' AND NOT(state='completed' AND remote_state='completed') AND NOT(action='retain' AND state='blocked' AND lease IS NULL AND remote_state IS NULL))",[&e.id],|r|r.get(0))?;
            let (deletion, quiescence) = if !present && quiet {
                ("confirmed_deleted", "proven")
            } else if !b.enabled
                || b.status == BindingStatus::Retired
                || b.status == BindingStatus::Suspended
            {
                ("needs_authorization", "unproven")
            } else if !present {
                ("currently_absent", "unproven")
            } else {
                ("pending", "unproven")
            };
            db.execute("UPDATE external_memory_tombstones SET remote_state=?1,quiescence=?2 WHERE evidence_id=?3",params![deletion,quiescence,e.id])?;
            if !present {
                state = if quiet { "completed" } else { "unknown" }.into()
            } else {
                state = "queued".into();
                action = "delete_document".into()
            }
        }
        MemoryWorkObservation::NotDispatched { reason: r } => {
            reason = Some(r);
            // Definitive no-dispatch is still not an automatic paid retry.
            // A skipped read cannot undo an earlier POST's uncertainty.
            state = if op.kind == "delete"
                || ["inspect_bank", "poll_retain", "inspect_document"].contains(&op.action.as_str())
            {
                "unknown"
            } else {
                "blocked"
            }
            .into();
            if op.action == "retain" {
                evidence_state = Some("blocked")
            }
        }
        MemoryWorkObservation::MutationOutcomeUnknown { reason: r } => {
            reason = Some(r);
            action = match op.action.as_str() {
                "create_bank" => "inspect_bank",
                "retain" => "poll_retain",
                "cancel_retain" => "delete_document",
                "delete_document" => "inspect_document",
                _ => return Err(CoreError::MemoryWorkConflict),
            }
            .into();
            if op.kind == "retain" {
                evidence_state = Some("unknown")
            }
        }
        MemoryWorkObservation::ReadUnavailable { reason: r } => {
            if !["inspect_bank", "poll_retain", "inspect_document"].contains(&op.action.as_str()) {
                return Err(CoreError::MemoryWorkConflict);
            }
            reason = Some(r);
        }
    }
    let revision = bump(op.revision)?;
    let delay = if state == "queued" {
        0
    } else {
        (1u64 << op.attempts.min(4)).min(15) * 1000
    };
    db.execute("UPDATE external_memory_outbox SET state=?1,action=?2,lease=NULL,revision=?3,remote_state=?4,reason_json=?5,next_attempt_at=?6 WHERE id=?7",params![state,action,integer(revision)?,remote,reason.as_ref().map(encode).transpose()?,integer(time.saturating_add(delay))?,op.id])?;
    if let (Some(e), Some(status)) = (&e, evidence_state) {
        db.execute("UPDATE external_memory_evidence SET state=?1,reason_json=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND state!='suppressed'",params![status,reason.as_ref().map(encode).transpose()?,integer(now_ms())?,e.id])?;
    }
    db.execute(
        "UPDATE external_memory_bindings SET ledger_revision=ledger_revision+1 WHERE id=?1",
        [&b.id],
    )?;
    let e = op
        .evidence_id
        .as_ref()
        .map(|id| evidence(db, id))
        .transpose()?
        .flatten();
    Ok(MemoryWorkReceipt {
        operation_id: op.id,
        operation_revision: revision,
        evidence: e.as_ref().map(|e| write_receipt(db, &e.id)).transpose()?,
        deletion: e.as_ref().map(|e| forgotten(db, e)).transpose()?.flatten(),
    })
}
pub(crate) fn recover(db: &Connection) -> Result<MemoryRecoverySummary> {
    let n=db.execute("UPDATE external_memory_outbox SET state='unknown',action=CASE action WHEN 'create_bank' THEN 'inspect_bank' WHEN 'retain' THEN 'poll_retain' WHEN 'cancel_retain' THEN 'delete_document' WHEN 'delete_document' THEN 'inspect_document' ELSE action END,lease=NULL,revision=revision+1,next_attempt_at=0 WHERE state='dispatching' OR lease IS NOT NULL",[])?;
    db.execute("UPDATE external_memory_evidence SET state='unknown',revision=revision+1 WHERE state IN('queued','accepted') AND id IN(SELECT evidence_id FROM external_memory_outbox WHERE kind='retain' AND state='unknown')",[])?;
    Ok(MemoryRecoverySummary {
        uncertain_operations: n as u64,
        blocked_operations: 0,
    })
}

type Result<T> = std::result::Result<T, CoreError>;
pub(crate) const MAX_PAYLOAD: usize = 64 * 1024;
pub(crate) const MAX_PENDING: i64 = 128;
pub(crate) const MAX_PENDING_BYTES: i64 = 8 * 1024 * 1024;
const BIND_COLS: &str = "id,agent_id,revision,ledger_revision,plugin_id,contribution_id,plugin_revision,endpoint,credential_id,bank_id,enabled,status,policy_json,created_at";
pub(crate) fn invalid(s: &str) -> CoreError {
    CoreError::Invalid(s.into())
}
pub(crate) fn uuid(value: &str) -> Result<()> {
    if Uuid::parse_str(value).is_ok_and(|u| u.to_string() == value) {
        Ok(())
    } else {
        Err(invalid("记忆标识无效"))
    }
}
pub(crate) fn integer(n: u64) -> Result<i64> {
    i64::try_from(n).map_err(|_| invalid("记忆版本超出范围"))
}
pub(crate) fn bump(n: u64) -> Result<u64> {
    n.checked_add(1)
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| invalid("记忆版本已达上限"))
}
fn json<T: DeserializeOwned>(raw: String, column: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(e))
    })
}
pub(crate) fn encode<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn enum_text<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("记忆状态无效"))
}
fn parse_enum<T: DeserializeOwned>(text: String, column: usize) -> rusqlite::Result<T> {
    json(serde_json::Value::String(text).to_string(), column)
}
fn unsigned(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let n: i64 = row.get(index)?;
    u64::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, n))
}
pub(crate) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE external_memory_bindings(
        id TEXT PRIMARY KEY,agent_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),ledger_revision INTEGER NOT NULL CHECK(ledger_revision>=0),
        plugin_id TEXT NOT NULL,contribution_id TEXT NOT NULL,plugin_revision INTEGER NOT NULL CHECK(plugin_revision>0),
        endpoint TEXT NOT NULL,credential_id TEXT,bank_id TEXT NOT NULL UNIQUE,enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),
        status TEXT NOT NULL CHECK(status IN('provisioning','ready','suspended','retired')),policy_json TEXT NOT NULL,created_at INTEGER NOT NULL);
      CREATE INDEX external_bindings_agent ON external_memory_bindings(agent_id,created_at DESC,id);
      CREATE UNIQUE INDEX external_one_provisioning ON external_memory_bindings(agent_id) WHERE status='provisioning';
      CREATE TABLE external_memory_evidence(
        id TEXT PRIMARY KEY,binding_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),event_key TEXT NOT NULL,
        document_id TEXT NOT NULL,source_json TEXT NOT NULL,text TEXT,payload_hash TEXT,payload_bytes INTEGER NOT NULL CHECK(payload_bytes>=0),
        state TEXT NOT NULL CHECK(state IN('queued','accepted','committed','blocked','unknown','failed','suppressed')),reason_json TEXT,
        created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(binding_id,event_key),UNIQUE(binding_id,document_id));
      CREATE INDEX external_evidence_order ON external_memory_evidence(binding_id,created_at DESC,id);
      CREATE INDEX external_evidence_state ON external_memory_evidence(binding_id,state);
      CREATE TABLE external_memory_outbox(
        id TEXT PRIMARY KEY,binding_id TEXT NOT NULL,evidence_id TEXT,revision INTEGER NOT NULL CHECK(revision>0),
        grant_json TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN('create','retain','delete')),
        action TEXT NOT NULL CHECK(action IN('create_bank','inspect_bank','retain','poll_retain','cancel_retain','delete_document','inspect_document')),
        state TEXT NOT NULL CHECK(state IN('queued','dispatching','accepted','unknown','completed','failed','blocked')),
        lease TEXT,payload_hash TEXT,remote_state TEXT,next_attempt_at INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,reason_json TEXT,
        UNIQUE(evidence_id,kind));
      CREATE INDEX external_work_due ON external_memory_outbox(state,next_attempt_at,id);
      CREATE UNIQUE INDEX external_one_worker ON external_memory_outbox(binding_id) WHERE lease IS NOT NULL;
      CREATE TABLE external_memory_tombstones(
        evidence_id TEXT PRIMARY KEY,binding_id TEXT NOT NULL,document_id TEXT NOT NULL,created_at INTEGER NOT NULL,
        remote_state TEXT NOT NULL CHECK(remote_state IN('pending','currently_absent','confirmed_deleted','unknown','failed','needs_authorization')),
        quiescence TEXT NOT NULL CHECK(quiescence IN('proven','unproven')));
      CREATE TRIGGER external_evidence_insert AFTER INSERT ON external_memory_evidence BEGIN
        UPDATE external_memory_bindings SET ledger_revision=ledger_revision+1 WHERE id=new.binding_id;
      END;
      CREATE TRIGGER external_evidence_update AFTER UPDATE ON external_memory_evidence BEGIN
        UPDATE external_memory_bindings SET ledger_revision=ledger_revision+1 WHERE id=new.binding_id;
      END;")?;
    Ok(())
}
#[derive(Clone)]
pub(crate) struct Binding {
    pub id: String,
    pub agent_id: String,
    pub revision: u64,
    pub ledger_revision: u64,
    pub plugin_id: String,
    pub contribution_id: String,
    pub plugin_revision: u64,
    pub endpoint: String,
    pub credential_id: Option<String>,
    pub bank_id: String,
    pub enabled: bool,
    pub status: BindingStatus,
    pub policy: MemoryPolicy,
    pub created_at: u64,
}
impl Binding {
    pub fn grant(&self) -> MemoryAuthorityGrant {
        MemoryAuthorityGrant {
            binding_id: self.id.clone(),
            binding_revision: self.revision,
            plugin_id: self.plugin_id.clone(),
            contribution_id: self.contribution_id.clone(),
            plugin_revision: self.plugin_revision,
        }
    }
    pub fn route(&self) -> MemoryRoute {
        MemoryRoute {
            binding_id: self.id.clone(),
            endpoint: self.endpoint.clone(),
            bank_id: self.bank_id.clone(),
            credential_id: self.credential_id.clone(),
            adapter_contract_version: 1,
        }
    }
    pub fn view(&self, db: &Connection) -> Result<MemoryBindingView> {
        let (evidence_count,blocked_count):(u64,u64)=db.query_row("SELECT count(*),coalesce(sum(state='blocked'),0) FROM external_memory_evidence WHERE binding_id=?1",[&self.id],|r|Ok((unsigned(r,0)?,unsigned(r,1)?)))?;
        let pending_count:u64=db.query_row("SELECT count(*) FROM external_memory_outbox WHERE binding_id=?1 AND state IN('queued','dispatching','accepted','unknown')",[&self.id],|r|unsigned(r,0))?;
        let deletion_pending_count:u64=db.query_row("SELECT count(*) FROM external_memory_tombstones WHERE binding_id=?1 AND remote_state NOT IN('confirmed_deleted')",[&self.id],|r|unsigned(r,0))?;
        Ok(MemoryBindingView {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            revision: self.revision,
            ledger_revision: self.ledger_revision,
            provider_id: "hindsight".into(),
            plugin_id: self.plugin_id.clone(),
            contribution_id: self.contribution_id.clone(),
            plugin_revision: self.plugin_revision,
            endpoint: self.endpoint.clone(),
            enabled: self.enabled,
            status: self.status.clone(),
            policy: self.policy.clone(),
            configured: self.credential_id.is_some(),
            evidence_count,
            pending_count,
            blocked_count,
            deletion_pending_count,
        })
    }
}
fn bind_row(r: &Row<'_>) -> rusqlite::Result<Binding> {
    Ok(Binding {
        id: r.get(0)?,
        agent_id: r.get(1)?,
        revision: unsigned(r, 2)?,
        ledger_revision: unsigned(r, 3)?,
        plugin_id: r.get(4)?,
        contribution_id: r.get(5)?,
        plugin_revision: unsigned(r, 6)?,
        endpoint: r.get(7)?,
        credential_id: r.get(8)?,
        bank_id: r.get(9)?,
        enabled: r.get(10)?,
        status: parse_enum(r.get(11)?, 11)?,
        policy: json(r.get(12)?, 12)?,
        created_at: unsigned(r, 13)?,
    })
}
pub(crate) fn binding(db: &Connection, id: &str) -> Result<Binding> {
    db.query_row(
        &format!("SELECT {BIND_COLS} FROM external_memory_bindings WHERE id=?1"),
        [id],
        bind_row,
    )
    .optional()?
    .ok_or_else(|| CoreError::NotFound {
        kind: "记忆来源",
        id: id.into(),
    })
}
pub(crate) fn selection(state: &Snapshot, agent_id: &str) -> Result<MemoryProviderSelection> {
    state
        .agents
        .iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.memory_provider.clone())
        .ok_or_else(|| CoreError::NotFound {
            kind: "Agent",
            id: agent_id.into(),
        })
}
pub(crate) fn agent_enabled(state: &Snapshot, b: &Binding) -> bool {
    state.agents.iter().any(|a| {
        a.id == b.agent_id
            && a.memory_enabled
            && a.memory_provider.binding_id.as_deref() == Some(&b.id)
    })
}
pub(crate) fn check_grant(b: &Binding, grant: &MemoryAuthorityGrant) -> Result<()> {
    if &b.grant() != grant {
        return Err(CoreError::MemoryProviderConflict);
    }
    Ok(())
}
pub(crate) fn usable(state: &Snapshot, b: &Binding) -> bool {
    b.enabled && b.status == BindingStatus::Ready && agent_enabled(state, b)
}
pub(crate) fn capture_authority(
    db: &Connection,
    state: &Snapshot,
    scene_id: &str,
    grant: Option<&MemoryAuthorityGrant>,
) -> Result<Option<RunMemoryAuthority>> {
    let scene = state
        .scenes
        .iter()
        .find(|s| s.id == scene_id)
        .ok_or(CoreError::StaleRun)?;
    let agent = state
        .agents
        .iter()
        .find(|a| a.id == scene.agent_id)
        .ok_or(CoreError::StaleRun)?;
    if !agent.memory_enabled {
        return Ok(None);
    }
    if let Some(id) = &agent.memory_provider.binding_id {
        let Some(grant) = grant else { return Ok(None) };
        let b = binding(db, id)?;
        check_grant(&b, grant)?;
        if !usable(state, &b) {
            return Err(CoreError::MemoryDisabled);
        }
        Ok(Some(RunMemoryAuthority::External {
            selection_revision: agent.memory_provider.revision,
            grant: grant.clone(),
            completed_turn_sync: b.policy.completed_turn_sync,
        }))
    } else {
        if grant.is_some() {
            return Err(CoreError::MemoryProviderConflict);
        }
        Ok(Some(RunMemoryAuthority::Local {
            selection_revision: agent.memory_provider.revision,
        }))
    }
}
pub(crate) fn run_binding(
    db: &Connection,
    state: &Snapshot,
    scene_id: &str,
    run_id: &str,
    grant: &MemoryAuthorityGrant,
) -> Result<Binding> {
    let scene = state
        .scenes
        .iter()
        .find(|s| s.id == scene_id && !s.closed)
        .ok_or(CoreError::StaleRun)?;
    let run = scene
        .run
        .as_ref()
        .filter(|r| r.id == run_id && r.status == RunStatus::Running)
        .ok_or(CoreError::StaleRun)?;
    let Some(RunMemoryAuthority::External {
        selection_revision,
        grant: original,
        ..
    }) = &run.memory_authority
    else {
        return Err(CoreError::MemoryDisabled);
    };
    let b = binding(db, &grant.binding_id)?;
    check_grant(&b, grant)?;
    if original != grant
        || b.agent_id != scene.agent_id
        || selection(state, &b.agent_id)?.revision != *selection_revision
        || !usable(state, &b)
    {
        return Err(CoreError::MemoryProviderConflict);
    }
    Ok(b)
}
pub(crate) fn block_unclaimed(
    db: &Connection,
    binding_id: &str,
    reason: MemoryReason,
) -> Result<()> {
    let ids:Vec<String>=db.prepare("SELECT evidence_id FROM external_memory_outbox WHERE binding_id=?1 AND action='retain' AND state='queued' AND lease IS NULL")?.query_map([binding_id],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for id in ids {
        db.execute("UPDATE external_memory_evidence SET state='blocked',reason_json=?1,revision=revision+1 WHERE id=?2 AND state!='suppressed'",params![encode(&reason)?,id])?;
    }
    db.execute("UPDATE external_memory_outbox SET state='blocked',reason_json=?1,revision=revision+1 WHERE binding_id=?2 AND action IN('retain','create_bank') AND state='queued' AND lease IS NULL",params![encode(&reason)?,binding_id])?;
    Ok(())
}
pub(crate) fn validate(db: &Connection, state: &Snapshot) -> Result<()> {
    // Check byte budgets before hydrating strings/JSON from a damaged database.
    let oversized:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_bindings WHERE length(CAST(endpoint AS BLOB))>4096 OR length(policy_json)>512 OR length(plugin_id)>200 OR length(contribution_id)>200) OR EXISTS(SELECT 1 FROM external_memory_evidence WHERE length(CAST(text AS BLOB))>65536 OR length(source_json)>4096 OR length(event_key)>200 OR length(reason_json)>200) OR EXISTS(SELECT 1 FROM external_memory_outbox WHERE length(grant_json)>2048 OR length(reason_json)>200)",[],|r|r.get(0))?;
    if oversized {
        return Err(invalid("记忆账本字段超过预算"));
    }
    let mut bindings = db.prepare(&format!("SELECT {BIND_COLS} FROM external_memory_bindings"))?;
    for b in bindings.query_map([], bind_row)? {
        let b = b?;
        uuid(&b.id)?;
        integer(b.created_at)?;
        if b.revision == 0
            || b.plugin_revision == 0
            || !state.agents.iter().any(|a| a.id == b.agent_id)
            || b.bank_id != format!("mewu-{}", Uuid::parse_str(&b.id).unwrap().simple())
        {
            return Err(invalid("记忆来源账本无效"));
        }
        if let Some(id) = &b.credential_id {
            uuid(id)?
        }
        if b.endpoint.is_empty()
            || b.endpoint.len() > 4096
            || b.endpoint.chars().any(char::is_control)
            || b.plugin_id.is_empty()
            || b.contribution_id.is_empty()
            || b.plugin_id.len() > 200
            || b.contribution_id.len() > 200
            || b.plugin_id.chars().any(char::is_control)
            || b.contribution_id.chars().any(char::is_control)
            || b.endpoint.trim() != b.endpoint
            || b.status == BindingStatus::Retired && b.enabled
        {
            return Err(invalid("记忆来源配置无效"));
        }
    }
    for agent in &state.agents {
        integer(agent.memory_provider.revision)?;
        if let Some(id) = &agent.memory_provider.binding_id {
            let b = binding(db, id)?;
            if b.agent_id != agent.id
                || b.status == BindingStatus::Retired
                || agent.memory_provider.revision == 0
            {
                return Err(invalid("Agent记忆来源不存在"));
            }
        }
    }
    let bad:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_evidence e WHERE NOT EXISTS(SELECT 1 FROM external_memory_bindings b WHERE b.id=e.binding_id)) OR EXISTS(SELECT 1 FROM external_memory_outbox o WHERE NOT EXISTS(SELECT 1 FROM external_memory_bindings b WHERE b.id=o.binding_id) OR (o.evidence_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM external_memory_evidence e WHERE e.id=o.evidence_id AND e.binding_id=o.binding_id))) OR EXISTS(SELECT 1 FROM external_memory_tombstones t WHERE NOT EXISTS(SELECT 1 FROM external_memory_evidence e WHERE e.id=t.evidence_id AND e.binding_id=t.binding_id AND e.document_id=t.document_id AND e.state='suppressed'))",[],|r|r.get(0))?;
    if bad {
        return Err(invalid("外接记忆账本引用不一致"));
    }
    let inconsistent:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_evidence e WHERE payload_bytes!=coalesce(length(CAST(text AS BLOB)),0) OR payload_bytes>65536 OR (state='suppressed' AND NOT EXISTS(SELECT 1 FROM external_memory_tombstones t WHERE t.evidence_id=e.id)) OR (text IS NULL AND state NOT IN('blocked','suppressed'))) OR EXISTS(SELECT 1 FROM external_memory_tombstones WHERE (remote_state='confirmed_deleted') != (quiescence='proven') OR created_at<0) OR EXISTS(SELECT 1 FROM external_memory_outbox o WHERE o.kind='delete' AND NOT EXISTS(SELECT 1 FROM external_memory_tombstones t WHERE t.evidence_id=o.evidence_id))",[],|r|r.get(0))?;
    if inconsistent {
        return Err(invalid("记忆证据或遗忘记录不一致"));
    }
    let scenes: HashMap<_, _> = state.scenes.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut statement = db.prepare(&format!("SELECT {},b.agent_id FROM external_memory_evidence e JOIN external_memory_bindings b ON b.id=e.binding_id",EVID_COLS.split(',').map(|c|format!("e.{c}")).collect::<Vec<_>>().join(",")))?;
    for e in statement.query_map([], |r| Ok((evidence_row(r)?, r.get::<_, String>(11)?)))? {
        let (e, agent) = e?;
        uuid(&e.id)?;
        if e.text.as_ref().is_some_and(|t| t.len() > MAX_PAYLOAD)
            || e.document_id != format!("mewu-evidence-{}", e.id)
            || e.revision == 0
            || e.updated_at < e.created_at
            || e.state == EvidenceState::Suppressed && e.text.is_some()
        {
            return Err(invalid("记忆证据无效"));
        }
        if let Some(hash) = &e.hash {
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            {
                return Err(invalid("记忆证据摘要无效"));
            }
        }
        let source = &e.source;
        match source.origin {
            EvidenceOrigin::Manual => {
                if source.scene_id.is_some()
                    || source.user_message_id.is_some()
                    || source.assistant_message_id.is_some()
                    || source.run_id.is_some()
                {
                    return Err(invalid("手工记忆来源无效"));
                }
            }
            EvidenceOrigin::UserQuote | EvidenceOrigin::CompletedTurn => {
                let origin = source
                    .scene_id
                    .as_deref()
                    .and_then(|id| scenes.get(id))
                    .filter(|s| s.agent_id == agent)
                    .ok_or_else(|| invalid("记忆来源会话不存在"))?;
                let run = source
                    .run_id
                    .as_deref()
                    .ok_or_else(|| invalid("记忆来源运行不存在"))?;
                let user = origin.messages.iter().any(|m| {
                    Some(m.id.as_str()) == source.user_message_id.as_deref()
                        && m.role == MessageRole::User
                        && m.run_id.as_deref() == Some(run)
                });
                let assistant = origin.messages.iter().any(|m| {
                    Some(m.id.as_str()) == source.assistant_message_id.as_deref()
                        && m.role == MessageRole::Assistant
                        && m.run_id.as_deref() == Some(run)
                });
                if !user
                    || (source.origin == EvidenceOrigin::CompletedTurn && !assistant)
                    || (source.origin == EvidenceOrigin::UserQuote
                        && source.assistant_message_id.is_some())
                {
                    return Err(invalid("记忆来源消息不一致"));
                }
            }
        }
        if let Some(text) = &e.text {
            use sha2::{Digest, Sha256};
            if e.hash.as_deref() != Some(format!("{:x}", Sha256::digest(text.as_bytes())).as_str())
            {
                return Err(invalid("记忆证据摘要不匹配"));
            }
        }
    }
    let mut operations=db.prepare(&format!("SELECT {},b.plugin_id,b.contribution_id,b.revision,b.plugin_revision,e.payload_hash FROM external_memory_outbox o JOIN external_memory_bindings b ON b.id=o.binding_id LEFT JOIN external_memory_evidence e ON e.id=o.evidence_id",OP_COLS.split(',').map(|c|format!("o.{c}")).collect::<Vec<_>>().join(",")))?;
    for row in operations.query_map([], |r| {
        Ok((
            operation_row(r)?,
            r.get::<_, String>(13)?,
            r.get::<_, String>(14)?,
            unsigned(r, 15)?,
            unsigned(r, 16)?,
            r.get::<_, Option<String>>(17)?,
        ))
    })? {
        let (op, plugin, contribution, revision, _plugin_revision, hash) = row?;
        uuid(&op.id)?;
        if let Some(lease) = &op.lease {
            uuid(lease)?;
        }
        let valid_kind = match op.kind.as_str() {
            "create" => {
                op.evidence_id.is_none()
                    && op.hash.is_none()
                    && ["create_bank", "inspect_bank"].contains(&op.action.as_str())
            }
            "retain" => {
                op.evidence_id.is_some()
                    && op.hash.is_some()
                    && op.hash == hash
                    && ["retain", "poll_retain"].contains(&op.action.as_str())
            }
            "delete" => {
                op.evidence_id.is_some()
                    && op.hash.is_none()
                    && ["cancel_retain", "delete_document", "inspect_document"]
                        .contains(&op.action.as_str())
            }
            _ => false,
        };
        if !valid_kind
            || op.revision == 0
            || (op.state == "dispatching") != op.lease.is_some()
            || op.grant.binding_id != op.binding_id
            || op.grant.plugin_id != plugin
            || op.grant.contribution_id != contribution
            || op.grant.binding_revision == 0
            || op.grant.binding_revision > revision
            || op.grant.plugin_revision == 0
        {
            return Err(invalid("记忆后台账本状态无效"));
        }
        if let Some(remote) = &op.remote_state {
            let _: RemoteOperationState =
                serde_json::from_value(serde_json::Value::String(remote.clone()))?;
        }
    }
    for scene in &state.scenes {
        if let Some(authority) = scene.run.as_ref().and_then(|r| r.memory_authority.as_ref()) {
            match authority {
                RunMemoryAuthority::Local { selection_revision } => {
                    integer(*selection_revision)?;
                }
                RunMemoryAuthority::External {
                    selection_revision,
                    grant,
                    ..
                } => {
                    integer(*selection_revision)?;
                    let b = binding(db, &grant.binding_id)?;
                    if b.agent_id != scene.agent_id
                        || grant.plugin_id != b.plugin_id
                        || grant.contribution_id != b.contribution_id
                        || grant.binding_revision == 0
                        || grant.binding_revision > b.revision
                        || grant.plugin_revision == 0
                    {
                        return Err(invalid("运行记忆授权来源不一致"));
                    }
                }
            }
        }
    }
    Ok(())
}
pub(crate) struct Evidence {
    pub id: String,
    pub binding_id: String,
    pub revision: u64,
    pub document_id: String,
    pub source: EvidenceSource,
    pub text: Option<String>,
    pub hash: Option<String>,
    pub state: EvidenceState,
    pub reason: Option<MemoryReason>,
    pub created_at: u64,
    pub updated_at: u64,
}
fn evidence_row(r: &Row<'_>) -> rusqlite::Result<Evidence> {
    Ok(Evidence {
        id: r.get(0)?,
        binding_id: r.get(1)?,
        revision: unsigned(r, 2)?,
        document_id: r.get(3)?,
        source: json(r.get(4)?, 4)?,
        text: r.get(5)?,
        hash: r.get(6)?,
        state: parse_enum(r.get(7)?, 7)?,
        reason: r
            .get::<_, Option<String>>(8)?
            .map(|s| json(s, 8))
            .transpose()?,
        created_at: unsigned(r, 9)?,
        updated_at: unsigned(r, 10)?,
    })
}
const EVID_COLS:&str="id,binding_id,revision,document_id,source_json,text,payload_hash,state,reason_json,created_at,updated_at";
pub(crate) fn evidence(db: &Connection, id: &str) -> Result<Option<Evidence>> {
    Ok(db
        .query_row(
            &format!("SELECT {EVID_COLS} FROM external_memory_evidence WHERE id=?1"),
            [id],
            evidence_row,
        )
        .optional()?)
}
pub(crate) fn forgotten(db: &Connection, e: &Evidence) -> Result<Option<ForgetReceipt>> {
    Ok(db
        .query_row(
            "SELECT remote_state,quiescence FROM external_memory_tombstones WHERE evidence_id=?1",
            [&e.id],
            |r| {
                Ok(ForgetReceipt {
                    evidence_id: e.id.clone(),
                    revision: e.revision,
                    local_suppressed: true,
                    remote_state: parse_enum(r.get(0)?, 0)?,
                    quiescence: parse_enum(r.get(1)?, 1)?,
                })
            },
        )
        .optional()?)
}
fn evidence_view(db: &Connection, e: &Evidence) -> Result<MemoryEvidenceView> {
    Ok(MemoryEvidenceView {
        id: e.id.clone(),
        binding_id: e.binding_id.clone(),
        revision: e.revision,
        source: e.source.clone(),
        preview: e
            .text
            .as_ref()
            .map(|t| t.chars().take(800).collect())
            .unwrap_or_default(),
        state: e.state.clone(),
        reason: e.reason.clone(),
        forgotten: forgotten(db, e)?,
        created_at: e.created_at,
        updated_at: e.updated_at,
    })
}
pub(crate) fn entry(
    db: &Connection,
    state: &Snapshot,
    agent: &str,
    binding_id: &str,
    id: &str,
) -> Result<Option<MemoryEvidenceDetail>> {
    let b = binding(db, binding_id)?;
    selection(state, agent)?;
    if b.agent_id != agent {
        return Err(CoreError::MemoryProviderConflict);
    }
    let Some(e) = evidence(db, id)?.filter(|e| e.binding_id == binding_id) else {
        return Ok(None);
    };
    Ok(Some(MemoryEvidenceDetail {
        summary: evidence_view(db, &e)?,
        attributed_text: e.text,
    }))
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageCursor {
    agent: String,
    binding: String,
    query: String,
    revision: u64,
    offset: u64,
}
pub(crate) fn page(
    db: &Connection,
    state: &Snapshot,
    agent: &str,
    binding_id: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<MemoryEvidencePage> {
    if !(1..=100).contains(&limit) || query.chars().count() > 200 {
        return Err(invalid("记忆分页参数无效"));
    }
    let b = binding(db, binding_id)?;
    selection(state, agent)?;
    if b.agent_id != agent {
        return Err(CoreError::MemoryProviderConflict);
    }
    let query = query.trim().to_string();
    let offset = if let Some(raw) = cursor {
        if raw.len() > 4096 {
            return Err(invalid("记忆分页游标无效"));
        }
        let c: PageCursor = serde_json::from_str(raw)?;
        if c.agent != agent
            || c.binding != binding_id
            || c.query != query
            || c.revision != b.ledger_revision
        {
            return Err(CoreError::MemoryProviderConflict);
        }
        c.offset
    } else {
        0
    };
    let total:u64=db.query_row("SELECT count(*) FROM external_memory_evidence WHERE binding_id=?1 AND (?2='' OR instr(lower(coalesce(text,'')),lower(?2))>0)",params![binding_id,query],|r|unsigned(r,0))?;
    let es:Vec<Evidence>=db.prepare(&format!("SELECT {EVID_COLS} FROM external_memory_evidence WHERE binding_id=?1 AND (?2='' OR instr(lower(coalesce(text,'')),lower(?2))>0) ORDER BY created_at DESC,id LIMIT ?3 OFFSET ?4"))?.query_map(params![binding_id,query,limit as i64,integer(offset)?],evidence_row)?.collect::<rusqlite::Result<_>>()?;
    let entries = es
        .iter()
        .map(|e| evidence_view(db, e))
        .collect::<Result<Vec<_>>>()?;
    let next = offset + entries.len() as u64;
    Ok(MemoryEvidencePage {
        entries,
        total,
        revision: b.ledger_revision,
        next_cursor: (next < total)
            .then(|| {
                encode(&PageCursor {
                    agent: agent.into(),
                    binding: binding_id.into(),
                    query,
                    revision: b.ledger_revision,
                    offset: next,
                })
            })
            .transpose()?,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingCursor {
    agent: String,
    selection: u64,
    fingerprint: String,
    offset: u64,
}
pub(crate) fn status(
    db: &Connection,
    state: &Snapshot,
    agent: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<AgentMemoryStatus> {
    if !(1..=100).contains(&limit) {
        return Err(invalid("记忆来源分页参数无效"));
    }
    let choice = selection(state, agent)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut check=db.prepare("SELECT id,revision,ledger_revision FROM external_memory_bindings WHERE agent_id=?1 ORDER BY id")?;
    for row in check.query_map([agent], |r| {
        Ok((r.get::<_, String>(0)?, unsigned(r, 1)?, unsigned(r, 2)?))
    })? {
        let (id, rev, ledger) = row?;
        hasher.update((id.len() as u64).to_le_bytes());
        hasher.update(id.as_bytes());
        hasher.update(rev.to_le_bytes());
        hasher.update(ledger.to_le_bytes());
        total += 1;
    }
    let fingerprint = format!("{:x}", hasher.finalize());
    let offset = if let Some(raw) = cursor {
        if raw.len() > 4096 {
            return Err(invalid("来源分页游标无效"));
        }
        let c: BindingCursor = serde_json::from_str(raw)?;
        if c.agent != agent || c.selection != choice.revision || c.fingerprint != fingerprint {
            return Err(CoreError::MemoryProviderConflict);
        }
        c.offset
    } else {
        0
    };
    let rows:Vec<Binding>=db.prepare(&format!("SELECT {BIND_COLS} FROM external_memory_bindings WHERE agent_id=?1 ORDER BY (id=?2) DESC,created_at DESC,id LIMIT ?3 OFFSET ?4"))?.query_map(params![agent,choice.binding_id,limit as i64,integer(offset)?],bind_row)?.collect::<rusqlite::Result<_>>()?;
    let bindings = rows
        .iter()
        .map(|b| b.view(db))
        .collect::<Result<Vec<_>>>()?;
    let next = offset + bindings.len() as u64;
    Ok(AgentMemoryStatus {
        selection: choice.clone(),
        bindings,
        next_cursor: (next < total)
            .then(|| {
                encode(&BindingCursor {
                    agent: agent.into(),
                    selection: choice.revision,
                    fingerprint,
                    offset: next,
                })
            })
            .transpose()?,
    })
}
