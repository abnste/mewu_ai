// SPDX-License-Identifier: MPL-2.0
use super::*;

impl Store {
    /// Remove a profile from active use. Keep identities required by durable
    /// history/remote receipts archived, without rewriting their provenance.
    /// Host cancels ended senders after publishing the returned snapshot.
    pub fn delete_agent(&mut self, agent_id: &str, replacement_agent_id: &str) -> Result<Snapshot> {
        if agent_id == replacement_agent_id {
            return Err(invalid("请选择不同的替代 Agent"));
        }
        self.journal_transaction(|db,state| {
            if !state.agents.iter().any(|a|a.id == agent_id && !a.archived) { return Err(not_found("Agent",agent_id)); }
            if !state.agents.iter().any(|a|a.id == replacement_agent_id && !a.archived) { return Err(not_found("替代 Agent",replacement_agent_id)); }
            if state.scenes.iter().any(|s|s.agent_id == agent_id && is_running(s)) { return Err(CoreError::RunInProgress); }
            let has_journal:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_run_records WHERE json_extract(data,'$.agent')=?1)",[agent_id],|r|r.get(0))?;
            let has_external:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM external_memory_bindings WHERE agent_id=?1)",[agent_id],|r|r.get(0))?;
            let preserve = has_journal || has_external || state.scenes.iter().any(|s|s.agent_id == agent_id && !s.messages.is_empty());
            let mut revoke=HashSet::new();
            for server in &mut state.mcp_servers {
                if server.grants.iter().any(|g|g.agent_id == agent_id) {
                    revoke.extend(granted_agents(server));
                    server.revision=next_mcp_revision(server.revision)?;
                    server.grants.retain(|g|g.agent_id != agent_id);
                }
            }
            cancel_mcp_runs(state,&revoke)?;
            if preserve {
                let agent = state.agents.iter_mut().find(|a|a.id == agent_id).expect("validated profile");
                agent.archived = true;
                agent.memory_enabled = false;
            } else {
                db.execute("DELETE FROM memory_records WHERE agent_id=?1",[agent_id])?;
                db.execute("DELETE FROM memory_state WHERE agent_id=?1",[agent_id])?;
                state.memories.retain(|m|m.agent_id != agent_id);
                state.agents.retain(|a|a.id != agent_id);
            }
            for target in state.scenes.iter_mut().filter(|s|s.agent_id == agent_id && (!preserve || s.messages.is_empty())) {
                target.agent_id=replacement_agent_id.into();
                touch(target);
            }
            Ok(())
        })?;
        Ok(self.snapshot())
    }
}
