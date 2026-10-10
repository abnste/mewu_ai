// SPDX-License-Identifier: MPL-2.0
//! Real leased video bytes and sparse native frames prepared before begin_run.
//! Dropping the caller cancels interest, never the actual worker's I/O lifetime.
use crate::{ai, video_clip_contract as media, video_host, Host};
use mewu_core::{
    Asset, AssetKind, ReferenceKind, RunStatus, Scene, VerifiedVideoAnnotationSource,
    VerifiedVideoInput, VerifiedVideoSource, VideoExportOrigin, VideoInputFrame,
    VideoInputManifest, VideoInputTarget, VideoRange, VideoSourceFence, VideoSourceView,
    VisualAnnotationGrant, VIDEO_PROFILE_ID,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

pub struct PreparedVideoRun {
    pub input: VerifiedVideoInput,
    pub assets: Vec<Asset>,
    app: AppHandle,
    handle: usize,
    resources: Resources,
}
impl PreparedVideoRun {
    /// Safe inside the final plugins -> Engine section: no locks and no I/O.
    pub fn ensure_current(&self) -> Result<(), String> {
        video_host::check_cancel(&self.resources.invoke.0)?;
        video_host::atomic_gate(&self.app.state::<Host>(), self.handle, false)?;
        if self.resources.source.is_none() {
            return Err("视频来源租约已结束".into());
        }
        Ok(())
    }
    /// Only after the atomic run/input/attachments transaction has succeeded.
    pub fn committed(&mut self) {
        self.resources.created.paths.clear();
    }
}

#[derive(Default)]
struct CreatedAttachments {
    paths: Vec<PathBuf>,
}
impl Drop for CreatedAttachments {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

struct Staging {
    path: PathBuf,
    count: usize,
    directory: Option<File>,
}
impl Staging {
    fn create(root: &Path, count: usize) -> Result<Self, String> {
        if !root.is_absolute() || !(2..=6).contains(&count) {
            return Err("视频临时目录无效".into());
        }
        let parent = root.join("video-input-staging");
        fs::create_dir_all(&parent).map_err(|_| "无法准备视频帧")?;
        let metadata = fs::symlink_metadata(&parent).map_err(|_| "无法准备视频帧")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("视频临时目录无效".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("视频临时目录无效".into());
            }
        }
        let parent = fs::canonicalize(parent).map_err(|_| "无法准备视频帧")?;
        let path = parent.join(uuid::Uuid::new_v4().to_string());
        // Never reuse, enumerate or recursively remove another request's folder.
        fs::create_dir(&path).map_err(|_| "无法准备视频帧")?;
        let mut stage = Self {
            path,
            count,
            directory: None,
        };
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // The supervised child may create its exact ordinal files; rename
            // and deletion of this owned directory remain denied until it exits.
            options.share_mode(3).custom_flags(0x02000000);
        }
        stage.directory = Some(options.open(&stage.path).map_err(|_| "无法准备视频帧")?);
        Ok(stage)
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        self.directory.take();
        for index in 0..self.count {
            let _ = fs::remove_file(self.path.join(format!("{index}.jpg")));
        }
        // An unexpected file makes remove_dir fail rather than erasing it.
        let _ = fs::remove_dir(&self.path);
    }
}

// Rust drops fields in declaration order. Real Work is intentionally last:
// cleanup and leased handles finish before exit can observe a released slot.
struct Resources {
    created: CreatedAttachments,
    staging: Option<Staging>,
    source: Option<video_host::SourceLease>,
    invoke: video_host::InvokeCancel,
    _work: video_host::Work,
}
struct WaitCancel {
    token: Arc<AtomicBool>,
    armed: bool,
}
impl Drop for WaitCancel {
    fn drop(&mut self) {
        if self.armed {
            self.token.store(true, Ordering::Release);
        }
    }
}

