// SPDX-License-Identifier: MPL-2.0
//! First-turn titles are optional metadata, never another chat turn.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTitleRequest {
    pub scene_id: String,
    pub run_id: String,
    pub message_id: String,
    pub prompt: String,
    pub title: String,
    pub title_revision: u64,
    pub conversation_start: usize,
}

pub fn title_request_is_current(state: &Snapshot, request: &SessionTitleRequest) -> bool {
    state
        .scenes
        .iter()
        .find(|s| s.id == request.scene_id)
        .is_some_and(|s| {
            !s.closed
                && s.auto_title_attempted
                && s.title_revision == request.title_revision
                && s.title == request.title
                && s.conversation_start.unwrap_or(0) == request.conversation_start
                && s.run
                    .as_ref()
                    .is_some_and(|r| r.id == request.run_id && r.status == RunStatus::Completed)
                && s.messages
                    .iter()
                    .skip(request.conversation_start)
                    .find(|m| m.id == request.message_id)
                    .is_some_and(|m| {
                        m.id == request.message_id && m.run_id.as_deref() == Some(&request.run_id)
                    })
        })
}

impl Store {
    pub fn session_title_request_is_current(&self, request: &SessionTitleRequest) -> bool {
        title_request_is_current(&self.state, request)
    }
    /// Durable once-only admission; restarting never replays a paid title request.
    pub fn claim_session_title(
        &mut self,
        scene_id: &str,
        run_id: &str,
    ) -> Result<Option<SessionTitleRequest>> {
        let target = scene(&self.state, scene_id)?;
        if target.closed
            || target.auto_title_attempted
            || !matches!(target.title.as_str(), "新场景" | "黑板")
            || !target.run.as_ref().is_some_and(|r| {
                r.id == run_id
                    && r.kind == RecordedRunKind::Chat
                    && r.status == RunStatus::Completed
            })
        {
            return Ok(None);
        }
        // Ignore host-generated workflow prompts and continuation labels. Old
        // messages without a journal conservatively count as existing history.
        let mut first = None;
        for message in target
            .messages
            .iter()
            .skip(target.conversation_start.unwrap_or(0))
            .filter(|m| m.role == MessageRole::User && !m.text.trim().is_empty())
        {
            if let Some(id) = &message.run_id {
                if crate::journal::record(&self.db, id)?
                    .is_some_and(|r| r.kind != RecordedRunKind::Chat)
                {
                    continue;
                }
            }
            first = Some(message);
            break;
        }
        let Some(message) = first.filter(|m| m.run_id.as_deref() == Some(run_id)) else {
            return Ok(None);
        };
        if !target
            .messages
            .iter()
            .any(|m| m.role == MessageRole::Assistant && m.run_id.as_deref() == Some(run_id))
        {
            return Ok(None);
        }
        let request = SessionTitleRequest {
            scene_id: scene_id.into(),
            run_id: run_id.into(),
            message_id: message.id.clone(),
            prompt: message.text.chars().take(4000).collect(),
            title: target.title.clone(),
            title_revision: target.title_revision,
            conversation_start: target.conversation_start.unwrap_or(0),
        };
        let mut next = self.state.clone();
        scene_mut(&mut next, scene_id)?.auto_title_attempted = true;
        self.persist(next)?;
        Ok(Some(request))
    }

