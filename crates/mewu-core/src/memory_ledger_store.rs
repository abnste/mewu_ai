// SPDX-License-Identifier: MPL-2.0
//! Store-owned transactions for the external memory ledger; no network I/O.
use super::*;
use crate::external_memory as ledger;
use rusqlite::OptionalExtension;

pub(super) fn cancel_authority(
    state: &mut Snapshot,
    binding: Option<&str>,
    agent: Option<&str>,
) -> Result<()> {
    for scene in &mut state.scenes {
        if !is_running(scene) || agent.is_some_and(|id| scene.agent_id != id) {
            continue;
        }
        let matches = match scene.run.as_ref().and_then(|r| r.memory_authority.as_ref()) {
            Some(RunMemoryAuthority::External { grant, .. }) => {
                binding.is_none_or(|id| grant.binding_id == id)
            }
            Some(RunMemoryAuthority::Local { .. }) => binding.is_none(),
            None => false,
        };
        if matches {
            let run = scene.run.as_mut().expect("checked running run");
            run.status = RunStatus::Canceled;
            run.error = Some("记忆来源或授权已改变".into());
            settle_steps(run, "记忆来源或授权已改变");
            archive_steps(scene)?;
            touch(scene);
        }
    }
    Ok(())
}

impl Store {
    /// Ledger mutations share the app_state revision fence, but do not clone or
    /// serialize the scene collection for background observations.
    fn ledger_transaction<T>(
        &mut self,
        mut next: Option<Snapshot>,
        change: impl FnOnce(&SqlConnection, &Snapshot) -> Result<T>,
    ) -> Result<T> {
        if let Some(state) = &next {
            validate_snapshot(state)?;
        }
        let transaction = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = transaction.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let app_id: i64 = transaction.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if version != DB_SCHEMA_VERSION {
            return Err(CoreError::UnsupportedSchema(version as u64));
        }
        if app_id != APPLICATION_ID {
            return Err(invalid("数据库应用标识已改变"));
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM app_state WHERE id=1 AND schema_version=1",
            [],
            |r| r.get(0),
        )?;
        if revision != self.revision {
            return Err(CoreError::ConcurrentModification);
        }
        let state = next.as_ref().unwrap_or(&self.state);
        let before = transaction.total_changes();
        let result = change(&transaction, state)?;
        if next.is_none() && transaction.total_changes() == before {
            transaction.commit()?;
            return Ok(result);
        }
        if let Some(next) = &next {
            ledger::on_snapshot_change(&transaction, &self.state, next)?;
        }
        if let Some(next) = &mut next {
            crate::journal::on_snapshot_change(&transaction, &self.state, next)?;
            validate_snapshot(next)?;
        }
        let changed = if let Some(next) = &next {
            transaction.execute(
                "UPDATE app_state SET payload=?1,revision=revision+1 WHERE id=1 AND revision=?2",
                params![serde_json::to_string(next)?, self.revision],
            )?
        } else {
            transaction.execute(
                "UPDATE app_state SET revision=revision+1 WHERE id=1 AND revision=?1",
                [self.revision],
            )?
        };
        if changed != 1 {
            return Err(CoreError::ConcurrentModification);
        }
        transaction.commit()?;
        self.revision += 1;
        if let Some(next) = next {
            self.state = next;
        }
        Ok(result)
    }

