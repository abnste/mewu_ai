// SPDX-License-Identifier: MPL-2.0
use super::journal_store::{live, local_result, respond_local};
use super::*;
use crate::journal as j;
use crate::visual_annotations as visual;
use std::collections::BTreeMap;

impl Store {
    /// Persist the exact sent attachments and their source authority together.
    /// The grant is supplied only by the host after checking the installed plugin.
    pub fn attach_run_visual_assets(
        &mut self,
        scene_id: &str,
        run_id: &str,
        assets: Vec<Asset>,
        grant: VisualAnnotationGrant,
        input: VerifiedVisualInput,
    ) -> Result<Snapshot> {
        validate_attachments(&assets)?;
        visual::validate_grant(&grant)?;
        visual::validate_asset_binding(input.manifest(), &assets)?;
        self.journal_transaction(|db, state| {
            let record = j::required(db, run_id)?;
            if record.scene != scene_id || record.kind == RecordedRunKind::Continuation {
                return Err(CoreError::JournalConflict);
            }
            live(state, &record)?;
            if let Some((hash, existing)) = visual::load(db, run_id)? {
                let message = scene(state, scene_id)?
                    .messages
                    .last()
                    .ok_or(CoreError::JournalConflict)?;
                if hash == input.sha256()
                    && existing.manifest == *input.manifest()
                    && existing.grant == grant
                    && !existing.revoked
                    && message.attachments.as_ref() == Some(&assets)
                {
                    return Ok(());
                }
                return Err(CoreError::JournalConflict);
            }
            if !j::events(db, run_id)?.is_empty() {
                return Err(CoreError::JournalConflict);
            }
            let target = scene_mut(state, scene_id)?;
            let background = target
                .background
                .as_ref()
                .ok_or(CoreError::DrawingConflict)?;
            let mut cursors = BTreeMap::new();
            for entry in &input.manifest().targets {
                let region = target
                    .regions
                    .iter()
                    .find(|r| r.id == entry.region_id)
                    .ok_or(CoreError::DrawingConflict)?;
                if visual_source_fence(background, region)? != entry.source {
                    return Err(CoreError::DrawingConflict);
                }
                cursors.insert(entry.region_id.clone(), entry.source.clone());
            }
            let message = target
                .messages
                .last_mut()
                .ok_or(CoreError::JournalConflict)?;
            if message.id != record.user
                || message.role != MessageRole::User
                || message.run_id.as_deref() != Some(run_id)
                || message.attachments.as_ref().is_some_and(|a| *a != assets)
            {
                return Err(CoreError::JournalConflict);
            }
            message.attachments = Some(assets);
            let stored = visual::StoredVisualInput {
                manifest: input.manifest().clone(),
                grant,
                cursors,
                created_count: 0,
                rich_content_bytes: 0,
                rich_png_bytes: 0,
                rich_pixels: 0,
                revoked: false,
            };
            db.execute(
                "INSERT INTO visual_run_inputs(run_id,manifest_sha256,data) VALUES(?1,?2,?3)",
                params![run_id, input.sha256(), serde_json::to_string(&stored)?],
            )?;
            touch(target);
            Ok(())
        })?;
        Ok(self.snapshot())
    }

