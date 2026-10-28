// SPDX-License-Identifier: MPL-2.0
//! Public tool invocations/results, never private provider continuation or
//! authority to replay a historical action. Legacy missing data stays explicit.
use super::*;
use crate::{journal as j, DispatchLease, ReceiptKind, ToolBinding};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

pub(crate) fn migrate(db: &SqlConnection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_tool_transcripts (
        event_id TEXT PRIMARY KEY NOT NULL, run_id TEXT NOT NULL,
        arguments_json TEXT NOT NULL CHECK(length(CAST(arguments_json AS BLOB))<=524288),
        result_json TEXT CHECK(result_json IS NULL OR length(CAST(result_json AS BLOB))<=131072)
    ); CREATE INDEX IF NOT EXISTS agent_tool_transcripts_run ON agent_tool_transcripts(run_id);",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CompletePublicTurn, ContentUnavailable, JournalContent, ModelDispatchInput,
        ObservedToolResult, ProposedTool, SceneCommand, ToolResponseKind,
    };
    fn invoke(store: &mut Store) -> (String, String, DispatchLease, String) {
        let scene = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: scene.clone(),
                draft: "执行一次真实测试".into(),
            })
            .unwrap();
        let run = store.begin_run(&scene).unwrap().run_id;
        let model = store
            .begin_model_request(
                &scene,
                &run,
                ModelDispatchInput {
                    round: 0,
                    projected_request_sha256: "a".repeat(64),
                },
            )
            .unwrap();
        let args = json!({"command":"printf '中文'","cwd":"/tmp/space name"}).to_string();
        let turn = store
            .record_model_turn(
                &model,
                CompletePublicTurn {
                    text: String::new(),
                    calls: vec![ProposedTool {
                        call_id: "command".into(),
                        binding: ToolBinding::Local {
                            name: "bash".into(),
                        },
                        arguments_wire_sha256: j::digest(args.as_bytes()),
                    }],
                },
            )
            .unwrap();
        (scene, run, turn.tools[0].lease.clone(), args)
    }
    #[test]
    fn actual_arguments_and_failed_output_survive_reopen_without_dispatch_or_replay() {
        let root =
            std::env::temp_dir().join(format!("mewu-tool-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("store.db");
        let mut store = Store::open(&file).unwrap();
        let (scene, run, lease, args) = invoke(&mut store);
        assert!(store
            .retain_tool_arguments(&[(lease.clone(), "{}".into())])
            .is_err());
        store
            .retain_tool_arguments(&[(lease.clone(), args.clone())])
            .unwrap();
        store
            .retain_tool_arguments(&[(lease.clone(), args.clone())])
            .unwrap();
        let before = store
            .agent_tool_history(&scene, &run, 0, 2, None, 0)
            .unwrap();
        assert_eq!(
            before["entries"][0]["arguments"]["command"],
            "printf '中文'"
        );
        assert_eq!(before["entries"][0]["phase"], "prepared");
        store.mark_tool_dispatched(&lease).unwrap();
        let result =
            json!({"output":"报错\n".repeat(4000),"exitCode":3,"isError":true}).to_string();
        store
            .record_tool_response(
                &lease,
                ObservedToolResult {
                    response_kind: ToolResponseKind::LocalRead,
                    returned_error: Some(true),
                    observed_encoding: "json".into(),
                    observed_bytes: result.len() as u64,
                    observed_sha256: j::digest(result.as_bytes()),
                    retained_content: JournalContent::Omitted {
                        reason: ContentUnavailable::UnsupportedContent,
                    },
                    references: vec![],
                    model_visible_json: Some(result.clone()),
                },
            )
            .unwrap();
        let id = before["entries"][0]["toolEntryId"]
            .as_str()
            .unwrap()
            .to_owned();
        store.fail_run(&scene, &run, "合成结束").unwrap();
        drop(store);
        let mut store = Store::open(&file).unwrap();
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let page = store
                .agent_tool_history(&scene, &run, 0, 1, Some(&id), offset)
                .unwrap();
            restored.push_str(page["content"].as_str().unwrap());
            match page["nextOffset"].as_u64() {
                Some(n) => offset = n as usize,
                None => break,
            }
        }
        let restored: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(restored["argumentsJson"], args);
        assert_eq!(restored["resultJson"], result);
        assert_eq!(restored["returnedError"], true);
        assert!(store.mark_tool_dispatched(&lease).is_err());
        store.apply(SceneCommand::NewScene).unwrap();
        let other = store.snapshot().active_scene_id;
        assert!(store
            .agent_tool_history(&other, &run, 0, 1, None, 0)
            .is_err());
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
pub(crate) fn validate(db: &SqlConnection) -> Result<()> {
    db.prepare(
        "SELECT event_id,run_id,arguments_json,result_json FROM agent_tool_transcripts LIMIT 0",
    )?;
    Ok(())
}
fn excerpt(value: &str, limit: usize) -> Value {
    if value.chars().count() <= limit {
        return serde_json::from_str(value).unwrap_or_else(|_| json!(value));
    }
    json!({"truncated":true,"jsonPrefix":value.chars().take(limit).collect::<String>(),"totalCharacters":value.chars().count(),"readMore":"session_read with runId/toolEntryId/offset"})
}
impl Store {
    pub fn recent_agent_tool_history(&self, scene: &str, run: &str) -> Result<Value> {
        let after=self.journal_read(|db,_|{
            let first:Option<i64>=db.query_row("SELECT min(sequence) FROM (SELECT sequence FROM agent_run_events WHERE run_id=?1 AND json_extract(metadata,'$.summary.kind')='tool' ORDER BY sequence DESC LIMIT 4)",[run],|row|row.get(0))?;
            Ok(first.unwrap_or(1).saturating_sub(1)as u64)
        })?;
        self.agent_tool_history(scene, run, after, 4, None, 0)
    }
    /// Archival binds exactly the argument bytes to the existing receipt hash.
    /// It neither dispatches nor accepts replacement tool arguments.
    pub fn retain_tool_arguments(&mut self, values: &[(DispatchLease, String)]) -> Result<()> {
        self.journal_transaction(|db,_state|{
            for (lease,args) in values {
                let (r,e,_)=j::leased(db,lease)?;
                if e.summary.kind!=ReceiptKind::Tool||args.len()>512*1024||e.arguments_sha256.as_deref()!=Some(j::digest(args.as_bytes()).as_str()){return Err(CoreError::JournalConflict);}
                let old:Option<String>=db.query_row("SELECT arguments_json FROM agent_tool_transcripts WHERE event_id=?1",[&e.summary.id],|row|row.get(0)).optional()?;
                if let Some(old)=old {if old!=*args{return Err(CoreError::JournalConflict);}}
                else{db.execute("INSERT INTO agent_tool_transcripts(event_id,run_id,arguments_json) VALUES(?1,?2,?3)",params![e.summary.id,r.id,args])?;}
            }Ok(())
        })
    }
    /// Only the originating current conversation/Agent may read these records.
    /// Another scene, prior reset conversation or forged wire hash is rejected.
    pub fn agent_tool_history(
        &self,
        scene: &str,
        run: &str,
        after: u64,
        limit: usize,
        entry: Option<&str>,
        offset: usize,
    ) -> Result<Value> {
        self.journal_read(|db,state|{
            if limit==0||limit>25||after>i64::MAX as u64{return Err(j::bad("工具记录页参数无效"));}
            let record=journal_store::owned(db,state,scene,run)?.ok_or(CoreError::JournalConflict)?;
            let current=state.scenes.iter().find(|s|s.id==scene).ok_or(CoreError::JournalConflict)?;
            if !current.messages[current.conversation_start.unwrap_or(0)..].iter().any(|m|m.run_id.as_deref()==Some(run)){return Err(CoreError::JournalConflict);}
            let mut statement=db.prepare("SELECT e.id,e.sequence,t.arguments_json,t.result_json FROM agent_run_events e LEFT JOIN agent_tool_transcripts t ON t.event_id=e.id AND t.run_id=e.run_id WHERE e.run_id=?1 AND e.sequence>?2 AND json_extract(e.metadata,'$.summary.kind')='tool' AND (?3 IS NULL OR e.id=?3) ORDER BY e.sequence LIMIT ?4")?;
            let rows=statement.query_map(params![record.id,after as i64,entry,(limit+1) as i64],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)? as u64,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
            let mut output=Vec::new();let mut last=after;let mut more=false;
            for (id,sequence,args,result) in rows {
                let (event_run,event,_)=j::event(db,&id)?;
                if event_run!=run||event.summary.kind!=ReceiptKind::Tool||event.summary.sequence!=sequence{return Err(CoreError::JournalConflict);}
                if output.len()==limit{more=true;break;}
                if let Some(args)=&args {if event.arguments_sha256.as_deref()!=Some(j::digest(args.as_bytes()).as_str()){return Err(CoreError::JournalConflict);}}
                if event.summary.observed_encoding.as_deref()==Some("mewu-local-json-v1") {
                    if let Some(body)=&result{if event.summary.observed_sha256.as_deref()!=Some(j::digest(body.as_bytes()).as_str())||event.summary.observed_bytes!=Some(body.len()as u64){return Err(CoreError::JournalConflict);}}
                }
                let tool=match &event.binding {Some(ToolBinding::Local{name})=>name.clone(),_=>event.summary.label.clone()};
                if entry.is_some(){
                    let full=json!({"runId":run,"toolEntryId":id,"tool":tool,"phase":event.summary.phase,"returnedError":event.summary.returned_error,"argumentsJson":args,"resultJson":result,"readOnlyEvidence":true}).to_string();
                    let total=full.chars().count();if offset>total{return Err(j::bad("offset超出记录长度"));}
                    let content=full.chars().skip(offset).take(8000).collect::<String>();let end=offset+content.chars().count();
                    return Ok(json!({"content":content,"offset":offset,"nextOffset":(end<total).then_some(end),"totalCharacters":total,"readOnlyEvidence":true}));
                }
                let data=json!({"runId":run,"toolEntryId":id,"sequence":sequence,"tool":tool,"phase":event.summary.phase,"returnedError":event.summary.returned_error,"arguments":args.as_deref().map(|s|excerpt(s,1000)),"result":result.as_deref().map(|s|excerpt(s,2000)),"legacyNotRecorded":args.is_none(),"readOnlyEvidence":true});
                last=sequence;output.push(data);
            }
            if entry.is_some(){return Err(CoreError::JournalConflict);}
            Ok(json!({"runId":run,"entries":output,"nextAfter":more.then_some(last),"readOnlyEvidence":true}))
        })
    }
}
