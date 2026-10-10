// SPDX-License-Identifier: MPL-2.0
use super::*;

impl Store {
    /// Delete a local profile only when it has no durable execution provenance
    /// or external-memory ownership. Historical conversations/assets stay intact.
    /// Host cancels ended senders after publishing the returned snapshot.
    pub fn delete_agent(&mut self, agent_id: &str, replacement_agent_id: &str) -> Result<Snapshot> {
        if agent_id == replacement_agent_id {
            return Err(invalid("请选择不同的替代 Agent"));
        }
        self.journal_transaction(|db,state| {
            if !state.agents.iter().any(|a|a.id == agent_id) { return Err(not_found("Agent",agent_id)); }
            if !state.agents.iter().any(|a|a.id == replacement_agent_id) { return Err(not_found("替代 Agent",replacement_agent_id)); }
            if state.scenes.iter().any(|s|s.agent_id == agent_id && is_running(s)) { return Err(CoreError::RunInProgress); }
            let has_journal:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_run_records WHERE json_extract(data,'$.agent')=?1)",[agent_id],|r|r.get(0))?;
            if has_journal { return Err(invalid("此 Agent 有执行记录，不能删除其来源身份")); }
            let has_external:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_bindings WHERE agent_id=?1)",[agent_id],|r|r.get(0))?;
            if has_external { return Err(invalid("此 Agent 有外接记忆绑定或待处理记录，不能直接删除")); }
            let mut revoke=HashSet::new();
            for server in &mut state.mcp_servers {
                if server.grants.iter().any(|g|g.agent_id == agent_id) {
                    revoke.extend(granted_agents(server));
                    server.revision=next_mcp_revision(server.revision)?;
                    server.grants.retain(|g|g.agent_id != agent_id);
                }
            }
            cancel_mcp_runs(state,&revoke)?;
            db.execute("DELETE FROM memory_records WHERE agent_id=?1",[agent_id])?;
            db.execute("DELETE FROM memory_state WHERE agent_id=?1",[agent_id])?;
            state.memories.retain(|m|m.agent_id != agent_id);
            state.agents.retain(|a|a.id != agent_id);
            for target in state.scenes.iter_mut().filter(|s|s.agent_id == agent_id) {
                target.agent_id=replacement_agent_id.into();
                touch(target);
            }
            Ok(())
        })?;
        Ok(self.snapshot())
    }
}