    pub fn memory_provider_status(
        &self,
        agent_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<AgentMemoryStatus> {
        let tx = self.db.unchecked_transaction()?;
        let result = ledger::status(&tx, &self.state, agent_id, cursor, limit)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn memory_binding(&self, binding_id: &str) -> Result<MemoryBindingView> {
        let tx = self.db.unchecked_transaction()?;
        let result = ledger::binding(&tx, binding_id)?.view(&tx)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn external_memory_page(
        &self,
        agent_id: &str,
        binding_id: &str,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MemoryEvidencePage> {
        let tx = self.db.unchecked_transaction()?;
        let result = ledger::page(&tx, &self.state, agent_id, binding_id, query, cursor, limit)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn external_memory_entry(
        &self,
        agent_id: &str,
        binding_id: &str,
        evidence_id: &str,
    ) -> Result<Option<MemoryEvidenceDetail>> {
        let tx = self.db.unchecked_transaction()?;
        let result = ledger::entry(&tx, &self.state, agent_id, binding_id, evidence_id)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn run_memory_authority(
        &self,
        scene_id: &str,
        run_id: &str,
    ) -> Result<Option<RunMemoryAuthority>> {
        let target = scene(&self.state, scene_id)?;
        let run = target
            .run
            .as_ref()
            .filter(|r| !target.closed && r.id == run_id && r.status == RunStatus::Running)
            .ok_or(CoreError::StaleRun)?;
        Ok(run.memory_authority.clone())
    }

    pub fn reserve_memory_binding(
        &mut self,
        expected_selection_revision: u64,
        input: HostNewMemoryBinding,
    ) -> Result<ReserveBindingReceipt> {
        ledger::uuid(&input.id)?;
        if let Some(id) = &input.credential_id {
            ledger::uuid(id)?;
        }
        for (s, max) in [
            (&input.endpoint, 4096),
            (&input.plugin_id, 200),
            (&input.contribution_id, 200),
        ] {
            if s.trim() != s || s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
                return Err(invalid("记忆来源配置无效"));
            }
        }
        if input.plugin_revision == 0 {
            return Err(invalid("记忆插件版本无效"));
        }
        ledger::integer(input.plugin_revision)?;
        let selection = ledger::selection(&self.state, &input.agent_id)?;
        if selection.revision != expected_selection_revision {
            return Err(CoreError::MemoryProviderConflict);
        }
        self.ledger_transaction(None, |db,_| {
            db.execute("INSERT INTO external_memory_bindings(id,agent_id,revision,ledger_revision,plugin_id,contribution_id,plugin_revision,endpoint,credential_id,bank_id,enabled,status,policy_json,created_at) VALUES(?1,?2,1,0,?3,?4,?5,?6,?7,?8,1,'provisioning',?9,?10)", params![input.id,input.agent_id,input.plugin_id,input.contribution_id,ledger::integer(input.plugin_revision)?,input.endpoint,input.credential_id,format!("mewu-{}",Uuid::parse_str(&input.id).expect("validated").simple()),ledger::encode(&input.policy)?,ledger::integer(ledger::now_ms())?])?;
            let b = ledger::binding(db,&input.id)?;
            let operation_id = ledger::enqueue(db,&b.grant(),None,"create","create_bank",None)?;
            Ok(ReserveBindingReceipt { binding:ledger::binding(db,&input.id)?.view(db)?, operation_id, selection_revision:selection.revision })
        })
    }

    pub fn select_memory_provider(
        &mut self,
        agent_id: &str,
        expected_selection_revision: u64,
        binding_id: Option<&str>,
        expected_binding_revision: Option<u64>,
    ) -> Result<MemoryProviderSelection> {
        let previous = ledger::selection(&self.state, agent_id)?;
        if previous.revision != expected_selection_revision {
            return Err(CoreError::MemoryProviderConflict);
        }
        if let Some(id) = binding_id {
            let b = ledger::binding(&self.db, id)?;
            if b.agent_id != agent_id
                || Some(b.revision) != expected_binding_revision
                || !b.enabled
                || b.status != BindingStatus::Ready
            {
                return Err(CoreError::MemoryProviderConflict);
            }
        } else if expected_binding_revision.is_some() {
            return Err(CoreError::MemoryProviderConflict);
        }
        if previous.binding_id.as_deref() == binding_id {
            return Ok(previous);
        }
        let selection = MemoryProviderSelection {
            revision: ledger::bump(previous.revision)?,
            binding_id: binding_id.map(str::to_owned),
        };
        let mut next = self.state.clone();
        next.agents
            .iter_mut()
            .find(|a| a.id == agent_id)
            .expect("validated")
            .memory_provider = selection.clone();
        cancel_authority(&mut next, None, Some(agent_id))?;
        self.ledger_transaction(Some(next), |db, _| {
            if let Some(id) = previous.binding_id {
                ledger::block_unclaimed(db, &id, MemoryReason::AuthorityChanged)?;
            }
            Ok(selection)
        })
    }

    pub fn set_memory_binding_policy(
        &mut self,
        binding_id: &str,
        expected_revision: u64,
        current_plugin_revision: u64,
        enabled: bool,
        policy: MemoryPolicy,
    ) -> Result<BindingMutationReceipt> {
        let old = ledger::binding(&self.db, binding_id)?;
        if old.revision != expected_revision
            || old.status == BindingStatus::Retired
            || current_plugin_revision == 0
        {
            return Err(CoreError::MemoryProviderConflict);
        }
        ledger::integer(current_plugin_revision)?;
        if old.enabled == enabled
            && old.policy == policy
            && old.plugin_revision == current_plugin_revision
        {
            return Ok(BindingMutationReceipt {
                binding: old.view(&self.db)?,
                selection: ledger::selection(&self.state, &old.agent_id)?,
            });
        }
        let mut next = self.state.clone();
        cancel_authority(&mut next, Some(binding_id), None)?;
        self.ledger_transaction(Some(next),|db,state| {
            ledger::block_unclaimed(db,binding_id,MemoryReason::AuthorityChanged)?;
            db.execute("UPDATE external_memory_bindings SET revision=?1,plugin_revision=?2,enabled=?3,policy_json=?4,status=CASE WHEN status='ready' AND ?3=0 THEN 'suspended' WHEN status='suspended' AND ?3=1 THEN 'ready' ELSE status END,ledger_revision=ledger_revision+1 WHERE id=?5",params![ledger::integer(ledger::bump(old.revision)?)?,ledger::integer(current_plugin_revision)?,enabled,ledger::encode(&policy)?,binding_id])?;
            if !enabled { db.execute("UPDATE external_memory_tombstones SET remote_state='needs_authorization' WHERE binding_id=?1 AND remote_state!='confirmed_deleted'",[binding_id])?; }
            Ok(BindingMutationReceipt{binding:ledger::binding(db,binding_id)?.view(db)?,selection:ledger::selection(state,&old.agent_id)?})
        })
    }
    pub fn retire_memory_binding(
        &mut self,
        binding_id: &str,
        expected_revision: u64,
    ) -> Result<BindingMutationReceipt> {
        let old = ledger::binding(&self.db, binding_id)?;
        if old.revision != expected_revision {
            return Err(CoreError::MemoryProviderConflict);
        }
        if old.status == BindingStatus::Retired {
            return Ok(BindingMutationReceipt {
                binding: old.view(&self.db)?,
                selection: ledger::selection(&self.state, &old.agent_id)?,
            });
        }
        let mut next = self.state.clone();
        let agent = next
            .agents
            .iter_mut()
            .find(|a| a.id == old.agent_id)
            .ok_or(CoreError::MemoryProviderConflict)?;
        if agent.memory_provider.binding_id.as_deref() == Some(binding_id) {
            agent.memory_provider = MemoryProviderSelection {
                revision: ledger::bump(agent.memory_provider.revision)?,
                binding_id: None,
            };
        }
        cancel_authority(&mut next, Some(binding_id), None)?;
        self.ledger_transaction(Some(next),|db,state| {
            ledger::block_unclaimed(db,binding_id,MemoryReason::AuthorityChanged)?;
            db.execute("UPDATE external_memory_bindings SET revision=?1,enabled=0,status='retired',ledger_revision=ledger_revision+1 WHERE id=?2",params![ledger::integer(ledger::bump(old.revision)?)?,binding_id])?;
            db.execute("UPDATE external_memory_tombstones SET remote_state='needs_authorization' WHERE binding_id=?1 AND remote_state!='confirmed_deleted'",[binding_id])?;
            Ok(BindingMutationReceipt{binding:ledger::binding(db,binding_id)?.view(db)?,selection:ledger::selection(state,&old.agent_id)?})
        })
    }

    pub fn queue_manual_memory_evidence(
        &mut self,
        agent_id: &str,
        binding_id: &str,
        expected_revision: u64,
        request_id: &str,
        text: &str,
    ) -> Result<WriteReceipt> {
        ledger::uuid(request_id)?;
        bounded_text(text, 64 * 1024, "记忆内容")?;
        let b = ledger::binding(&self.db, binding_id)?;
        if b.agent_id != agent_id
            || b.revision != expected_revision
            || !ledger::usable(&self.state, &b)
            || !b.policy.explicit_retain
        {
            return Err(CoreError::MemoryProviderConflict);
        }
        let body = serde_json::to_string(
            &serde_json::json!({"version":1,"parts":[{"role":"user","text":text}]}),
        )?;
        let source = EvidenceSource {
            origin: EvidenceOrigin::Manual,
            scene_id: None,
            user_message_id: None,
            assistant_message_id: None,
            run_id: None,
        };
        self.ledger_transaction(None, |db, _| {
            ledger::queue_evidence(
                db,
                &b,
                &format!("request:{request_id}"),
                source,
                Some(body),
                None,
            )
        })
    }
    pub fn queue_memory_quote_for_run(
        &mut self,
        scene_id: &str,
        run_id: &str,
        grant: &MemoryAuthorityGrant,
        request_id: &str,
        user_quote: &str,
    ) -> Result<WriteReceipt> {
        ledger::uuid(request_id)?;
        bounded_text(user_quote, 64 * 1024, "记忆原文")?;
        let b = ledger::run_binding(&self.db, &self.state, scene_id, run_id, grant)?;
        if !b.policy.explicit_retain {
            return Err(CoreError::MemoryDisabled);
        }
        let target = scene(&self.state, scene_id)?;
        let user = target
            .messages
            .iter()
            .find(|m| m.role == MessageRole::User && m.run_id.as_deref() == Some(run_id))
            .ok_or(CoreError::StaleRun)?;
        if !user.text.contains(user_quote) {
            return Err(invalid("只能保存本轮用户原文中的内容"));
        }
        let source = EvidenceSource {
            origin: EvidenceOrigin::UserQuote,
            scene_id: Some(scene_id.into()),
            user_message_id: Some(user.id.clone()),
            assistant_message_id: None,
            run_id: Some(run_id.into()),
        };
        let text = serde_json::to_string(
            &serde_json::json!({"version":1,"occurredAt":user.created_at,"parts":[{"role":"user","text":user_quote}]}),
        )?;
        self.ledger_transaction(None, |db, _| {
            ledger::queue_evidence(
                db,
                &b,
                &format!("request:{request_id}"),
                source,
                Some(text),
                None,
            )
        })
    }

    pub fn forget_memory_evidence(
        &mut self,
        agent_id: &str,
        binding_id: &str,
        evidence_id: &str,
        expected_revision: u64,
    ) -> Result<ForgetReceipt> {
        let b = ledger::binding(&self.db, binding_id)?;
        if b.agent_id != agent_id {
            return Err(CoreError::MemoryProviderConflict);
        }
        let e = ledger::evidence(&self.db, evidence_id)?
            .filter(|e| e.binding_id == binding_id)
            .ok_or_else(|| not_found("记忆证据", evidence_id))?;
        if e.revision != expected_revision {
            return Err(CoreError::MemoryConflict);
        }
        if let Some(receipt) = ledger::forgotten(&self.db, &e)? {
            return Ok(receipt);
        }
        let mut next = self.state.clone();
        cancel_authority(&mut next, Some(binding_id), None)?;
        self.ledger_transaction(Some(next), |db, state| {
            forget_transaction(db, state, &b, &e)
        })
    }

    pub fn pending_memory_work(&self, now_ms: u64, limit: usize) -> Result<Vec<PendingMemoryWork>> {
        ledger::pending(&self.db, &self.state, now_ms, limit)
    }
    pub fn claim_memory_work(
        &mut self,
        operation_id: &str,
        expected_operation_revision: u64,
        grant: &MemoryAuthorityGrant,
        now_ms: u64,
    ) -> Result<Option<MemoryDispatch>> {
        self.ledger_transaction(None, |db, state| {
            ledger::claim(
                db,
                state,
                operation_id,
                expected_operation_revision,
                grant,
                now_ms,
            )
        })
    }
    pub fn record_memory_work(
        &mut self,
        lease: MemoryWorkLease,
        observation: MemoryWorkObservation,
        now_ms: u64,
    ) -> Result<MemoryWorkReceipt> {
        self.ledger_transaction(None, |db, _| ledger::record(db, lease, observation, now_ms))
    }
    pub fn memory_work_is_authorized(
        &self,
        lease: &MemoryWorkLease,
        grant: &MemoryAuthorityGrant,
    ) -> Result<bool> {
        ledger::lease_authorized(&self.db, &self.state, lease, grant)
    }
    pub fn recover_memory_work(&mut self) -> Result<MemoryRecoverySummary> {
        self.ledger_transaction(None, |db, _| ledger::recover(db))
    }
    pub fn memory_route_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        grant: &MemoryAuthorityGrant,
    ) -> Result<MemoryRoute> {
        Ok(ledger::run_binding(&self.db, &self.state, scene_id, run_id, grant)?.route())
    }
    pub fn allowed_memory_documents_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        grant: &MemoryAuthorityGrant,
        candidate_document_ids: &[String],
    ) -> Result<Vec<AllowedMemoryDocument>> {
        let b = ledger::run_binding(&self.db, &self.state, scene_id, run_id, grant)?;
        if !b.policy.recall {
            return Err(CoreError::MemoryDisabled);
        }
        if candidate_document_ids.len() > 128
            || candidate_document_ids.iter().any(|s| s.len() > 200)
        {
            return Err(invalid("记忆召回候选过多"));
        }
        let mut found = HashSet::new();
        let mut result = Vec::new();
        let mut query=self.db.prepare("SELECT e.id,e.revision,e.source_json FROM external_memory_evidence e WHERE e.binding_id=?1 AND e.document_id=?2 AND e.state='committed' AND NOT EXISTS(SELECT 1 FROM external_memory_tombstones t WHERE t.evidence_id=e.id)")?;
        for doc in candidate_document_ids {
            if !found.insert(doc) {
                continue;
            }
            let row: Option<(String, i64, String)> = query
                .query_row(params![b.id, doc], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            if let Some((id, revision, source)) = row {
                result.push(AllowedMemoryDocument {
                    document_id: doc.clone(),
                    evidence_id: id,
                    evidence_revision: u64::try_from(revision)
                        .map_err(|_| invalid("记忆版本无效"))?,
                    source: serde_json::from_str(&source)?,
                });
            }
        }
        Ok(result)
    }
}

impl ExportView {
    pub fn memory_provider_status(
        &self,
        agent_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<AgentMemoryStatus> {
        ledger::status(&self.db, &self.state, agent_id, cursor, limit)
    }
    pub fn external_memory_page(
        &self,
        agent_id: &str,
        binding_id: &str,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MemoryEvidencePage> {
        ledger::page(
            &self.db,
            &self.state,
            agent_id,
            binding_id,
            query,
            cursor,
            limit,
        )
    }
    pub fn external_memory_entry(
        &self,
        agent_id: &str,
        binding_id: &str,
        evidence_id: &str,
    ) -> Result<Option<MemoryEvidenceDetail>> {
        ledger::entry(&self.db, &self.state, agent_id, binding_id, evidence_id)
    }
}

pub(super) fn forget_transaction(
    db: &SqlConnection,
    state: &Snapshot,
    b: &ledger::Binding,
    e: &ledger::Evidence,
) -> Result<ForgetReceipt> {
    let binding_id = b.id.as_str();
    let evidence_id = e.id.as_str();

    let prior:Option<(String,String,Option<String>,Option<String>,String)>=db.query_row("SELECT id,state,lease,remote_state,action FROM external_memory_outbox WHERE evidence_id=?1 AND kind='retain'",[evidence_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let unsent = prior
        .as_ref()
        .is_none_or(|(_, state, lease, remote, action)| {
            action == "retain"
                && ["queued", "blocked"].contains(&state.as_str())
                && lease.is_none()
                && remote.is_none()
        });
    if unsent {
        db.execute("UPDATE external_memory_outbox SET state='blocked',revision=revision+1,reason_json=?1 WHERE evidence_id=?2 AND kind='retain'",params![ledger::encode(&MemoryReason::CancelledBeforeDispatch)?,evidence_id])?;
    }
    let authorized = ledger::usable(state, &b);
    let remote = if unsent {
        "confirmed_deleted"
    } else if authorized {
        "pending"
    } else {
        "needs_authorization"
    };
    db.execute("UPDATE external_memory_evidence SET text=NULL,payload_bytes=0,state='suppressed',revision=revision+1,updated_at=?1 WHERE id=?2",params![ledger::integer(ledger::now_ms())?,evidence_id])?;
    db.execute("INSERT INTO external_memory_tombstones(evidence_id,binding_id,document_id,created_at,remote_state,quiescence) VALUES(?1,?2,?3,?4,?5,?6)",params![evidence_id,binding_id,e.document_id,ledger::integer(ledger::now_ms())?,remote,if unsent{"proven"}else{"unproven"}])?;
    if !unsent {
        let action = if prior.as_ref().is_some_and(|(_, state, _, remote, _)| {
            state == "completed" && remote.as_deref() == Some("completed")
        }) {
            "delete_document"
        } else {
            "cancel_retain"
        };
        ledger::enqueue(db, &b.grant(), Some(evidence_id), "delete", action, None)?;
    }
    ledger::forgotten(
        db,
        &ledger::evidence(db, evidence_id)?.expect("existing evidence"),
    )?
    .ok_or(CoreError::MemoryWorkConflict)
}
