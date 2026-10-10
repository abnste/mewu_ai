// SPDX-License-Identifier: MPL-2.0
use super::*;

fn editable(state: &Snapshot, id: &str) -> Result<()> {
    let target = scene(state, id)?;
    if state.active_scene_id != id || target.closed || target.frozen {
        return Err(invalid("会话已变化"));
    }
    if is_running(target) {
        return Err(CoreError::RunInProgress);
    }
    Ok(())
}

pub(crate) fn board_region(target: &Scene) -> Result<&Region> {
    let background = target
        .background
        .as_ref()
        .ok_or_else(|| invalid("黑板不存在"))?;
    let path = background.path.replace('\\', "/");
    if background.name != "黑板.png" || !path.ends_with(&format!("/{}.board.png", background.id))
    {
        return Err(invalid("黑板来源无效"));
    }
    let region = target
        .regions
        .first()
        .ok_or_else(|| invalid("黑板区域不存在"))?;
    if target.regions.len() != 1
        || region.x != 0.
        || region.y != 0.
        || background.width.map(f64::from) != Some(region.width)
        || background.height.map(f64::from) != Some(region.height)
    {
        return Err(invalid("黑板区域已变化"));
    }
    Ok(region)
}

fn board_item(id: String, board_id: &str, asset: Asset, offset: usize) -> SpaceItem {
    SpaceItem {
        id,
        asset,
        x: 0.10 + (offset % 4) as f64 * 0.035,
        y: 0.12 + (offset % 4) as f64 * 0.035,
        width: 0.48,
        height: 0.48,
        state: Some(std::collections::BTreeMap::from([(
            "blackboardSceneId".into(),
            serde_json::Value::String(board_id.into()),
        )])),
        video_edit: None,
        video_annotations: None,
    }
}

impl Store {
    /// The parent remains intact. Its image card and the editable backing scene
    /// are created together, so interrupted editing can always be reopened.
    pub fn create_blackboard_document(
        &mut self,
        parent_id: &str,
        asset: Asset,
    ) -> Result<Snapshot> {
        validate_background(&asset)?;
        editable(&self.state, parent_id)?;
        if scene(&self.state, parent_id)?.blackboard_link.is_some() {
            return Err(invalid("请先完成当前黑板"));
        }
        let mut next = self.state.clone();
        let parent = scene(&next, parent_id)?;
        let mut board = new_scene(&parent.agent_id, parent.connection_id.clone());
        let item_id = Uuid::new_v4().to_string();
        board.title = "黑板".into();
        board.blackboard_link = Some(BlackboardLink {
            parent_scene_id: parent_id.into(),
            item_id: item_id.clone(),
        });
        board.regions = vec![Region {
            id: Uuid::new_v4().to_string(),
            x: 0.,
            y: 0.,
            width: asset.width.unwrap() as f64,
            height: asset.height.unwrap() as f64,
            ..Region::default()
        }];
        board.refs = vec![Reference {
            kind: ReferenceKind::Region,
            id: board.regions[0].id.clone(),
        }];
        board.background = Some(asset.clone());
        board_region(&board)?;
        let parent = scene_mut(&mut next, parent_id)?;
        parent
            .items
            .push(board_item(item_id, &board.id, asset, parent.items.len()));
        touch(parent);
        for target in &mut next.scenes {
            target.frozen = true;
        }
        next.active_scene_id = board.id.clone();
        next.scenes.push(board);
        self.persist(next)
    }

    pub fn open_blackboard_document(&mut self, parent_id: &str, item_id: &str) -> Result<Snapshot> {
        editable(&self.state, parent_id)?;
        let parent = scene(&self.state, parent_id)?;
        if !parent.items.iter().any(|item| item.id == item_id) {
            return Err(invalid("黑板图片已移除"));
        }
        // Item.state is a UI hint, never authority for opening another scene.
        let board = self
            .state
            .scenes
            .iter()
            .find(|target| {
                target.blackboard_link.as_ref().is_some_and(|link| {
                    link.parent_scene_id == parent_id && link.item_id == item_id
                })
            })
            .ok_or_else(|| invalid("黑板不存在"))?;
        board_region(board)?;
        if board.closed || is_running(board) {
            return Err(invalid("黑板不可编辑"));
        }
        let board_id = board.id.clone();
        let mut next = self.state.clone();
        for target in &mut next.scenes {
            target.frozen = target.id != board_id;
        }
        next.active_scene_id = board_id;
        self.persist(next)
    }

    /// Preview pixels are registered only by the host after its exact source
    /// fence. Strokes/history stay in the backing scene; old previews remain
    /// immutable for historical AI attachments.
    pub fn finish_blackboard_document(
        &mut self,
        expected: &Scene,
        asset: Asset,
    ) -> Result<Snapshot> {
        editable(&self.state, &expected.id)?;
        if scene(&self.state, &expected.id)? != expected {
            return Err(CoreError::DrawingConflict);
        }
        let region = board_region(expected)?;
        validate_background(&asset)?;
        if asset.width != Some(region.width as u32) || asset.height != Some(region.height as u32) {
            return Err(invalid("黑板预览尺寸不符"));
        }
        let mut next = self.state.clone();
        let link = if let Some(link) = &expected.blackboard_link {
            link.clone()
        } else {
            // Explicitly finishing an old board adopts it without a startup
            // migration. Keep its chat in the original scene, its ink in a new one.
            let mut board = new_scene(&expected.agent_id, expected.connection_id.clone());
            let link = BlackboardLink {
                parent_scene_id: expected.id.clone(),
                item_id: Uuid::new_v4().to_string(),
            };
            board.title = "黑板".into();
            board.background = expected.background.clone();
            board.regions = expected.regions.clone();
            board.blackboard_link = Some(link.clone());
            board.frozen = true;
            let parent = scene_mut(&mut next, &expected.id)?;
            parent.background = None;
            parent.regions.clear();
            parent.geometry_history = Default::default();
            parent
                .refs
                .retain(|reference| reference.kind != ReferenceKind::Region);
            parent.items.push(board_item(
                link.item_id.clone(),
                &board.id,
                asset.clone(),
                parent.items.len(),
            ));
            next.scenes.push(board);
            link
        };
        let parent = scene_mut(&mut next, &link.parent_scene_id)?;
        if parent.closed || is_running(parent) {
            return Err(invalid("原会话不可恢复"));
        }
        let item = parent
            .items
            .iter_mut()
            .find(|item| item.id == link.item_id)
            .ok_or_else(|| invalid("黑板图片已移除"))?;
        item.asset = asset;
        let reference = Reference {
            kind: ReferenceKind::Item,
            id: item.id.clone(),
        };
        if !parent.refs.contains(&reference) {
            parent.refs.push(reference);
        }
        touch(parent);
        next.active_scene_id = link.parent_scene_id;
        for target in &mut next.scenes {
            target.frozen = target.id != next.active_scene_id;
        }
        self.persist(next)
    }
}