    pub fn visual_input_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        authority: &VerifiedVisualAuthority,
    ) -> Result<VerifiedVisualInput> {
        let record = j::required(&self.db, run_id)?;
        if record.scene != scene_id {
            return Err(CoreError::JournalConflict);
        }
        live(&self.state, &record)?;
        let (hash, input) = visual::load(&self.db, run_id)?.ok_or(CoreError::JournalConflict)?;
        if input.revoked || hash != authority.manifest_sha256 || input.grant != authority.grant {
            return Err(CoreError::JournalConflict);
        }
        let target = scene(&self.state, scene_id)?;
        let background = target
            .background
            .as_ref()
            .ok_or(CoreError::DrawingConflict)?;
        for (region_id, cursor) in &input.cursors {
            let region = target
                .regions
                .iter()
                .find(|r| r.id == *region_id)
                .ok_or(CoreError::DrawingConflict)?;
            if visual_source_fence(background, region)? != *cursor {
                return Err(CoreError::DrawingConflict);
            }
        }
        VerifiedVisualInput::new(input.manifest)
    }

    /// Host revokes its sender first. This durable fence is additional and must
    /// not be the only cancellation mechanism if the database write fails.
    pub fn revoke_visual_authority(&mut self, scene_id: &str, run_id: &str) -> Result<()> {
        self.journal_transaction(|db, state| {
            let record = j::required(db, run_id)?;
            if record.scene != scene_id {
                return Err(CoreError::JournalConflict);
            }
            super::journal_store::scope(state, &record)?;
            if let Some((_, mut input)) = visual::load(db, run_id)? {
                if !input.revoked {
                    input.revoked = true;
                    db.execute(
                        "UPDATE visual_run_inputs SET data=?1 WHERE run_id=?2",
                        params![serde_json::to_string(&input)?, run_id],
                    )?;
                }
            }
            Ok(())
        })
    }

    /// All objects, history, own-effect cursors and the tool receipt commit as
    /// one revision-fenced transaction. No callback, I/O or nested Store write.
    pub fn apply_visual_annotations_with_receipt(
        &mut self,
        lease: &DispatchLease,
        authority: &VerifiedVisualAuthority,
        batch: VisualAppendBatch,
    ) -> Result<LocalToolCommit> {
        self.apply_visual_annotations_with_rich_receipt(lease, authority, batch, vec![])
    }

    pub fn apply_visual_annotations_with_rich_receipt(
        &mut self,
        lease: &DispatchLease,
        authority: &VerifiedVisualAuthority,
        batch: VisualAppendBatch,
        layouts: Vec<VerifiedRichLayout>,
    ) -> Result<LocalToolCommit> {
        if batch.groups.is_empty() || batch.groups.len() > 6 {
            return Err(j::bad("批注批次无效"));
        }
        let count = batch.groups.iter().try_fold(0usize, |total, group| {
            total
                .checked_add(group.drawings.len())
                .ok_or_else(|| j::bad("批注图元过多"))
        })?;
        if count == 0
            || count > MAX_VISUAL_OBJECTS
            || batch.groups.iter().any(|g| g.drawings.is_empty())
        {
            return Err(j::bad("一次最多添加 48 个批注图元"));
        }
        let (result,receipt) = self.journal_transaction(|db,state| {
            let (record,event,_) = j::leased(db,lease)?;
            live(state,&record)?;
            let (hash,mut input) = visual::load(db,&record.id)?.ok_or(CoreError::JournalConflict)?;
            if event.summary.phase != ReceiptPhase::Prepared || input.revoked || hash != authority.manifest_sha256 || input.grant != authority.grant || event.binding != Some(input.grant.binding(&hash)) { return Err(CoreError::JournalConflict); }
            visual::validate_rich_usage(db,&record.id,&input,&hash)?;
            if input.created_count as usize + count > MAX_VISUAL_OBJECTS { return Err(j::bad("本次运行最多添加 48 个批注图元")); }
            let mut registered = BTreeMap::new();
            for layout in &layouts {
                if layout.reference().kind.is_raster_edit() { return Err(j::bad("AI 批注不能指定本地修补图层")); }
                if registered.insert(layout.reference().layout_id.clone(),layout.reference().clone()).is_some() {
                    return Err(j::bad("富标注布局标识重复"));
                }
                input.rich_content_bytes = input.rich_content_bytes.checked_add(layout.content_bytes()?).ok_or_else(||j::bad("富标注内容预算超限"))?;
                input.rich_png_bytes = input.rich_png_bytes.checked_add(layout.png().len() as u64).ok_or_else(||j::bad("富标注PNG预算超限"))?;
                input.rich_pixels = input.rich_pixels.checked_add(layout.pixels()).ok_or_else(||j::bad("富标注像素预算超限"))?;
            }
            if input.rich_content_bytes > MAX_RICH_RUN_CONTENT_BYTES || input.rich_png_bytes > MAX_RICH_RUN_PNG_BYTES || input.rich_pixels > MAX_RICH_PIXELS {
                return Err(j::bad("本次运行富标注预算超限"));
            }
            let mut referenced = HashSet::new();
            for drawing in batch.groups.iter().flat_map(|g|&g.drawings) {
                if drawing.kind == DrawingKind::Rich {
                    let reference=drawing.rich.as_ref().ok_or_else(||j::bad("富标注缺少注册布局"))?;
                    if registered.get(&reference.layout_id) != Some(reference) { return Err(j::bad("富标注布局未由宿主登记或引用不匹配")); }
                    referenced.insert(reference.layout_id.clone());
                }
            }
            if referenced.len() != registered.len() { return Err(j::bad("不能登记未使用的富标注布局")); }
            for layout in &layouts { crate::rich_annotations::insert(db,layout)?; }
            let target = scene_mut(state,&record.scene)?;
            let background = target.background.as_ref().ok_or(CoreError::DrawingConflict)?;
            let mut groups: BTreeMap<String,Vec<Drawing>> = BTreeMap::new();
            let mut seen_handles = HashSet::new(); let mut seen_ids = HashSet::new();
            let group_id = id();
            for group in batch.groups {
                if !seen_handles.insert(group.target_handle.clone()) { return Err(j::bad("同一批注目标重复")); }
                let entry = input.manifest.targets.iter().find(|t|t.handle == group.target_handle).ok_or(CoreError::DrawingConflict)?;
                let region = target.regions.iter().find(|r|r.id == entry.region_id).ok_or(CoreError::DrawingConflict)?;
                if input.cursors.get(&entry.region_id) != Some(&visual_source_fence(background,region)?) { return Err(CoreError::DrawingConflict); }
                let first = entry.source_point(DrawingPoint{x:0.,y:0.})?;
                let last = entry.source_point(DrawingPoint{x:entry.encoded.width as f64,y:entry.encoded.height as f64})?;
                for mut drawing in group.drawings {
                    if drawing.origin.is_some() || drawing.kind == DrawingKind::Mosaic || !seen_ids.insert(drawing.id.clone()) || drawing.points.len() > 128 || drawing.text.as_ref().is_some_and(|t|t.chars().count() > 500)
                        || drawing.points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x < first.x || p.y < first.y || p.x > last.x || p.y > last.y) { return Err(j::bad("批注图元超出已发送的附件内容")); }
                    drawing.origin = Some(VisualDrawingOrigin { run_id:record.id.clone(),user_message_id:record.user.clone(),tool_event_id:event.summary.id.clone(),group_id:group_id.clone(),target_handle:entry.handle.clone(),manifest_sha256:hash.clone() });
                    groups.entry(entry.region_id.clone()).or_default().push(drawing);
                }
            }
            let mut applied = Vec::new();
            for (region_id,drawings) in groups {
                let region = target.regions.iter_mut().find(|r|r.id == region_id).ok_or(CoreError::DrawingConflict)?;
                let drawing_ids: Vec<_> = drawings.iter().map(|d|d.id.clone()).collect();
                let identities: Vec<_> = drawings.iter().map(|d|{
                    let mut value=serde_json::json!({"id":d.id,"targetHandle":d.origin.as_ref().expect("core origin").target_handle});
                    if let Some(reference)=&d.rich { value["rich"]=serde_json::to_value(reference).expect("rich reference serializes"); }
                    value
                }).collect();
                crate::drawing::apply(region,background,crate::drawing::Mutation::AppendBatch(drawings))?;
                region.ocr=None;
                if let Some(translation)=&mut region.translation { translation.drawing_revision=region.drawing_revision; }
                input.cursors.insert(region_id.clone(),visual_source_fence(background,region)?);
                applied.push(serde_json::json!({"regionId":region_id,"drawingIds":drawing_ids,"drawings":identities,"drawingRevision":region.drawing_revision}));
            }
            input.created_count += count as u32;
            db.execute("UPDATE visual_run_inputs SET data=?1 WHERE run_id=?2",params![serde_json::to_string(&input)?,record.id])?;
            touch(target);
            let result=local_result(serde_json::json!({"applied":true,"groupId":group_id,"manifestSha256":hash,"objects":count,"regions":applied}))?;
            let receipt=respond_local(db,state,lease,result.clone())?;
            Ok((result,receipt))
        })?;
        Ok(LocalToolCommit {
            snapshot: self.snapshot(),
            result,
            receipt,
        })
    }
}