fn selected_video(scene: &Scene) -> Result<VideoExportOrigin, String> {
    let mut selected = None;
    for reference in &scene.refs {
        if reference.kind != ReferenceKind::Item {
            continue;
        }
        let item = scene
            .items
            .iter()
            .find(|item| item.id == reference.id)
            .ok_or("视频引用已更改")?;
        if item.asset.kind != AssetKind::Video {
            continue;
        }
        if selected.is_some() {
            return Err("每次视频原位作答请选择一个视频".into());
        }
        selected = Some(VideoExportOrigin {
            scene_id: scene.id.clone(),
            item_id: item.id.clone(),
            source: VideoSourceView {
                asset: item.asset.clone(),
                edit: item.video_edit.clone(),
                annotations: item.video_annotations.clone(),
            },
        });
    }
    selected.ok_or_else(|| "请先引用一个视频".into())
}
fn check_scene(
    snapshot: &mewu_core::Snapshot,
    scene: &Scene,
    origin: &VideoExportOrigin,
) -> Result<(), String> {
    let current = snapshot
        .scenes
        .iter()
        .find(|value| value.id == scene.id)
        .ok_or("视频所在会话已更改")?;
    if current != scene
        || current.closed
        || current.frozen
        || current
            .run
            .as_ref()
            .is_some_and(|run| run.status == RunStatus::Running)
        || !video_host::matches(snapshot, origin)
    {
        return Err("视频会话或引用已更改，请重试".into());
    }
    Ok(())
}
fn space_handle(app: &AppHandle) -> Result<usize, String> {
    let window = app.get_webview_window("space").ok_or("空间窗口不可用")?;
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|handle| handle.0 as usize)
            .map_err(|_| "空间窗口不可用".into())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err("当前系统不支持视频原位作答".into())
    }
}
fn gate(
    app: &AppHandle,
    host: &Host,
    handle: usize,
    token: Option<&AtomicBool>,
) -> Result<(), String> {
    if let Some(token) = token {
        video_host::check_cancel(token)?;
    }
    video_host::atomic_gate(host, handle, false)?;
    crate::recording::ensure_idle(app)?;
    if host.capturing.load(Ordering::Acquire) {
        return Err("请先完成当前采集或空间切换".into());
    }
    Ok(())
}
fn check_current(
    app: &AppHandle,
    scene: &Scene,
    origin: &VideoExportOrigin,
    grant: &VisualAnnotationGrant,
    token: &AtomicBool,
    handle: usize,
) -> Result<(), String> {
    let host = app.state::<Host>();
    let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
    plugins.store.visual_annotations(grant)?;
    let engine = host.lock()?;
    gate(app, &host, handle, Some(token))?;
    check_scene(&engine.store.snapshot(), scene, origin)
}
fn sample_ticks(range: VideoRange, other_count: usize) -> Result<Vec<u64>, String> {
    let count = 6usize
        .checked_sub(other_count)
        .filter(|count| *count >= 2)
        .ok_or("其它引用过多，请为视频保留至少两个帧附件")?;
    let duration = range
        .end_ticks
        .checked_sub(range.start_ticks)
        .filter(|duration| *duration >= media::MIN_RANGE_TICKS)
        .ok_or("视频范围过短或无效")?;
    let mut ticks = Vec::with_capacity(count);
    for index in 0..count {
        let offset = duration.checked_mul(index as u64).ok_or("视频范围无效")? / count as u64;
        ticks.push(
            range
                .start_ticks
                .checked_add(offset)
                .ok_or("视频范围无效")?,
        );
    }
    if ticks.first().copied() != Some(range.start_ticks)
        || ticks.last().is_none_or(|tick| *tick >= range.end_ticks)
        || ticks.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("视频采样时间无效".into());
    }
    Ok(ticks)
}

