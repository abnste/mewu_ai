// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::journal as j;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    run: String,
    revision: u64,
    after: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunsCursor {
    scene: String,
    created: u64,
    id: String,
}

pub(super) fn scope<'a>(state: &'a Snapshot, r: &j::Record) -> Result<&'a Scene> {
    state
        .scenes
        .iter()
        .find(|s| s.id == r.scene && s.agent_id == r.agent)
        .ok_or(CoreError::JournalConflict)
}
pub(super) fn live(state: &Snapshot, r: &j::Record) -> Result<()> {
    require_running(scope(state, r)?, &r.id)?;
    if r.status != RecordedRunStatus::Running {
        return Err(CoreError::StaleRun);
    }
    Ok(())
}
fn owned(
    db: &SqlConnection,
    state: &Snapshot,
    scene_id: &str,
    run_id: &str,
) -> Result<Option<j::Record>> {
    let Some(r) = j::record(db, run_id)? else {
        return Ok(None);
    };
    if r.scene != scene_id {
        return Err(CoreError::JournalConflict);
    }
    scope(state, &r)?;
    Ok(Some(r))
}
fn blocked(state: &Snapshot, r: &j::Record) -> Result<Option<ContinueBlockedReason>> {
    let s = scope(state, r)?;
    Ok(if r.status == RecordedRunStatus::Running || is_running(s) {
        Some(ContinueBlockedReason::Running)
    } else if s.closed {
        Some(ContinueBlockedReason::SceneClosed)
    } else if s.frozen {
        Some(ContinueBlockedReason::SceneFrozen)
    } else if state.active_scene_id != s.id {
        Some(ContinueBlockedReason::SceneInactive)
    } else if s
        .messages
        .iter()
        .position(|message| message.id == r.user)
        .is_none_or(|ordinal| ordinal < s.conversation_start.unwrap_or(0))
    {
        Some(ContinueBlockedReason::ConversationChanged)
    } else if s.connection_id.is_none() {
        Some(ContinueBlockedReason::ConnectionUnselected)
    } else {
        None
    })
}
fn summary(db: &SqlConnection, state: &Snapshot, r: &j::Record) -> Result<RunJournalSummary> {
    let mut reason = blocked(state, r)?;
    let (source, _) = canonical(db, r)?;
    if reason.is_none()
        && !j::events(db, &source.id)?
            .iter()
            .any(|e| e.summary.selectable)
    {
        reason = Some(ContinueBlockedReason::NoConfirmedContent)
    }
    Ok(RunJournalSummary {
        run_id: r.id.clone(),
        scene_id: r.scene.clone(),
        agent_id: r.agent.clone(),
        user_message_id: r.user.clone(),
        kind: r.kind.clone(),
        status: r.status.clone(),
        revision: r.revision,
        checkpoint_seq: r.checkpoint,
        created_at: r.created,
        updated_at: r.updated,
        can_continue: reason.is_none(),
        continue_blocked_reason: reason,
    })
}
fn canonical(db: &SqlConnection, r: &j::Record) -> Result<(j::Record, Option<Vec<String>>)> {
    if let Some(p) = &r.provenance {
        let source = j::required(db, &p.source_run_id)?;
        if source.scene != r.scene
            || source.agent != r.agent
            || source.kind == RecordedRunKind::Continuation
        {
            return Err(CoreError::JournalConflict);
        }
        Ok((source, Some(p.selected_event_ids.clone())))
    } else {
        Ok((r.clone(), None))
    }
}
struct ProjectionPlan {
    decision: ContinuationDecision,
    original_messages: Vec<RecordedMessage>,
    entries: Vec<j::Event>,
}

// Passing no connection produces only the metadata envelope. Body reads are
// reserved for building an accepted continuation, never a page or preview.
fn evidence_envelope(
    db: Option<&SqlConnection>,
    source: &j::Record,
    entries: &[j::Event],
    selected: &[String],
) -> Result<String> {
    let mut records = Vec::with_capacity(entries.len());
    for e in entries {
        let chosen = selected.contains(&e.summary.id);
        let content = if chosen {
            db.map(|db| j::event(db, &e.summary.id).map(|event| event.2))
                .transpose()?
        } else {
            None
        };
        records.push(serde_json::json!({"id":e.summary.id,"kind":e.summary.kind,"phase":e.summary.phase,"label":e.summary.label,"responseKind":e.summary.response_kind,"returnedError":e.summary.returned_error,"contentUnavailable":e.summary.content_unavailable,"contentOmitted":e.summary.selectable&&!chosen,"content":content}));
    }
    Ok(serde_json::to_string(
        &serde_json::json!({"version":1,"sourceRunId":source.id,"records":records}),
    )?)
}

