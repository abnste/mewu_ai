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
    pub fn update_blackboard_text(&mut self, scene_id: &str, item_id: &str, expected_asset_id: &str, replacement: Asset) -> Result<Snapshot> {
        self.journal_transaction(|_, state| {
            editable(state, scene_id)?;
            let target = scene_mut(state, scene_id)?;
            if target.blackboard_link.is_none() { return Err(invalid("此会话不是黑板")); }
            let item = target.items.iter_mut().find(|i| i.id == item_id).ok_or_else(|| not_found("文本对象", item_id))?;
            if item.asset.id != expected_asset_id || !crate::blackboard_objects::is_text(&item.asset)
                || !crate::blackboard_objects::is_text(&replacement) || replacement.id == expected_asset_id || replacement.name != item.asset.name {
                return Err(invalid("文本对象已变化，请重新读取"));
            }
            validate_asset(&replacement)?;
            item.asset = replacement;
            touch(target);
            Ok(())
        })?;
        Ok(self.snapshot())
    }
    /// The parent remains intact. Its image card and the editable backing scene
    /// are created together, so interrupted editing can always be reopened.
    pub fn create_blackboard_document(
        &mut self,
        parent_id: &str,
        asset: Asset,
    ) -> Result<Snapshot> {
        self.create_blackboard_document_with_captures(parent_id, asset, vec![], vec![])
    }

    /// Capture rasters and the editable board enter the database in one transaction.
    /// Only the native host can prepare VerifiedRichLayout values; PNGs never enter IPC JSON.
    pub fn create_blackboard_document_with_captures(
        &mut self,
        parent_id: &str,
        asset: Asset,
        captures: Vec<Drawing>,
        layouts: Vec<VerifiedRichLayout>,
    ) -> Result<Snapshot> {
        validate_background(&asset)?;
        editable(&self.state, parent_id)?;
        let source = scene(&self.state, parent_id)?;
        if source.blackboard_link.is_some() {
            return Err(invalid("请先完成当前黑板"));
        }
        if captures.len() != layouts.len()
            || (!captures.is_empty() && captures.len() != source.regions.len())
        {
            return Err(invalid("黑板截图对象不完整"));
        }
        let mut total_bytes = 0usize;
        for ((drawing, layout), region) in captures.iter().zip(&layouts).zip(&source.regions) {
            let background = source
                .background
                .as_ref()
                .ok_or_else(|| invalid("截图不存在"))?;
            let fence = visual_source_fence(background, region)?;
            if drawing.origin.is_some()
                || drawing.kind != DrawingKind::Rich
                || drawing.rich.as_ref() != Some(layout.reference())
                || !matches!(layout.content(), RichContent::Raster { version: 1, role: RasterRole::Extracted, source_sha256 } if *source_sha256 == fence.visual_sha256)
            {
                return Err(invalid("黑板截图对象来源无效"));
            }
            total_bytes = total_bytes
                .checked_add(layout.png().len())
                .ok_or_else(|| invalid("截图对象过大"))?;
            if total_bytes > 32 * 1024 * 1024 {
                return Err(invalid("截图对象过大"));
            }
        }
        let mut next = self.state.clone();
        let parent = scene(&next, parent_id)?;
        let mut board = new_scene(&parent.agent_id, parent.connection_id.clone());
        let item_id = Uuid::new_v4().to_string();
        board.title = "黑板".into();
        board.items = crate::blackboard_objects::carried(parent, &asset)?;
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
            drawing_revision: u64::from(!captures.is_empty()),
            drawings: captures,
            ..Region::default()
        }];
        board.refs = vec![Reference {
            kind: ReferenceKind::Region,
            id: board.regions[0].id.clone(),
        }];
        for carried in &board.items {
            let source = crate::blackboard_objects::source(carried)?.ok_or_else(|| invalid("黑板素材来源缺失"))?;
            if parent.refs.iter().any(|r| r.kind == ReferenceKind::Item && r.id == source.item_id) {
                board.refs.push(Reference {kind:ReferenceKind::Item,id:carried.id.clone()});
            }
        }
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
        self.journal_transaction(|db, state| {
            for layout in &layouts {
                crate::rich_annotations::insert(db, layout)?;
            }
            *state = next;
            Ok(())
        })?;
        Ok(self.snapshot())
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
