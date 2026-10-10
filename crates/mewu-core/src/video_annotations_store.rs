// SPDX-License-Identifier: MPL-2.0
use super::journal_store::{live, local_result, respond_local};
use super::*;
use crate::journal as j;
use crate::video_annotation_algorithm as algorithm;
use crate::video_annotation_layouts as layouts;
use crate::video_annotations as video;
use crate::video_registered_layouts as registered;
use crate::video_vector_layouts as vector_layouts;

fn indexes(
    state: &Snapshot,
    target: &VideoAnnotationTarget,
    manual: bool,
) -> Result<(usize, usize)> {
    let s = state
        .scenes
        .iter()
        .position(|s| s.id == target.scene_id)
        .ok_or(CoreError::VideoAnnotationConflict)?;
    let scene = &state.scenes[s];
    if manual && (state.active_scene_id != scene.id || scene.frozen || scene.closed) {
        return Err(CoreError::VideoAnnotationConflict);
    }
    if manual && is_running(scene) {
        return Err(CoreError::RunInProgress);
    }
    let i = scene
        .items
        .iter()
        .position(|i| i.id == target.item_id)
        .ok_or(CoreError::VideoAnnotationConflict)?;
    let item = &scene.items[i];
    if item.asset.kind != AssetKind::Video
        || item.asset.id != target.source_id
        || item.video_edit.as_ref().map_or(0, |e| e.revision) != target.expected_range_revision
        || item.video_annotations.as_ref().map_or(0, |d| d.revision)
            != target.expected_annotation_revision
    {
        return Err(CoreError::VideoAnnotationConflict);
    }
    video::validate_item(item)?;
    Ok((s, i))
}
fn registered_layouts(
    db: &SqlConnection,
    before: Option<&VideoAnnotationDocument>,
    after: &VideoAnnotationDocument,
    new: &[VerifiedVideoTextLayout],
    fresh_only: bool,
) -> Result<()> {
    registered::registered_layouts(db, before, after, new, &[], fresh_only)
}
impl Store {
    /// Narrow in-memory view for manual document reads/edits. This grants no
    /// native source-file permission and never clones unrelated conversations.
    /// Read-only callers may inspect during a run; both modes require the
    /// active, open, unfrozen scene and the exact existing document target.
    pub fn video_annotation_source(
        &self,
        target: &VideoAnnotationTarget,
        editing: bool,
    ) -> Result<VideoSourceView> {
        let (s, i) = indexes(&self.state, target, editing)?;
        let scene = &self.state.scenes[s];
        let item = &scene.items[i];
        if self.state.active_scene_id != scene.id
            || scene.closed
            || scene.frozen
            || item.video_annotations.is_none()
        {
            return Err(CoreError::VideoAnnotationConflict);
        }
        Ok(VideoSourceView {
            asset: item.asset.clone(),
            edit: item.video_edit.clone(),
            annotations: item.video_annotations.clone(),
        })
    }