fn manifest(
    item_id: &str,
    source: VideoSourceFence,
    window: VideoRange,
    frames: &[media::FrameDescriptor],
    ids: &[String],
    offset: usize,
) -> Result<VerifiedVideoInput, String> {
    if frames.len() != ids.len() || offset + frames.len() > 6 {
        return Err("视频帧附件数量不匹配".into());
    }
    let mapped = frames
        .iter()
        .zip(ids)
        .enumerate()
        .map(|(index, (frame, id))| VideoInputFrame {
            handle: uuid::Uuid::new_v4().to_string(),
            attachment_id: id.clone(),
            attachment_ordinal: (offset + index) as u32,
            jpeg_sha256: frame.jpeg_sha256.clone(),
            encoded_width: frame.encoded_width,
            encoded_height: frame.encoded_height,
            source_pts_ticks: frame.actual_pts_ticks,
            sample_duration_ticks: frame.sample_duration_ticks,
            source_playback_ticks: frame.requested_ticks,
        })
        .collect();
    VerifiedVideoInput::new(VideoInputManifest {
        version: 1,
        profile_id: VIDEO_PROFILE_ID.into(),
        targets: vec![VideoInputTarget {
            handle: uuid::Uuid::new_v4().to_string(),
            item_id: item_id.into(),
            source,
            sent_window: window,
            range_end_handle: uuid::Uuid::new_v4().to_string(),
            frames: mapped,
        }],
    })
    .map_err(|error| error.to_string())
}
fn read_frame(
    path: &Path,
    frame: &media::FrameDescriptor,
    token: &AtomicBool,
) -> Result<Vec<u8>, String> {
    video_host::check_cancel(token)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(path).map_err(|_| "无法读取视频帧")?;
    let metadata = file.metadata().map_err(|_| "无法读取视频帧")?;
    if !metadata.is_file()
        || metadata.len() != frame.bytes_length
        || !(1..=media::MAX_FRAME_JPEG_BYTES).contains(&metadata.len())
    {
        return Err("视频帧已更改".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(media::MAX_FRAME_JPEG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取视频帧")?;
    if bytes.len() as u64 != metadata.len()
        || format!("{:x}", Sha256::digest(&bytes)) != frame.jpeg_sha256
    {
        return Err("视频帧已更改".into());
    }
    video_host::check_cancel(token)?;
    Ok(bytes)
}

/// No Engine lock crosses a file read, encoding operation or native worker wait.
pub async fn prepare(
    app: &AppHandle,
    scene: &Scene,
    grant: &VisualAnnotationGrant,
) -> Result<PreparedVideoRun, String> {
    let handle = space_handle(app)?;
    let origin = selected_video(scene)?;
    let host = app.state::<Host>();
    let registry = host.video.clone();
    let id = uuid::Uuid::new_v4().to_string();
    let (work, cancel) = {
        // Consistent with revoke/reconcile: plugins -> Engine -> registry.
        let plugins = host.plugins.lock().map_err(|_| "插件状态不可用")?;
        plugins.store.visual_annotations(grant)?;
        let engine = host.lock()?;
        gate(app, &host, handle, None)?;
        registry.begin(&id, origin.clone(), || {
            // Do not acquire the recording mutex while registry data is held.
            video_host::atomic_gate(&host, handle, false)?;
            check_scene(&engine.store.snapshot(), scene, &origin)
        })?
    };
    let mut waiter = WaitCancel {
        token: cancel.clone(),
        armed: true,
    };
    let resources = Resources {
        created: CreatedAttachments::default(),
        staging: None,
        source: None,
        invoke: video_host::InvokeCancel(cancel.clone()),
        _work: work,
    };
    let app = app.clone();
    let scene = scene.clone();
    let grant = grant.clone();
    let (send, receive) = oneshot::channel();
    // A detached owner continues cancelling, reaping and cleaning even if the
    // renderer drops this invoke. Failed send drops the entire owned result.
    tauri::async_runtime::spawn(async move {
        let result = prepare_owned(
            app, scene, origin, grant, registry, resources, cancel, handle,
        )
        .await;
        let _ = send.send(result);
    });
    let result = receive.await.map_err(|_| "视频输入准备中断")?;
    if result.is_ok() {
        waiter.armed = false;
    }
    result
}

async fn prepare_owned(
    app: AppHandle,
    scene: Scene,
    origin: VideoExportOrigin,
    grant: VisualAnnotationGrant,
    registry: video_host::VideoRegistry,
    mut resources: Resources,
    cancel: Arc<AtomicBool>,
    handle: usize,
) -> Result<PreparedVideoRun, String> {
    check_current(&app, &scene, &origin, &grant, &cancel, handle)?;
    let app2 = app.clone();
    let scene2 = scene.clone();
    let origin2 = origin.clone();
    let grant2 = grant.clone();
    let cancel2 = cancel.clone();
    let (returned, prepared) = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            check_current(&app2, &scene2, &origin2, &grant2, &cancel2, handle)?;
            let host = app2.state::<Host>();
            resources.source = Some(video_host::SourceLease::open(
                &origin2.source.asset,
                &host.assets,
            )?);
            let prepared = ai::prepare_video_context(&scene2, &host.assets)?;
            if prepared.attachment_count() > 4 {
                return Err("其它引用过多，请为视频保留至少两个帧附件".into());
            }
            let source_sha = resources
                .source
                .as_ref()
                .ok_or("视频来源租约已结束")?
                .source_sha256(&cancel2)?;
            check_current(&app2, &scene2, &origin2, &grant2, &cancel2, handle)?;
            Ok::<_, String>((prepared, source_sha))
        })();
        (resources, result)
    })
    .await
    .map_err(|_| "视频引用准备中断")?;
    resources = returned;
    let (prepared, source_sha) = prepared?;
    check_current(&app, &scene, &origin, &grant, &cancel, handle)?;
    let metadata = video_host::metadata(
        &registry,
        &origin.source.asset,
        resources.source.as_ref().ok_or("视频来源租约已结束")?,
        cancel.clone(),
    )
    .await?;
    check_current(&app, &scene, &origin, &grant, &cancel, handle)?;
    let proof = VerifiedVideoAnnotationSource::new(
        VerifiedVideoSource {
            asset: origin.source.asset.clone(),
            duration_ticks: metadata.duration_ticks,
            width: metadata.width,
            height: metadata.height,
        },
        source_sha,
    )
    .map_err(|error| error.to_string())?;
    let fence =
        mewu_core::video_source_fence(&origin.source, &proof).map_err(|error| error.to_string())?;
    let window = fence.range.unwrap_or(VideoRange {
        start_ticks: 0,
        end_ticks: metadata.duration_ticks,
    });
    let offset = prepared.attachment_count();
    let requested = sample_ticks(window, offset)?;
    media::validate_extraction_request(
        &metadata,
        &media::VideoRange {
            start_ticks: window.start_ticks,
            end_ticks: window.end_ticks,
        },
        &requested,
    )
    .map_err(|error| error.to_string())?;
    let root = app.state::<Host>().root.clone();
    let count = requested.len();
    resources = tauri::async_runtime::spawn_blocking(move || {
        video_host::check_cancel(&resources.invoke.0)?;
        resources.staging = Some(Staging::create(&root, count)?);
        Ok::<_, String>(resources)
    })
    .await
    .map_err(|_| "视频临时目录准备中断")??;
    check_current(&app, &scene, &origin, &grant, &cancel, handle)?;
    let output = resources
        .staging
        .as_ref()
        .ok_or("视频临时目录已结束")?
        .path
        .clone();
    let source = resources
        .source
        .as_ref()
        .ok_or("视频来源租约已结束")?
        .path()
        .to_path_buf();
    let decoded = video_host::native_job(
        media::ClipJob::ExtractFrames {
            source,
            output_directory: output.clone(),
            expected: metadata.clone(),
            window: media::VideoRange {
                start_ticks: window.start_ticks,
                end_ticks: window.end_ticks,
            },
            requested_ticks: requested.clone(),
        },
        cancel.clone(),
        Arc::new(|_| {}),
    )
    .await?;
    // native_job returns only after the real child/Job/pipe lifetime is idle.
    let frames = match decoded {
        media::ClipJobResult::FramesExtracted { frames } => frames,
        _ => return Err("视频采样结果无效".into()),
    };
    check_current(&app, &scene, &origin, &grant, &cancel, handle)?;
    let app2 = app.clone();
    let scene2 = scene.clone();
    let grant2 = grant.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_current(&app2, &scene2, &origin, &grant2, &cancel, handle)?;
        crate::video_annotation_render::validate_extracted_files(
            &output,
            &metadata,
            &media::VideoRange {
                start_ticks: window.start_ticks,
                end_ticks: window.end_ticks,
            },
            &requested,
            &frames,
            &cancel,
        )
        .map_err(|error| error.to_string())?;
        let ids: Vec<_> = frames
            .iter()
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        let input = manifest(&origin.item_id, fence, window, &frames, &ids, offset)?;
        let mut images = Vec::with_capacity(frames.len());
        for (frame, id) in frames.iter().zip(ids) {
            let jpeg = read_frame(&output.join(format!("{}.jpg", frame.index)), frame, &cancel)?;
            images.push(ai::VideoFrameImage {
                id,
                name: format!("视频帧 {}.jpg", frame.index + 1),
                width: frame.encoded_width,
                height: frame.encoded_height,
                jpeg,
            });
        }
        let host = app2.state::<Host>();
        let prepared = ai::append_video_frame_images(prepared, &scene2, &host.assets, images)?;
        check_current(&app2, &scene2, &origin, &grant2, &cancel, handle)?;
        // persist_attachments uses create_new and handles its partial failure.
        let assets = ai::persist_attachments(prepared, &host.assets)?;
        resources.created.paths = assets
            .iter()
            .map(|asset| PathBuf::from(&asset.path))
            .collect();
        if assets
            .iter()
            .any(|asset| Path::new(&asset.path).parent() != Some(host.assets.as_path()))
        {
            return Err("视频附件保存位置无效".into());
        }
        mewu_core::VerifiedVideoRunInput::new(grant2.clone(), input.clone(), assets.clone())
            .map_err(|error| error.to_string())?;
        check_current(&app2, &scene2, &origin, &grant2, &cancel, handle)?;
        // Exact temporary JPEG files are now unused. The source lease and actual
        // Work remain owned through the caller's atomic begin transaction.
        resources.staging.take();
        Ok::<_, String>(PreparedVideoRun {
            input,
            assets,
            app: app2,
            handle,
            resources,
        })
    })
    .await
    .map_err(|_| "视频帧附件准备中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_owns_only_exact_new_files_and_never_recursively_erases_unknown_files() {
        let root = std::env::temp_dir().join(format!("mewu-video-input-{}", uuid::Uuid::new_v4()));
        assert!(!root
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("c:\\hermes"));
        fs::create_dir(&root).unwrap();
        let original = root.join("original.mp4");
        fs::write(&original, b"unchanged source").unwrap();
        let new = root.join("new.jpg");
        fs::write(&new, b"new attachment").unwrap();
        drop(CreatedAttachments {
            paths: vec![new.clone()],
        });
        assert!(!new.exists());
        assert_eq!(fs::read(&original).unwrap(), b"unchanged source");
        let stage = Staging::create(&root, 2).unwrap();
        let directory = stage.path.clone();
        for index in 0..2 {
            fs::write(directory.join(format!("{index}.jpg")), b"owned frame").unwrap();
        }
        let unrelated = directory.join("unrelated.txt");
        fs::write(&unrelated, b"preserve").unwrap();
        drop(stage);
        assert!(!directory.join("0.jpg").exists());
        assert!(!directory.join("1.jpg").exists());
        assert_eq!(fs::read(&unrelated).unwrap(), b"preserve");
        assert_eq!(fs::read(&original).unwrap(), b"unchanged source");
        fs::remove_file(unrelated).unwrap();
        fs::remove_dir(directory).unwrap();
        fs::remove_dir(root.join("video-input-staging")).unwrap();
        fs::remove_file(original).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn sampling_reserves_real_frames_without_omitting_other_context() {
        let range = VideoRange {
            start_ticks: 3_000_000,
            end_ticks: 4_000_000,
        };
        assert_eq!(sample_ticks(range, 4).unwrap(), vec![3_000_000, 3_500_000]);
        let six = sample_ticks(range, 0).unwrap();
        assert_eq!(six.len(), 6);
        assert_eq!(six[0], range.start_ticks);
        assert!(six.windows(2).all(|p| p[0] < p[1]));
        assert!(six.iter().all(|tick| *tick < range.end_ticks));
        assert!(sample_ticks(range, 5).is_err());
        assert!(sample_ticks(
            VideoRange {
                start_ticks: 4_000_000,
                end_ticks: 3_000_000
            },
            0
        )
        .is_err());
        assert!(sample_ticks(
            VideoRange {
                start_ticks: 0,
                end_ticks: 999_999
            },
            0
        )
        .is_err());
    }
    #[test]
    fn manifest_keeps_held_actual_pts_separate_and_uses_full_attachment_ordinals() {
        let source = VideoSourceFence {
            clock: mewu_core::VideoSourceClock::SourcePlaybackTicks,
            source_id: uuid::Uuid::new_v4().to_string(),
            asset_sha256: "a".repeat(64),
            source_sha256: "b".repeat(64),
            source_duration_ticks: 10_000_000,
            source_width: 320,
            source_height: 180,
            range_revision: 1,
            range: Some(VideoRange {
                start_ticks: 4_000_000,
                end_ticks: 6_000_000,
            }),
            annotation_revision: 0,
            document_sha256: format!("{:x}", Sha256::digest(b"null")),
        };
        let frames: Vec<_> = [4_000_000, 5_000_000]
            .into_iter()
            .enumerate()
            .map(|(index, tick)| media::FrameDescriptor {
                index: index as u32,
                requested_ticks: tick,
                actual_pts_ticks: 3_000_000,
                sample_duration_ticks: 3_000_000,
                encoded_width: 160,
                encoded_height: 90,
                jpeg_sha256: "d".repeat(64),
                bytes_length: 1000,
            })
            .collect();
        let ids = vec![
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        let input = manifest(
            &uuid::Uuid::new_v4().to_string(),
            source.clone(),
            source.range.unwrap(),
            &frames,
            &ids,
            4,
        )
        .unwrap();
        let target = &input.manifest().targets[0];
        assert_eq!(target.frames[0].attachment_ordinal, 4);
        assert_eq!(target.frames[1].attachment_ordinal, 5);
        assert_eq!(target.frames[0].source_pts_ticks, 3_000_000);
        assert_eq!(target.frames[1].source_pts_ticks, 3_000_000);
        assert_eq!(target.frames[1].source_playback_ticks, 5_000_000);
        assert_ne!(target.handle, target.range_end_handle);
        assert_ne!(target.frames[0].handle, target.frames[1].handle);
        assert!(manifest(
            &uuid::Uuid::new_v4().to_string(),
            source.clone(),
            source.range.unwrap(),
            &frames,
            &ids,
            5
        )
        .is_err());
    }
}