fn projection_plan(
    db: &SqlConnection,
    state: &Snapshot,
    r: &j::Record,
    selected: Option<&[String]>,
) -> Result<ProjectionPlan> {
    let s = scope(state, r)?;
    let end = s
        .messages
        .iter()
        .position(|m| m.id == r.user && m.role == MessageRole::User)
        .ok_or(CoreError::JournalConflict)?;
    let range = original_conversation_range(s, end)?;
    let original_messages: Vec<RecordedMessage> = s.messages[range]
        .iter()
        .map(|m| RecordedMessage {
            id: m.id.clone(),
            role: m.role.clone(),
            text: m.text.clone(),
            attachments: m.attachments.clone().unwrap_or_default(),
        })
        .collect();
    let entries = j::events(db, &r.id)?;
    let defaults: Vec<String> = entries
        .iter()
        .filter(|e| e.summary.selectable)
        .map(|e| e.summary.id.clone())
        .collect();
    let selection = selected
        .map(|s| s.to_vec())
        .unwrap_or_else(|| defaults.clone());
    let unique: HashSet<&str> = selection.iter().map(String::as_str).collect();
    if unique.len() != selection.len() || selection.iter().any(|id| !defaults.contains(id)) {
        return Err(CoreError::JournalConflict);
    }
    // All call states remain mandatory. The only selected-body differences are
    // replacing `null` (4 bytes) by its escaped JSON and `true` by `false` (+1).
    // Cached body contributions are exact, not a raw-content length estimate.
    let mandatory = evidence_envelope(None, r, &entries, &[])?;
    let mandatory_size = serde_json::to_vec(&serde_json::json!({"originalMessages":original_messages,"quotedEvidence":mandatory,"continuationRequest":"根据以上已确认记录继续回答。工具不可用；不要声称未知操作已完成。"}))?.len() as u64;
    let bytes = |ids: &[String]| -> Result<u64> {
        entries
            .iter()
            .filter(|e| ids.contains(&e.summary.id))
            .try_fold(mandatory_size, |total, e| {
                e.projection_content_bytes
                    .checked_sub(3)
                    .and_then(|added| total.checked_add(added))
                    .ok_or_else(|| j::bad("执行正文投影长度无效"))
            })
    };
    let size = bytes(&selection)?;
    let default_size = bytes(&defaults)?;
    let mut reason = blocked(state, r)?;
    if reason.is_none() {
        reason = if mandatory_size > j::PROJECTION_LIMIT as u64 {
            Some(ContinueBlockedReason::MandatoryContextTooLarge)
        } else if selection.is_empty() {
            Some(ContinueBlockedReason::NoConfirmedContent)
        } else if size > j::PROJECTION_LIMIT as u64 {
            Some(ContinueBlockedReason::SelectionTooLarge)
        } else {
            None
        };
    }
    Ok(ProjectionPlan {
        decision: ContinuationDecision {
            source_run_id: r.id.clone(),
            journal_revision: r.revision,
            checkpoint_seq: r.checkpoint,
            budget_bytes: j::PROJECTION_LIMIT as u64,
            projection_bytes: size,
            default_projection_bytes: default_size,
            default_event_ids: defaults,
            selected_event_ids: selection,
            selection_required: default_size > j::PROJECTION_LIMIT as u64,
            ready: reason.is_none(),
            blocked_reason: reason,
        },
        original_messages,
        entries,
    })
}