    pub fn finish_session_title(
        &mut self,
        request: &SessionTitleRequest,
        title: String,
    ) -> Result<Snapshot> {
        if !title_request_is_current(&self.state, request) {
            return Err(CoreError::StaleRun);
        }
        if title.trim().is_empty()
            || title.chars().count() > 64
            || title.contains(['\n', '\r', '\0'])
        {
            return Err(invalid("会话标题格式无效"));
        }
        let mut next = self.state.clone();
        let target = scene_mut(&mut next, &request.scene_id)?;
        target.title = title.trim().into();
        target.title_revision = target
            .title_revision
            .checked_add(1)
            .ok_or_else(|| invalid("会话标题版本已达到上限"))?;
        // Do not reorder history or change messages, drafts, references or the run.
        self.persist(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready(store: &mut Store) -> (String, String) {
        let id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "帮我规划项目的发布流程".into(),
            })
            .unwrap();
        let run = store.begin_run(&id).unwrap();
        store.finish_run(&id, &run.run_id, "正常回答").unwrap();
        (id, run.run_id)
    }
    #[test]
    fn once_only_metadata_does_not_add_a_turn() {
        let mut store = Store::open_in_memory().unwrap();
        let (id, run) = ready(&mut store);
        let before = store.snapshot().scenes[0].clone();
        let request = store.claim_session_title(&id, &run).unwrap().unwrap();
        assert_eq!(request.prompt, "帮我规划项目的发布流程");
        assert!(store.claim_session_title(&id, &run).unwrap().is_none());
        let after = store
            .finish_session_title(&request, "项目发布流程".into())
            .unwrap()
            .scenes[0]
            .clone();
        assert_eq!(after.title, "项目发布流程");
        assert_eq!(after.messages, before.messages);
        assert_eq!(after.run, before.run);
        assert_eq!(after.updated_at, before.updated_at);
        assert!(store
            .finish_session_title(&request, "重复结果".into())
            .is_err());
    }
    #[test]
    fn rename_even_to_same_title_blocks_late_generation() {
        let mut store = Store::open_in_memory().unwrap();
        let (id, run) = ready(&mut store);
        let request = store.claim_session_title(&id, &run).unwrap().unwrap();
        store
            .apply(SceneCommand::RenameScene {
                scene_id: id,
                title: request.title.clone(),
            })
            .unwrap();
        assert!(!title_request_is_current(&store.snapshot(), &request));
        assert!(store
            .finish_session_title(&request, "迟到标题".into())
            .is_err());
    }
    #[test]
    fn minimized_can_finish_but_closed_or_new_conversation_cannot() {
        for mode in 0..3 {
            let mut store = Store::open_in_memory().unwrap();
            let (id, run) = ready(&mut store);
            let request = store.claim_session_title(&id, &run).unwrap().unwrap();
            store
                .apply(match mode {
                    0 => SceneCommand::FreezeScene { scene_id: id },
                    1 => SceneCommand::CloseScene { scene_id: id },
                    _ => SceneCommand::NewConversation { scene_id: id },
                })
                .unwrap();
            assert_eq!(
                store
                    .finish_session_title(&request, "会话任务".into())
                    .is_ok(),
                mode == 0
            );
        }
    }
    #[test]
    fn unfinished_failed_second_turn_and_manually_named_do_not_request_title() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.snapshot().active_scene_id;
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "首次任务".into(),
            })
            .unwrap();
        let first = store.begin_run(&id).unwrap();
        assert!(store
            .claim_session_title(&id, &first.run_id)
            .unwrap()
            .is_none());
        store.fail_run(&id, &first.run_id, "网络错误").unwrap();
        assert!(store
            .claim_session_title(&id, &first.run_id)
            .unwrap()
            .is_none());
        store
            .apply(SceneCommand::SetDraft {
                scene_id: id.clone(),
                draft: "第二条".into(),
            })
            .unwrap();
        let second = store.begin_run(&id).unwrap();
        store.finish_run(&id, &second.run_id, "回答").unwrap();
        assert!(store
            .claim_session_title(&id, &second.run_id)
            .unwrap()
            .is_none());
        store
            .apply(SceneCommand::NewConversation {
                scene_id: id.clone(),
            })
            .unwrap();
        store
            .apply(SceneCommand::RenameScene {
                scene_id: id,
                title: "我的标题".into(),
            })
            .unwrap();
        let (id, run) = ready(&mut store);
        assert!(store.claim_session_title(&id, &run).unwrap().is_none());
    }
    #[test]
    fn legacy_defaults_are_read_only_and_claim_survives_restart() {
        let path = std::env::temp_dir().join(format!("mewu-title-{}.sqlite", Uuid::new_v4()));
        {
            let mut store = Store::open(&path).unwrap();
            let (id, run) = ready(&mut store);
            let raw = serde_json::to_value(store.snapshot()).unwrap();
            assert!(raw["scenes"][0].get("autoTitleAttempted").is_none());
            assert!(raw["scenes"][0].get("titleRevision").is_none());
            store.claim_session_title(&id, &run).unwrap().unwrap();
        }
        {
            let mut store = Store::open(&path).unwrap();
            let s = store.snapshot().scenes[0].clone();
            assert!(store
                .claim_session_title(&s.id, &s.run.unwrap().id)
                .unwrap()
                .is_none());
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn workflow_before_typed_message_does_not_take_the_user_title_slot() {
        let mut store = Store::open_in_memory().unwrap();
        let scene = store.snapshot().active_scene_id;
        store
            .set_background(
                &scene,
                Asset {
                    id: id(),
                    kind: AssetKind::Image,
                    name: "fixture.png".into(),
                    path: "fixture.png".into(),
                    width: Some(100),
                    height: Some(80),
                    origin_x: None,
                    origin_y: None,
                    scale_factor: None,
                },
            )
            .unwrap();
        let region = id();
        store
            .apply(SceneCommand::AddRegion {
                scene_id: scene.clone(),
                region: Region {
                    id: region.clone(),
                    x: 0.0,
                    y: 0.0,
                    width: 80.0,
                    height: 60.0,
                    ..Default::default()
                },
            })
            .unwrap();
        let workflow = store
            .begin_workflow(&scene, "自动工具提示", &region)
            .unwrap();
        store
            .finish_run(&scene, &workflow.run_id, "工具结果")
            .unwrap();
        assert!(store
            .claim_session_title(&scene, &workflow.run_id)
            .unwrap()
            .is_none());
        let (scene, run) = ready(&mut store);
        let request = store.claim_session_title(&scene, &run).unwrap().unwrap();
        assert_eq!(request.prompt, "帮我规划项目的发布流程");
        assert!(store
            .finish_session_title(&request, "发布流程规划".into())
            .is_ok());
    }
}
