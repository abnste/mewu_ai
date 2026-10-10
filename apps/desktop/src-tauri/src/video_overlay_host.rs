// SPDX-License-Identifier: MPL-2.0
//! Exact native preview rasters, leased until the media child has really exited.
use crate::{video_annotation_raster as raster, video_clip_contract as media};
use mewu_core::{Store, VideoExportOrigin};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) struct StagedOverlays {
    directory: PathBuf,
    paths: Vec<PathBuf>,
    leases: Vec<File>,
    pub overlays: Vec<media::OverlayRaster>,
}
impl Drop for StagedOverlays {
    fn drop(&mut self) {
        self.leases.clear();
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}
pub(crate) fn has_overlays(origin: &VideoExportOrigin) -> bool {
    origin.source.annotations.as_ref().is_some_and(|doc| {
        let range = origin
            .source
            .edit
            .as_ref()
            .and_then(|edit| edit.range)
            .unwrap_or(mewu_core::VideoRange {
                start_ticks: 0,
                end_ticks: doc.source_duration_ticks,
            });
        doc.objects.iter().any(|object| {
            object.interval.start_ticks < range.end_ticks
                && object.interval.end_ticks > range.start_ticks
        })
    })
}
impl StagedOverlays {
    pub(crate) fn create(
        root: &Path,
        origin: &VideoExportOrigin,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        let doc = origin
            .source
            .annotations
            .as_ref()
            .ok_or("视频标注文档不存在")?;
        let staging_root = root.join("video-overlay-staging");
        std::fs::create_dir_all(&staging_root).map_err(|_| "无法创建视频处理目录")?;
        let parent = staging_root
            .canonicalize()
            .map_err(|_| "视频处理目录不可用")?;
        let data_root = root.canonicalize().map_err(|_| "数据目录不可用")?;
        if !parent.starts_with(&data_root) || parent == data_root {
            return Err("视频处理目录无效".into());
        }
        let directory = parent.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&directory).map_err(|_| "无法创建视频处理目录")?;
        let mut result = Self {
            directory,
            paths: vec![],
            leases: vec![],
            overlays: vec![],
        };
        let range = origin
            .source
            .edit
            .as_ref()
            .and_then(|edit| edit.range)
            .unwrap_or(mewu_core::VideoRange {
                start_ticks: 0,
                end_ticks: doc.source_duration_ticks,
            });
        let (mut pixels, mut bytes) = (0u64, 0usize);
        for object in &doc.objects {
            check()?;
            if object.interval.start_ticks >= range.end_ticks
                || object.interval.end_ticks <= range.start_ticks
            {
                continue;
            }
            let layer = raster::render_in_source(
                object,
                doc.source_width,
                doc.source_height,
                |reference| {
                    Store::read_video_text_layout(root.join("spaces.db"), reference)
                        .map_err(|e| e.to_string())
                },
                |reference| {
                    Store::read_video_vector_layout(root.join("spaces.db"), reference)
                        .map_err(|e| e.to_string())
                },
            )?;
            pixels = pixels
                .checked_add(u64::from(layer.reference.width) * u64::from(layer.reference.height))
                .ok_or("视频标注图层过大")?;
            bytes = bytes
                .checked_add(layer.png.len())
                .ok_or("视频标注图片过大")?;
            if pixels > raster::MAX_TOTAL_PIXELS || bytes > raster::MAX_TOTAL_PNG_BYTES {
                return Err("视频标注图层预算超限".into());
            }
            let path = result
                .directory
                .join(format!("{}.png", uuid::Uuid::new_v4()));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| "无法保存视频标注图层")?;
            result.paths.push(path.clone());
            file.write_all(&layer.png)
                .and_then(|_| file.flush())
                .and_then(|_| file.sync_all())
                .map_err(|_| "无法保存视频标注图层")?;
            drop(file);
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(1);
            }
            let lease = options.open(&path).map_err(|_| "视频标注图层无法读取")?;
            result.leases.push(lease);
            result.overlays.push(media::OverlayRaster {
                png: path,
                png_sha256: layer.reference.png_sha256,
                width: layer.reference.width,
                height: layer.reference.height,
                bounds: media::OverlayBounds {
                    x: layer.bounds.x,
                    y: layer.bounds.y,
                    width: layer.bounds.width,
                    height: layer.bounds.height,
                },
                interval: media::VideoRange {
                    start_ticks: object.interval.start_ticks,
                    end_ticks: object.interval.end_ticks,
                },
            });
        }
        check()?;
        Ok(result)
    }
}