fn build_projection(
    db: &SqlConnection,
    source: &j::Record,
    plan: ProjectionPlan,
) -> Result<ContinuationProjection> {
    let quoted = evidence_envelope(
        Some(db),
        source,
        &plan.entries,
        &plan.decision.selected_event_ids,
    )?;
    let original_messages = plan.original_messages;
    // Recheck the actual encoding before a new Run is committed. Derived
    // metadata never authorizes a body that is larger than the accepted budget.
    let size = serde_json::to_vec(&serde_json::json!({"originalMessages":original_messages,"quotedEvidence":quoted,"continuationRequest":"根据以上已确认记录继续回答。工具不可用；不要声称未知操作已完成。"}))?.len() as u64;
    if size != plan.decision.projection_bytes || size > j::PROJECTION_LIMIT as u64 {
        return Err(j::bad("执行正文投影长度不匹配"));
    }
    let provenance = ContinuationProvenance {
        source_run_id: source.id.clone(),
        source_revision: source.revision,
        checkpoint_seq: source.checkpoint,
        selected_event_ids: plan.decision.selected_event_ids,
        projection_sha256: j::digest(
            serde_json::to_vec(
                &serde_json::json!({"originalMessages":original_messages,"quotedEvidence":quoted}),
            )?
            .as_slice(),
        ),
        projection_version: 1,
        original_message_ids: original_messages.iter().map(|m| m.id.clone()).collect(),
    };
    Ok(ContinuationProjection {
        provenance,
        original_messages,
        quoted_evidence_json: quoted,
    })
}
fn page(
    db: &SqlConnection,
    state: &Snapshot,
    scene: &str,
    run: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Option<JournalPage>> {
    if !(1..=25).contains(&limit) {
        return Err(j::bad("记录页大小应为 1 至 25"));
    }
    let Some(r) = owned(db, state, scene, run)? else {
        return Ok(None);
    };
    let after = if let Some(cursor) = cursor {
        if cursor.len() > 512 {
            return Err(CoreError::JournalConflict);
        }
        let c: Cursor = serde_json::from_str(cursor)?;
        if c.run != run || c.revision != r.revision {
            return Err(CoreError::JournalConflict);
        }
        c.after
    } else {
        0
    };
    let all = j::events(db, run)?;
    let entries: Vec<_> = all
        .iter()
        .filter(|e| e.summary.sequence > after)
        .take(limit)
        .map(|e| e.summary.clone())
        .collect();
    let last = entries.last().map(|e| e.sequence).unwrap_or(after);
    let next_cursor = if all.iter().any(|e| e.summary.sequence > last) {
        Some(j::encode(&Cursor {
            run: run.into(),
            revision: r.revision,
            after: last,
        })?)
    } else {
        None
    };
    let (source, selected) = canonical(db, &r)?;
    let continuation = projection_plan(db, state, &source, selected.as_deref())?.decision;
    let mut run_summary = summary(db, state, &r)?;
    if continuation.blocked_reason == Some(ContinueBlockedReason::MandatoryContextTooLarge) {
        run_summary.can_continue = false;
        run_summary.continue_blocked_reason = continuation.blocked_reason.clone();
    }
    Ok(Some(JournalPage {
        summary: run_summary,
        entries,
        continuation,
        next_cursor,
    }))
}
fn entry(
    db: &SqlConnection,
    state: &Snapshot,
    scene: &str,
    run: &str,
    id: &str,
    revision: u64,
) -> Result<JournalEntryDetail> {
    let r = owned(db, state, scene, run)?.ok_or(CoreError::JournalConflict)?;
    if r.revision != revision {
        return Err(CoreError::JournalConflict);
    }
    let (owner, e, content) = j::event(db, id)?;
    if owner != run {
        return Err(CoreError::JournalConflict);
    }
    Ok(JournalEntryDetail {
        run_id: run.into(),
        journal_revision: revision,
        entry: e.summary,
        binding: e.binding,
        arguments_wire_sha256: e.arguments_sha256,
        content,
        references: e.references,
    })
}
fn runs_page(
    db: &SqlConnection,
    state: &Snapshot,
    scene: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<RunJournalPage> {
    if !(1..=25).contains(&limit) || !state.scenes.iter().any(|s| s.id == scene) {
        return Err(CoreError::JournalConflict);
    }
    let c = cursor
        .map(|s| {
            if s.len() > 512 {
                return Err(CoreError::JournalConflict);
            }
            Ok(serde_json::from_str::<RunsCursor>(s)?)
        })
        .transpose()?;
    if c.as_ref().is_some_and(|c| c.scene != scene) {
        return Err(CoreError::JournalConflict);
    }
    let mut q=db.prepare("SELECT data FROM agent_run_records WHERE scene_id=?1 AND (?2 IS NULL OR created_at<?2 OR (created_at=?2 AND id>?3)) ORDER BY created_at DESC,id LIMIT ?4")?;
    let rows = q.query_map(
        params![
            scene,
            c.as_ref().map(|c| c.created as i64),
            c.as_ref().map(|c| c.id.as_str()),
            (limit + 1) as i64
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut records = rows
        .map(|row| Ok(serde_json::from_str::<j::Record>(&row?)?))
        .collect::<Result<Vec<_>>>()?;
    let more = records.len() > limit;
    records.truncate(limit);
    let next_cursor = if more {
        let r = records.last().ok_or(CoreError::JournalConflict)?;
        Some(j::encode(&RunsCursor {
            scene: scene.into(),
            created: r.created,
            id: r.id.clone(),
        })?)
    } else {
        None
    };
    Ok(RunJournalPage {
        runs: records
            .iter()
            .map(|r| summary(db, state, r))
            .collect::<Result<_>>()?,
        next_cursor,
    })
}

impl Store {
    fn journal_read<T>(
        &self,
        read: impl FnOnce(&SqlConnection, &Snapshot) -> Result<T>,
    ) -> Result<T> {
        let tx = self.db.unchecked_transaction()?;
        let revision: i64 = tx.query_row("SELECT revision FROM app_state WHERE id=1", [], |r| {
            r.get(0)
        })?;
        if revision != self.revision {
            return Err(CoreError::ConcurrentModification);
        }
        let result = read(&tx, &self.state)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn run_journal_summary(&self, scene: &str, run: &str) -> Result<Option<RunJournalSummary>> {
        self.journal_read(|db, state| {
            owned(db, state, scene, run)?
                .map(|r| summary(db, state, &r))
                .transpose()
        })
    }
    pub fn scene_run_journal_page(
        &self,
        scene: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RunJournalPage> {
        self.journal_read(|db, state| runs_page(db, state, scene, cursor, limit))
    }
    pub fn run_journal_page(
        &self,
        scene: &str,
        run: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Option<JournalPage>> {
        self.journal_read(|db, state| page(db, state, scene, run, cursor, limit))
    }
    pub fn run_journal_entry(
        &self,
        scene: &str,
        run: &str,
        entry_id: &str,
        revision: u64,
    ) -> Result<JournalEntryDetail> {
        self.journal_read(|db, state| entry(db, state, scene, run, entry_id, revision))
    }
    pub fn preview_run_continuation(
        &self,
        input: &ContinuationSelection,
    ) -> Result<ContinuationDecision> {
        self.journal_read(|db, state| {
            let r = owned(db, state, &input.scene_id, &input.source_run_id)?
                .ok_or(CoreError::JournalConflict)?;
            if r.revision != input.expected_journal_revision
                || r.checkpoint != input.expected_checkpoint_seq
            {
                return Err(CoreError::JournalConflict);
            }
            let (source, saved) = canonical(db, &r)?;
            let selected = input.selected_event_ids.as_deref().or(saved.as_deref());
            Ok(projection_plan(db, state, &source, selected)?.decision)
        })
    }

    pub(super) fn journal_transaction<T>(
        &mut self,
        change: impl FnOnce(&SqlConnection, &mut Snapshot) -> Result<T>,
    ) -> Result<T> {
        let mut next = self.state.clone();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let app: i64 = tx.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version != DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(version as u64));
        }
        if app != APPLICATION_ID {
            return Err(j::bad("数据库应用标识已改变"));
        }
        let revision: i64 = tx.query_row("SELECT revision FROM app_state WHERE id=1", [], |r| {
            r.get(0)
        })?;
        if revision != self.revision {
            return Err(CoreError::ConcurrentModification);
        }
        let before = tx.total_changes();
        let result = change(&tx, &mut next)?;
        if tx.total_changes() == before && next == self.state {
            tx.commit()?;
            return Ok(result);
        }
        crate::external_memory::on_snapshot_change(&tx, &self.state, &next)?;
        j::on_snapshot_change(&tx, &self.state, &mut next)?;
        next.memory_stats = crate::memory::stats(&tx, &next)?;
        validate_snapshot(&next)?;
        crate::rich_annotations::validate(&tx, &next)?;
        crate::video_registered_layouts::validate(&tx, &next)?;
        crate::video_annotations::validate_ledger(&tx, &next)?;
        tx.execute(
            "UPDATE app_state SET revision=revision+1,payload=?1 WHERE id=1 AND revision=?2",
            params![serde_json::to_string(&next)?, self.revision],
        )?;
        tx.commit()?;
        self.state = next;
        self.revision += 1;
        Ok(result)
    }
    pub fn begin_model_request(
        &mut self,
        scene: &str,
        run: &str,
        input: ModelDispatchInput,
    ) -> Result<DispatchLease> {
        if input.round >= 6 || !j::hash_valid(&input.projected_request_sha256) {
            return Err(j::bad("模型请求记录无效"));
        }
        self.journal_transaction(|db, state| {
            let mut r = owned(db, state, scene, run)?.ok_or(CoreError::JournalConflict)?;
            live(state, &r)?;
            if j::events(db, run)?
                .iter()
                .any(|e| e.summary.kind == ReceiptKind::Model && e.summary.round == input.round)
            {
                return Err(CoreError::JournalConflict);
            }
            let (mut e, lease) = j::create_event(
                db,
                &r,
                ReceiptKind::Model,
                input.round,
                "模型".into(),
                None,
                None,
                None,
                Some(input.projected_request_sha256),
            )?;
            e.summary.phase = ReceiptPhase::Dispatched;
            j::save_event(
                db,
                &e,
                &JournalContent::Omitted {
                    reason: ContentUnavailable::NotReceived,
                },
            )?;
            j::bump(db, &mut r)?;
            Ok(lease)
        })
    }
    pub fn record_model_turn(
        &mut self,
        lease: &DispatchLease,
        turn: CompletePublicTurn,
    ) -> Result<TurnCommit> {
        if turn.text.len() > j::TEXT_LIMIT || turn.calls.len() > 8 {
            return Err(j::bad("模型回执超出预算"));
        }
        for c in &turn.calls {
            j::validate_binding(&c.binding)?;
            if c.call_id.is_empty()
                || c.call_id.len() > 200
                || c.call_id.chars().any(char::is_control)
                || !j::hash_valid(&c.arguments_wire_sha256)
            {
                return Err(j::bad("工具调用记录无效"));
            }
        }
        self.journal_transaction(|db, state| {
            let (mut r, mut e, _) = j::leased(db, lease)?;
            if e.summary.kind != ReceiptKind::Model
                || !matches!(
                    e.summary.phase,
                    ReceiptPhase::Dispatched | ReceiptPhase::Unknown
                )
            {
                return Err(CoreError::JournalConflict);
            }
            e.summary.phase = ReceiptPhase::Responded;
            e.summary.finished_at = Some(j::stamp().max(e.summary.started_at));
            let body = JournalContent::Text { text: turn.text };
            j::set_content(&mut e, &body)?;
            j::save_event(db, &e, &body)?;
            let mut tools = vec![];
            for c in turn.calls {
                let label = tool_label(&c.binding);
                let (mut tool, lease) = j::create_event(
                    db,
                    &r,
                    ReceiptKind::Tool,
                    e.summary.round,
                    label,
                    Some(c.call_id.clone()),
                    Some(c.binding),
                    Some(c.arguments_wire_sha256),
                    None,
                )?;
                if r.status != RecordedRunStatus::Running {
                    tool.summary.phase = ReceiptPhase::NotSent;
                    tool.summary.finished_at = Some(j::stamp().max(tool.summary.started_at));
                    tool.not_sent = Some(NotSentReason::RunEnded);
                    j::save_event(
                        db,
                        &tool,
                        &JournalContent::Omitted {
                            reason: ContentUnavailable::NotReceived,
                        },
                    )?;
                }
                j::sync_step(state, &r, &tool)?;
                tools.push(PreparedTool {
                    call_id: c.call_id,
                    lease,
                });
            }
            let c = j::bump(db, &mut r)?;
            Ok(TurnCommit {
                journal_revision: c.journal_revision,
                checkpoint_seq: c.checkpoint_seq,
                tools,
            })
        })
    }
    pub fn mark_tool_dispatched(&mut self, lease: &DispatchLease) -> Result<ReceiptCommit> {
        self.journal_transaction(|db, state| {
            let (mut r, mut e, body) = j::leased(db, lease)?;
            live(state, &r)?;
            if e.summary.kind != ReceiptKind::Tool || e.summary.phase != ReceiptPhase::Prepared {
                return Err(CoreError::JournalConflict);
            }
            dispatched_budget(db, &r.id)?;
            if r.kind == RecordedRunKind::Continuation {
                return Err(CoreError::McpPermissionDenied);
            }
            if let Some(ToolBinding::Mcp {
                server_id,
                revision,
                name,
                ..
            }) = &e.binding
            {
                let server = state
                    .mcp_servers
                    .iter()
                    .find(|s| s.id == *server_id && s.revision == *revision && s.enabled)
                    .ok_or(CoreError::McpPermissionDenied)?;
                if !server.tools.iter().any(|t| t.name == *name)
                    || !server
                        .grants
                        .iter()
                        .any(|g| g.agent_id == r.agent && g.tool_names.contains(name))
                {
                    return Err(CoreError::McpPermissionDenied);
                }
            } else if matches!(e.binding, Some(ToolBinding::Rejected { .. })) {
                return Err(CoreError::McpPermissionDenied);
            }
            e.summary.phase = ReceiptPhase::Dispatched;
            j::save_event(db, &e, &body)?;
            j::sync_step(state, &r, &e)?;
            j::bump(db, &mut r)
        })
    }
    pub fn record_tool_not_sent(
        &mut self,
        lease: &DispatchLease,
        reason: NotSentReason,
    ) -> Result<ReceiptCommit> {
        self.journal_transaction(|db, state| {
            let (mut r, mut e, body) = j::leased(db, lease)?;
            if e.summary.kind == ReceiptKind::Tool && e.summary.phase == ReceiptPhase::NotSent {
                return Ok(ReceiptCommit {
                    journal_revision: r.revision,
                    checkpoint_seq: r.checkpoint,
                });
            }
            if e.summary.kind != ReceiptKind::Tool || e.summary.phase != ReceiptPhase::Prepared {
                return Err(CoreError::JournalConflict);
            }
            e.summary.phase = ReceiptPhase::NotSent;
            e.not_sent = Some(reason);
            e.summary.finished_at = Some(j::stamp().max(e.summary.started_at));
            j::save_event(db, &e, &body)?;
            j::sync_step(state, &r, &e)?;
            j::bump(db, &mut r)
        })
    }
    pub fn record_request_unknown(
        &mut self,
        lease: &DispatchLease,
        reason: NoResponseReason,
    ) -> Result<ReceiptCommit> {
        self.journal_transaction(|db, state| {
            let (mut r, mut e, body) = j::leased(db, lease)?;
            if e.summary.phase == ReceiptPhase::Unknown {
                return Ok(ReceiptCommit {
                    journal_revision: r.revision,
                    checkpoint_seq: r.checkpoint,
                });
            }
            if e.summary.phase != ReceiptPhase::Dispatched {
                return Err(CoreError::JournalConflict);
            }
            e.summary.phase = ReceiptPhase::Unknown;
            e.unknown = Some(reason);
            e.summary.finished_at = Some(j::stamp().max(e.summary.started_at));
            j::save_event(db, &e, &body)?;
            j::sync_step(state, &r, &e)?;
            j::bump(db, &mut r)
        })
    }
    pub fn record_tool_response(
        &mut self,
        lease: &DispatchLease,
        result: ObservedToolResult,
    ) -> Result<ReceiptCommit> {
        self.journal_transaction(|db, state| respond(db, state, lease, result))
    }

    pub fn apply_local_memory_with_receipt(
        &mut self,
        lease: &DispatchLease,
        effect: LocalMemoryEffect,
    ) -> Result<LocalToolCommit> {
        let (result, receipt) = self.journal_transaction(|db, state| {
            let (r, e, _) = j::leased(db, lease)?;
            live(state, &r)?;
            let (name, mutation, quote) = match effect {
                LocalMemoryEffect::Save {
                    id,
                    text,
                    expected_revision,
                    source_quote,
                } => (
                    "memory_save",
                    MemoryMutation::Save {
                        id,
                        text,
                        expected_revision,
                    },
                    Some(source_quote),
                ),
                LocalMemoryEffect::Forget {
                    id,
                    expected_revision,
                } => (
                    "memory_forget",
                    MemoryMutation::Delete {
                        id,
                        expected_revision,
                    },
                    None,
                ),
            };
            if e.summary.phase != ReceiptPhase::Prepared
                || !matches!(&e.binding,Some(ToolBinding::Local{name:n}) if n==name)
            {
                return Err(CoreError::JournalConflict);
            }
            let (agent, source) = local_memory_scope(state, &r.scene, &r.id)?;
            if let Some(quote) = quote {
                let quote = quote.trim();
                if quote.is_empty()
                    || quote.chars().count() > 500
                    || !scope(state, &r)?
                        .messages
                        .iter()
                        .any(|m| m.id == r.user && m.text.contains(quote))
                {
                    return Err(j::bad("记忆依据必须来自本轮用户输入"));
                }
            }
            crate::memory::mutate(db, &agent, mutation, Some(source))?;
            let result = local_result(if name == "memory_save" {
                serde_json::json!({"saved":true})
            } else {
                serde_json::json!({"deleted":true})
            })?;
            let receipt = respond_local(db, state, lease, result.clone())?;
            Ok((result, receipt))
        })?;
        Ok(LocalToolCommit {
            snapshot: self.snapshot(),
            result,
            receipt,
        })
    }

    pub fn queue_external_quote_with_receipt(
        &mut self,
        lease: &DispatchLease,
        quote: &str,
    ) -> Result<LocalToolCommit> {
        let (result,receipt)=self.journal_transaction(|db,state|{
            let(r,e,_)=j::leased(db,lease)?;live(state,&r)?;let grant=external_grant(&e,"memory_save")?;
            let b=crate::external_memory::run_binding(db,state,&r.scene,&r.id,&grant)?;
            if !b.policy.explicit_retain{return Err(CoreError::MemoryDisabled)}
            if e.summary.phase!=ReceiptPhase::Prepared||quote.trim().is_empty()||quote.len()>64*1024{return Err(CoreError::JournalConflict)}
            let user=scope(state,&r)?.messages.iter().find(|m|m.id==r.user&&m.text.contains(quote)).ok_or_else(||j::bad("只能保存本轮用户原文"))?;
            let source=EvidenceSource{origin:EvidenceOrigin::UserQuote,scene_id:Some(r.scene.clone()),user_message_id:Some(r.user.clone()),assistant_message_id:None,run_id:Some(r.id.clone())};
            let payload=serde_json::to_string(&serde_json::json!({"version":1,"occurredAt":user.created_at,"parts":[{"role":"user","text":quote}]}))?;
            let queued=crate::external_memory::queue_evidence(db,&b,&format!("request:{}",e.summary.id),source,Some(payload),None)?;
            let mut result=local_result(serde_json::to_value(&queued)?)?;
            result.references.push(EvidenceReference::ExternalMemory{binding_id:b.id,evidence_id:queued.evidence_id,operation_id:queued.operation_id,revision:queued.revision});
            let receipt=respond_local(db,state,lease,result.clone())?;Ok((result,receipt))
        })?;
        Ok(LocalToolCommit {
            snapshot: self.snapshot(),
            result,
            receipt,
        })
    }

    pub fn forget_external_evidence_with_receipt(
        &mut self,
        lease: &DispatchLease,
        evidence_id: &str,
        expected_revision: u64,
    ) -> Result<LocalToolCommit> {
        use crate::external_memory as ledger;
        let (result, receipt) = self.journal_transaction(|db, state| {
            let (r, e, _) = j::leased(db, lease)?;
            live(state, &r)?;
            let grant = external_grant(&e, "memory_forget")?;
            let b = ledger::run_binding(db, state, &r.scene, &r.id, &grant)?;
            if e.summary.phase != ReceiptPhase::Prepared {
                return Err(CoreError::JournalConflict);
            }
            let evidence = ledger::evidence(db, evidence_id)?
                .filter(|e| e.binding_id == b.id)
                .ok_or(CoreError::MemoryConflict)?;
            if evidence.revision != expected_revision {
                return Err(CoreError::MemoryConflict);
            }
            let forgotten = if let Some(receipt) = ledger::forgotten(db, &evidence)? {
                receipt
            } else {
                super::memory_ledger_store::cancel_authority(state, Some(&b.id), None)?;
                super::memory_ledger_store::forget_transaction(db, state, &b, &evidence)?
            };
            let mut result = local_result(serde_json::to_value(&forgotten)?)?;
            result.references.push(EvidenceReference::ExternalMemory {
                binding_id: b.id,
                evidence_id: evidence_id.into(),
                operation_id: None,
                revision: forgotten.revision,
            });
            // This commit records the caller's receipt even though forget cancels
            // its own run. The outer transaction settles the other pending work.
            let receipt = respond_local(db, state, lease, result.clone())?;
            Ok((result, receipt))
        })?;
        Ok(LocalToolCommit {
            snapshot: self.snapshot(),
            result,
            receipt,
        })
    }

    pub fn begin_recorded_continuation(&mut self, input: ContinueFromRecord) -> Result<RunContext> {
        self.journal_transaction(|db,state|{
            let r=owned(db,state,&input.scene_id,&input.source_run_id)?.ok_or(CoreError::JournalConflict)?;
            if r.revision!=input.expected_journal_revision||r.checkpoint!=input.expected_checkpoint_seq{return Err(CoreError::JournalConflict)}
            let(r,prior_selection)=canonical(db,&r)?;
            let plan=projection_plan(db,state,&r,input.selected_event_ids.as_deref().or(prior_selection.as_deref()))?;
            if !plan.decision.ready{return Err(j::bad("当前记录不能直接续答，请重新读取或选择证据"))}
            let projection=build_projection(db,&r,plan)?;
            let original=scope(state,&r)?;
            let connection=state.connections.iter().find(|c|Some(c.id.as_str())==original.connection_id.as_deref()&&c.id==input.expected_connection_id&&c.revision==input.expected_connection_revision).cloned().ok_or(CoreError::ConnectionConflict)?;
            let mut agent=state.agents.iter().find(|a|a.id==r.agent).cloned().ok_or(CoreError::JournalConflict)?;agent.memory_enabled=false;
            let new_id=id();let user_id=id();let scene=scene_mut(state,&r.scene)?;
            scene.messages.push(Message{id:user_id.clone(),role:MessageRole::User,text:"继续回答".into(),reasoning:None,conversation_start:None,created_at:now(),run_id:Some(new_id.clone()),refs:Some(vec![]),attachments:Some(vec![]),tool_steps:vec![]});
            scene.run=Some(Run{id:new_id.clone(),kind:RecordedRunKind::Continuation,status:RunStatus::Running,connection_id:Some(connection.id.clone()),connection_revision:Some(connection.revision),memory_authority:None,error:None,steps:vec![]});touch(scene);
            let t=j::stamp();let record=j::Record{id:new_id.clone(),scene:r.scene.clone(),agent:r.agent.clone(),user:user_id,kind:RecordedRunKind::Continuation,status:RecordedRunStatus::Running,revision:1,checkpoint:0,created:t,updated:t,connection_id:connection.id.clone(),connection_revision:connection.revision,provenance:Some(projection.provenance.clone())};
            db.execute("INSERT INTO agent_run_records(id,scene_id,created_at,revision,data) VALUES(?1,?2,?3,1,?4)",params![record.id,record.scene,t as i64,j::encode(&record)?])?;
            Ok(RunContext{scene_id:r.scene,run_id:new_id,memory_authority:None,agent,memories:vec![],mcp_servers:vec![],connection:connection.connection(),connection_profile:connection,messages:scene.messages.iter().skip(scene.conversation_start.unwrap_or(0)).cloned().collect(),background:None,regions:vec![],items:vec![],refs:vec![],projection:RunProjection::RecordedContinuation(projection)})
        })
    }
}
fn external_grant(e: &j::Event, wanted: &str) -> Result<MemoryAuthorityGrant> {
    match &e.binding {
        Some(ToolBinding::ExternalMemory {
            binding_id,
            binding_revision,
            plugin_id,
            plugin_revision,
            contribution_id,
            name,
        }) if name == wanted => Ok(MemoryAuthorityGrant {
            binding_id: binding_id.clone(),
            binding_revision: *binding_revision,
            plugin_id: plugin_id.clone(),
            plugin_revision: *plugin_revision,
            contribution_id: contribution_id.clone(),
        }),
        _ => Err(CoreError::JournalConflict),
    }
}
pub(super) fn local_result(value: serde_json::Value) -> Result<ObservedToolResult> {
    let bytes = serde_json::to_vec(&value)?;
    Ok(ObservedToolResult {
        response_kind: ToolResponseKind::LocalCommit,
        returned_error: None,
        observed_encoding: "mewu-local-json-v1".into(),
        observed_bytes: bytes.len() as u64,
        observed_sha256: j::digest(&bytes),
        retained_content: JournalContent::Json {
            value: value.clone(),
        },
        references: vec![],
        model_visible_json: Some(serde_json::to_string(&value)?),
    })
}
pub(super) fn respond_local(
    db: &SqlConnection,
    state: &mut Snapshot,
    lease: &DispatchLease,
    result: ObservedToolResult,
) -> Result<ReceiptCommit> {
    let (_, mut e, body) = j::leased(db, lease)?;
    if e.summary.phase != ReceiptPhase::Prepared {
        return Err(CoreError::JournalConflict);
    }
    dispatched_budget(db, &lease.run_id)?;
    e.summary.phase = ReceiptPhase::Dispatched;
    j::save_event(db, &e, &body)?;
    respond(db, state, lease, result)
}
fn dispatched_budget(db: &SqlConnection, run: &str) -> Result<()> {
    let count:i64=db.query_row("SELECT count(*) FROM agent_run_events WHERE run_id=?1 AND json_extract(metadata,'$.summary.kind')='tool' AND json_extract(metadata,'$.summary.phase') NOT IN ('prepared','not_sent')",[run],|r|r.get(0))?;
    if count >= 16 {
        return Err(j::bad("本次工具调用次数已达上限"));
    }
    Ok(())
}
fn tool_label(binding: &ToolBinding) -> String {
    if matches!(binding, ToolBinding::VideoAnnotation { .. }) {
        return "视频批注".into();
    }
    if matches!(binding, ToolBinding::VisualAnnotation { .. }) {
        return "绘制批注".into();
    }
    match binding {
        ToolBinding::Local { name } | ToolBinding::ExternalMemory { name, .. } => {
            match name.as_str() {
                "memory_recall" => "检索记忆",
                "memory_save" => "保存记忆",
                "memory_forget" => "删除记忆",
                "history_search" => "查找历史",
                _ => name,
            }
        }
        ToolBinding::Mcp { name, .. } => name,
        ToolBinding::Rejected { advertised_name } => advertised_name,
        ToolBinding::VisualAnnotation { name, .. } | ToolBinding::VideoAnnotation { name, .. } => {
            name
        }
    }
    .chars()
    .take(80)
    .collect()
}
fn respond(
    db: &SqlConnection,
    state: &mut Snapshot,
    lease: &DispatchLease,
    result: ObservedToolResult,
) -> Result<ReceiptCommit> {
    let (mut r, mut e, old) = j::leased(db, lease)?;
    if e.summary.kind != ReceiptKind::Tool
        || !j::hash_valid(&result.observed_sha256)
        || result.observed_encoding.is_empty()
        || result.observed_encoding.len() > 100
    {
        return Err(CoreError::JournalConflict);
    }
    if e.summary.phase == ReceiptPhase::Responded {
        if old == result.retained_content
            && e.summary.observed_sha256.as_deref() == Some(&result.observed_sha256)
            && e.summary.observed_bytes == Some(result.observed_bytes)
        {
            return Ok(ReceiptCommit {
                journal_revision: r.revision,
                checkpoint_seq: r.checkpoint,
            });
        }
        return Err(CoreError::JournalConflict);
    }
    if !matches!(
        e.summary.phase,
        ReceiptPhase::Dispatched | ReceiptPhase::Unknown
    ) {
        return Err(CoreError::JournalConflict);
    }
    match &result.retained_content {
        JournalContent::Json { value } if serde_json::to_vec(value)?.len() <= j::TOOL_LIMIT => {}
        JournalContent::Omitted { .. } => {}
        _ => return Err(j::bad("工具结果超出记录预算")),
    }
    j::validate_references(&result.references)?;
    e.summary.phase = ReceiptPhase::Responded;
    e.summary.finished_at = Some(j::stamp().max(e.summary.started_at));
    e.summary.returned_error = result.returned_error;
    e.summary.response_kind = Some(result.response_kind);
    e.summary.observed_encoding = Some(result.observed_encoding);
    e.summary.observed_bytes = Some(result.observed_bytes);
    e.summary.observed_sha256 = Some(result.observed_sha256);
    e.references = result.references;
    j::set_content(&mut e, &result.retained_content)?;
    j::save_event(db, &e, &result.retained_content)?;
    j::sync_step(state, &r, &e)?;
    j::bump(db, &mut r)
}

impl ExportView {
    pub fn run_journal_summary(&self, scene: &str, run: &str) -> Result<Option<RunJournalSummary>> {
        owned(&self.db, &self.state, scene, run)?
            .map(|r| summary(&self.db, &self.state, &r))
            .transpose()
    }
    pub fn scene_run_journal_page(
        &self,
        scene: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RunJournalPage> {
        runs_page(&self.db, &self.state, scene, cursor, limit)
    }
    pub fn run_journal_page(
        &self,
        scene: &str,
        run: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Option<JournalPage>> {
        page(&self.db, &self.state, scene, run, cursor, limit)
    }
    pub fn run_journal_entry(
        &self,
        scene: &str,
        run: &str,
        id: &str,
        revision: u64,
    ) -> Result<JournalEntryDetail> {
        entry(&self.db, &self.state, scene, run, id, revision)
    }
}

#[cfg(test)]
mod projection_read_tests {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn builtin_labels_are_localized_without_renaming_mcp_tools() {
        for (name, label) in [
            ("memory_recall", "检索记忆"),
            ("memory_save", "保存记忆"),
            ("memory_forget", "删除记忆"),
            ("history_search", "查找历史"),
        ] {
            assert_eq!(tool_label(&ToolBinding::Local { name: name.into() }), label);
            assert_eq!(
                tool_label(&ToolBinding::ExternalMemory {
                    binding_id: id(),
                    binding_revision: 1,
                    plugin_id: "memory.fixture".into(),
                    plugin_revision: 1,
                    contribution_id: "memory".into(),
                    name: name.into(),
                }),
                label
            );
            assert_eq!(
                tool_label(&ToolBinding::Mcp {
                    server_id: id(),
                    revision: 1,
                    name: name.into(),
                    advertised_alias: format!("fixture_{name}"),
                }),
                name
            );
        }
    }

    #[test]
    fn pages_and_previews_work_with_sqlite_body_reads_denied() {
        let mut store = Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "合成问题".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene).unwrap();
        for round in 0..2 {
            let lease = store
                .begin_model_request(
                    &scene,
                    &run.run_id,
                    ModelDispatchInput {
                        round,
                        projected_request_sha256: "a".repeat(64),
                    },
                )
                .unwrap();
            store
                .record_model_turn(
                    &lease,
                    CompletePublicTurn {
                        text: format!("合成正文 {round} \"\\🙂"),
                        calls: vec![],
                    },
                )
                .unwrap();
        }
        store.fail_run(&scene, &run.run_id, "合成中断").unwrap();
        let denied = Arc::new(AtomicUsize::new(0));
        let attempts = denied.clone();
        store
            .db
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "agent_run_events",
                        column_name: "content",
                    }
                ) {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }))
            .unwrap();
        let page = store
            .run_journal_page(&scene, &run.run_id, None, 1)
            .unwrap()
            .unwrap();
        let next = store
            .run_journal_page(&scene, &run.run_id, page.next_cursor.as_deref(), 1)
            .unwrap()
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(next.entries.len(), 1);
        assert_ne!(page.entries[0].id, next.entries[0].id);
        let selection = ContinuationSelection {
            scene_id: scene.clone(),
            source_run_id: run.run_id.clone(),
            expected_journal_revision: page.summary.revision,
            expected_checkpoint_seq: page.summary.checkpoint_seq,
            selected_event_ids: Some(vec![page.entries[0].id.clone()]),
        };
        let preview = store.preview_run_continuation(&selection).unwrap();
        assert!(preview.ready);
        assert_eq!(denied.load(Ordering::SeqCst), 0);
        assert!(store
            .run_journal_entry(
                &scene,
                &run.run_id,
                &page.entries[0].id,
                page.summary.revision
            )
            .is_err());
        assert_eq!(denied.load(Ordering::SeqCst), 1);
        let connection = store.connection_for_scene(&scene).unwrap();
        let input = ContinueFromRecord {
            scene_id: scene,
            source_run_id: run.run_id,
            expected_journal_revision: selection.expected_journal_revision,
            expected_checkpoint_seq: selection.expected_checkpoint_seq,
            expected_connection_id: connection.id,
            expected_connection_revision: connection.revision,
            selected_event_ids: selection.selected_event_ids,
        };
        let before = store.snapshot();
        assert!(store.begin_recorded_continuation(input.clone()).is_err());
        assert_eq!(store.snapshot(), before);
        assert_eq!(denied.load(Ordering::SeqCst), 2);
        store
            .db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();

        // The accepted begin path reads exactly the selected body, not the
        // other eligible body. Each direct event query is prepared separately.
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        store
            .db
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "agent_run_events",
                        column_name: "content",
                    }
                ) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
                Authorization::Allow
            }))
            .unwrap();
        store.begin_recorded_continuation(input).unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        store
            .db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
    }
}