    /// Creation-only narrow view. Unlike an existing-document read, None is
    /// legal precisely when target.expected_annotation_revision is zero.
    /// Native still owes source-file proof, visibility and plugin authority.
    pub fn video_annotation_creation_source(
        &self,
        target: &VideoAnnotationTarget,
    ) -> Result<VideoSourceView> {
        let (s, i) = indexes(&self.state, target, true)?;
        let item = &self.state.scenes[s].items[i];
        Ok(VideoSourceView {
            asset: item.asset.clone(),
            edit: item.video_edit.clone(),
            annotations: item.video_annotations.clone(),
        })
    }
    pub fn video_vector_layout(
        &self,
        reference: &VideoVectorLayoutRef,
    ) -> Result<VerifiedVideoVectorLayout> {
        vector_layouts::load(&self.db, reference)
    }
    /// Literal immutable descriptor read; this grants no scene or source rights.
    pub fn read_video_vector_layout(
        path: impl AsRef<Path>,
        reference: &VideoVectorLayoutRef,
    ) -> Result<VerifiedVideoVectorLayout> {
        vector_layouts::read_layout(path.as_ref(), reference)
    }
    /// Native-authorized drawing only: Core chooses every object ID/origin and
    /// the original full playback interval, independently of the current trim.
    pub fn append_manual_video_drawings(
        &mut self,
        target: &VideoAnnotationTarget,
        proof: &VerifiedVideoAnnotationSource,
        primitives: Vec<VideoAnnotationPrimitive>,
        new: VideoAnnotationLayouts,
    ) -> Result<Snapshot> {
        if primitives.iter().any(|p| {
            !matches!(
                p,
                VideoAnnotationPrimitive::Text { .. } | VideoAnnotationPrimitive::Vector { .. }
            )
        }) {
            return Err(j::bad("视频手绘只能登记文字或完整矢量布局"));
        }
        let interval = VideoRange {
            start_ticks: 0,
            end_ticks: proof.source().duration_ticks,
        };
        let drafts = primitives
            .into_iter()
            .map(|primitive| VideoAnnotationDraft {
                interval,
                primitive,
            })
            .collect();
        self.append_registered_video_annotations(target, proof, drafts, new)
    }
    /// Run, real user message, immutable sent frames, grant and source manifest
    /// are registered in one SQLite transaction. Failed admission keeps draft.
    pub fn begin_run_with_video_input(
        &mut self,
        scene_id: &str,
        memory: Option<MemoryAuthorityGrant>,
        input: VerifiedVideoRunInput,
    ) -> Result<RunContext> {
        let scene = scene(&self.state, scene_id)?;
        if self.state.active_scene_id != scene_id || scene.frozen || scene.closed {
            return Err(CoreError::VideoAnnotationConflict);
        }
        validate_attachments(input.assets())?;
        self.begin_run_with_input(scene_id, None, memory.as_ref(), Some(input))
    }
    pub fn begin_video_workflow_with_input(
        &mut self,
        scene_id: &str,
        prompt: &str,
        item_id: &str,
        input: VerifiedVideoRunInput,
    ) -> Result<RunContext> {
        bounded_text(prompt, 16_000, "工作流提示")?;
        if prompt.contains('\0') {
            return Err(j::bad("工作流提示不能包含空字符"));
        }
        let scene = scene(&self.state, scene_id)?;
        if self.state.active_scene_id != scene_id || scene.frozen || scene.closed {
            return Err(CoreError::VideoAnnotationConflict);
        }
        validate_attachments(input.assets())?;
        self.begin_run_with_input(scene_id, Some((prompt, item_id)), None, Some(input))
    }
    pub fn video_text_layout(
        &self,
        reference: &VideoTextLayoutRef,
    ) -> Result<VerifiedVideoTextLayout> {
        layouts::load(&self.db, reference)
    }
    /// A read grants no source/scene/export permission. Host first derives the
    /// immutable ref from its fenced plan and checks the plan again afterwards.
    pub fn read_video_text_layout(
        path: impl AsRef<Path>,
        reference: &VideoTextLayoutRef,
    ) -> Result<VerifiedVideoTextLayout> {
        layouts::read_layout(path.as_ref(), reference)
    }
    pub fn append_video_annotations(
        &mut self,
        target: &VideoAnnotationTarget,
        proof: &VerifiedVideoAnnotationSource,
        drafts: Vec<VideoAnnotationDraft>,
        new: Vec<VerifiedVideoTextLayout>,
    ) -> Result<Snapshot> {
        if drafts
            .iter()
            .any(|d| matches!(&d.primitive, VideoAnnotationPrimitive::Vector { .. }))
        {
            return Err(j::bad("新增视频手绘须使用全时段登记入口"));
        }
        self.append_registered_video_annotations(
            target,
            proof,
            drafts,
            VideoAnnotationLayouts {
                text: new,
                vector: vec![],
            },
        )
    }
    fn append_registered_video_annotations(
        &mut self,
        target: &VideoAnnotationTarget,
        proof: &VerifiedVideoAnnotationSource,
        drafts: Vec<VideoAnnotationDraft>,
        new: VideoAnnotationLayouts,
    ) -> Result<Snapshot> {
        if drafts.is_empty() || drafts.len() > MAX_VIDEO_ANNOTATIONS {
            return Err(j::bad("视频批注批次无效"));
        }
        let (s, i) = indexes(&self.state, target, true)?;
        let item = &self.state.scenes[s].items[i];
        video_source_fence(
            &self.video_source(&target.scene_id, &target.item_id)?,
            proof,
        )?;
        let objects = drafts
            .into_iter()
            .map(|d| TimedVideoAnnotation {
                id: id(),
                interval: d.interval,
                primitive: d.primitive,
                origin: None,
            })
            .collect();
        let operation = id();
        let candidate = match &item.video_annotations {
            Some(doc) => algorithm::mutate_document(
                &item.asset,
                doc,
                VideoAnnotationMutation::Append(objects),
                &operation,
            )?
            .ok_or_else(|| j::bad("空视频批注操作"))?,
            None => algorithm::create_document(proof, objects, &operation)?,
        };
        self.journal_transaction(|db, state| {
            let (s, i) = indexes(state, target, true)?;
            let item = &state.scenes[s].items[i];
            video_source_fence(
                &VideoSourceView {
                    asset: item.asset.clone(),
                    edit: item.video_edit.clone(),
                    annotations: item.video_annotations.clone(),
                },
                proof,
            )?;
            registered::registered_layouts(
                db,
                item.video_annotations.as_ref(),
                &candidate,
                &new.text,
                &new.vector,
                false,
            )?;
            state.scenes[s].items[i].video_annotations = Some(candidate);
            touch(&mut state.scenes[s]);
            Ok(())
        })?;
        Ok(self.snapshot())
    }
    pub fn edit_video_annotation_with_layouts(
        &mut self,
        target: &VideoAnnotationTarget,
        mutation: VideoAnnotationMutation,
        new: Vec<VerifiedVideoTextLayout>,
    ) -> Result<Snapshot> {
        self.edit_video_annotation_with_registered_layouts(
            target,
            mutation,
            VideoAnnotationLayouts {
                text: new,
                vector: vec![],
            },
        )
    }
    pub fn edit_video_annotation_with_registered_layouts(
        &mut self,
        target: &VideoAnnotationTarget,
        mutation: VideoAnnotationMutation,
        new: VideoAnnotationLayouts,
    ) -> Result<Snapshot> {
        if matches!(mutation, VideoAnnotationMutation::Append(_)) {
            return Err(j::bad("新增视频批注需要已验证的原视频"));
        }
        let (s, i) = indexes(&self.state, target, true)?;
        let item = &self.state.scenes[s].items[i];
        let before = item
            .video_annotations
            .as_ref()
            .ok_or(CoreError::VideoAnnotationConflict)?;
        let Some(candidate) = algorithm::mutate_document(&item.asset, before, mutation, &id())?
        else {
            if !new.is_empty() {
                return Err(j::bad("不能登记未使用视频文字布局"));
            }
            self.ensure_current_revision()?;
            return Ok(self.snapshot());
        };
        self.journal_transaction(|db, state| {
            let (s, i) = indexes(state, target, true)?;
            registered::registered_layouts(
                db,
                state.scenes[s].items[i].video_annotations.as_ref(),
                &candidate,
                &new.text,
                &new.vector,
                false,
            )?;
            state.scenes[s].items[i].video_annotations = Some(candidate);
            touch(&mut state.scenes[s]);
            Ok(())
        })?;
        Ok(self.snapshot())
    }
    pub fn video_input_for_run(
        &self,
        scene_id: &str,
        run_id: &str,
        authority: &VerifiedVideoAnnotationAuthority,
    ) -> Result<VerifiedVideoInput> {
        let record = j::required(&self.db, run_id)?;
        if record.scene != scene_id {
            return Err(CoreError::JournalConflict);
        }
        live(&self.state, &record)?;
        let (hash, input) = video::load(&self.db, run_id)?.ok_or(CoreError::JournalConflict)?;
        if input.revoked || hash != authority.manifest_sha256 || input.grant != authority.grant {
            return Err(CoreError::JournalConflict);
        }
        let scene = scene(&self.state, scene_id)?;
        for (item_id, cursor) in &input.cursors {
            let item = scene
                .items
                .iter()
                .find(|i| i.id == *item_id)
                .ok_or(CoreError::VideoAnnotationConflict)?;
            if video::fence_for_item(item, cursor)? != *cursor {
                return Err(CoreError::VideoAnnotationConflict);
            }
        }
        VerifiedVideoInput::new(input.manifest)
    }
    /// Host cancels transport before this durable additional rejection fence.
    pub fn revoke_video_authority(&mut self, scene_id: &str, run_id: &str) -> Result<()> {
        self.journal_transaction(|db, state| {
            let record = j::required(db, run_id)?;
            if record.scene != scene_id {
                return Err(CoreError::JournalConflict);
            }
            super::journal_store::scope(state, &record)?;
            if let Some((_, mut input)) = video::load(db, run_id)? {
                if !input.revoked {
                    input.revoked = true;
                    db.execute(
                        "UPDATE video_run_inputs SET data=?1 WHERE run_id=?2",
                        params![serde_json::to_string(&input)?, run_id],
                    )?;
                }
            }
            Ok(())
        })
    }
    /// Exact input fence, plugin binding, whole object batch, immutable rasters,
    /// history, own-effect cursor and prepared->responded receipt commit once.
    pub fn apply_video_annotations_with_receipt(
        &mut self,
        lease: &DispatchLease,
        authority: &VerifiedVideoAnnotationAuthority,
        batch: ValidatedVideoAppendBatch,
        new: Vec<VerifiedVideoTextLayout>,
    ) -> Result<LocalToolCommit> {
        let count = batch.groups.iter().try_fold(0usize, |n, g| {
            n.checked_add(g.objects.len())
                .ok_or_else(|| j::bad("视频批注对象过多"))
        })?;
        if batch.groups.len() != 1
            || count == 0
            || count > MAX_VIDEO_ANNOTATIONS
            || batch.groups.iter().any(|g| g.objects.is_empty())
        {
            return Err(j::bad("视频批注批次无效"));
        }
        let (result, receipt) = self.journal_transaction(|db, state| {
            let (record, event, _) = j::leased(db, lease)?;
            live(state, &record)?;
            let (hash, mut input) =
                video::load(db, &record.id)?.ok_or(CoreError::JournalConflict)?;
            if event.summary.phase != ReceiptPhase::Prepared
                || input.revoked
                || hash != authority.manifest_sha256
                || input.grant != authority.grant
                || event.binding != Some(input.grant.video_binding(&hash))
            {
                return Err(CoreError::JournalConflict);
            }
            video::validate_usage(db, &record.id, &input, &hash)?;
            if input.created_count as usize + count > MAX_VIDEO_ANNOTATIONS {
                return Err(j::bad("本次视频批注对象预算超限"));
            }
            let group_id = id();
            let target_scene = scene_mut(state, &record.scene)?;
            let mut committed = Vec::new();
            let mut used_layouts = HashSet::new();
            for group in batch.groups {
                let entry = input
                    .manifest
                    .targets
                    .iter()
                    .find(|t| t.handle == group.target_handle)
                    .ok_or(CoreError::VideoAnnotationConflict)?;
                let item = target_scene
                    .items
                    .iter_mut()
                    .find(|i| i.id == entry.item_id)
                    .ok_or(CoreError::VideoAnnotationConflict)?;
                let cursor = input
                    .cursors
                    .get(&entry.item_id)
                    .ok_or(CoreError::VideoAnnotationConflict)?;
                if video::fence_for_item(item, cursor)? != *cursor {
                    return Err(CoreError::VideoAnnotationConflict);
                }
                let mut objects = Vec::new();
                let mut identities = Vec::new();
                for draft in group.objects {
                    if matches!(&draft.primitive, VideoAnnotationPrimitive::Vector { .. }) {
                        return Err(j::bad("模型批次不允许视频手绘矢量"));
                    }
                    video::validate_sent_interval(entry, draft.interval)?;
                    algorithm::validate_primitive(
                        &draft.primitive,
                        entry.source.source_width,
                        entry.source.source_height,
                    )?;
                    if let VideoAnnotationPrimitive::Text { layout, .. } = &draft.primitive {
                        if !used_layouts.insert(layout.layout_id.clone())
                            || !new.iter().any(|v| v.reference() == layout)
                        {
                            return Err(j::bad("视频文字需本次独立验证布局"));
                        }
                    }
                    let object_id = id();
                    let created_sha256 = video::creation_hash(draft.interval, &draft.primitive)?;
                    identities.push(video::VideoCreatedObject {
                        id: object_id.clone(),
                        target_handle: entry.handle.clone(),
                        created_sha256: created_sha256.clone(),
                        interval: draft.interval,
                        primitive: draft.primitive.clone(),
                    });
                    objects.push(TimedVideoAnnotation {
                        id: object_id,
                        interval: draft.interval,
                        primitive: draft.primitive,
                        origin: Some(VideoAnnotationOrigin {
                            run_id: record.id.clone(),
                            user_message_id: record.user.clone(),
                            tool_event_id: event.summary.id.clone(),
                            group_id: group_id.clone(),
                            target_handle: entry.handle.clone(),
                            manifest_sha256: hash.clone(),
                            created_sha256,
                        }),
                    });
                }
                let proof = VerifiedVideoAnnotationSource::new(
                    VerifiedVideoSource {
                        asset: item.asset.clone(),
                        duration_ticks: entry.source.source_duration_ticks,
                        width: entry.source.source_width,
                        height: entry.source.source_height,
                    },
                    entry.source.source_sha256.clone(),
                )?;
                let doc = match &item.video_annotations {
                    Some(doc) => algorithm::mutate_document(
                        &item.asset,
                        doc,
                        VideoAnnotationMutation::Append(objects),
                        &group_id,
                    )?
                    .ok_or_else(|| j::bad("视频批注空操作"))?,
                    None => algorithm::create_document(&proof, objects, &group_id)?,
                };
                registered_layouts(db, item.video_annotations.as_ref(), &doc, &new, false)?;
                let annotation_revision = doc.revision;
                item.video_annotations = Some(doc);
                let final_source = video::fence_for_item(item, &entry.source)?;
                committed.push(video::VideoCommittedTarget {
                    item_id: entry.item_id.clone(),
                    annotation_revision,
                    source: final_source.clone(),
                    objects: identities,
                });
                input.cursors.insert(entry.item_id.clone(), final_source);
            }
            if used_layouts.len() != new.len() {
                return Err(j::bad("不能登记未使用视频文字布局"));
            }
            for v in &new {
                input.text_content_bytes = input
                    .text_content_bytes
                    .checked_add(v.content_bytes()?)
                    .ok_or_else(|| j::bad("视频文字内容预算超限"))?;
                input.text_png_bytes = input
                    .text_png_bytes
                    .checked_add(v.png().len() as u64)
                    .ok_or_else(|| j::bad("视频文字PNG预算超限"))?;
                input.text_pixels = input
                    .text_pixels
                    .checked_add(v.pixels())
                    .ok_or_else(|| j::bad("视频文字像素预算超限"))?;
            }
            if input.text_content_bytes > 64 * 1024
                || input.text_png_bytes > 32 * 1024 * 1024
                || input.text_pixels > 32 * 1024 * 1024
            {
                return Err(j::bad("本次视频文字预算超限"));
            }
            input.created_count += count as u32;
            db.execute(
                "UPDATE video_run_inputs SET data=?1 WHERE run_id=?2",
                params![serde_json::to_string(&input)?, record.id],
            )?;
            touch(target_scene);
            let result = local_result(serde_json::to_value(video::VideoCommitReceipt {
                applied: true,
                group_id,
                manifest_sha256: hash,
                objects: count as u32,
                targets: committed,
            })?)?;
            let receipt = respond_local(db, state, lease, result.clone())?;
            Ok((result, receipt))
        })?;
        Ok(LocalToolCommit {
            snapshot: self.snapshot(),
            result,
            receipt,
        })
    }
}
