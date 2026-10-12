// SPDX-License-Identifier: MPL-2.0
//! Fixed native codec preparation, currently included only by the explicit Linux
//! recording fixture. No production host/worker or GUI route imports this module.
//! The caller must own the protected output file and the original hard deadline.
//! Native shutdown/fsync may block: use the real supervised worker process before
//! any production admission, and retain its permit until Child/PID1/IO drain.

use gst::prelude::*;
use gstreamer as gst;
use std::{
    fs::File,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

const FPS: u64 = 15;
const MAX_PIXELS: u64 = 4_000_000;
const QUEUED_FRAMES: u64 = 2;

pub(crate) struct Codec {
    pipeline: gst::Pipeline,
    source: gstreamer_app::AppSrc,
    file: File,
    expires: Instant,
    bytes: usize,
    frames: u64,
}

fn element(name: &'static str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| "本机固定录屏编码组件不可用".into())
}

impl Codec {
    pub(crate) fn new(
        file: File,
        width: u32,
        height: u32,
        expires: Instant,
    ) -> Result<Self, String> {
        let now = Instant::now();
        if width < 2
            || height < 2
            || width % 2 != 0
            || height % 2 != 0
            || u64::from(width) * u64::from(height) > MAX_PIXELS
            || expires <= now
            || expires.duration_since(now) > Duration::from_secs(600)
        {
            return Err("录屏编码范围或原始截止时间无效".into());
        }
        let metadata = file.metadata().map_err(|_| "无法核对录屏输出文件")?;
        if !metadata.is_file() || metadata.len() != 0 {
            return Err("录屏输出必须是宿主新建的空普通文件".into());
        }
        gst::init().map_err(|_| "本机录屏媒体组件不可用")?;
        let pipeline = gst::Pipeline::new();
        let source = element("appsrc")?
            .downcast::<gstreamer_app::AppSrc>()
            .map_err(|_| "录屏帧源组件类型无效")?;
        let convert = element("videoconvert")?;
        let raw = element("capsfilter")?;
        let encoder = element("openh264enc")?;
        let parser = element("h264parse")?;
        let compressed = element("capsfilter")?;
        let mux = element("mp4mux")?;
        let sink = element("fdsink")?;
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|size| size.checked_mul(4))
            .ok_or("录屏像素尺寸无效")?;
        source.set_caps(Some(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", width as i32)
                .field("height", height as i32)
                .field("framerate", gst::Fraction::new(FPS as i32, 1))
                .build(),
        ));
        source.set_format(gst::Format::Time);
        source.set_max_buffers(QUEUED_FRAMES);
        source.set_max_bytes(QUEUED_FRAMES * bytes as u64);
        source.set_property("block", false);
        source.set_property("emit-signals", false);
        raw.set_property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "I420")
                .build(),
        );
        encoder.set_property("multi-thread", 1u32);
        compressed.set_property(
            "caps",
            gst::Caps::builder("video/x-h264")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .build(),
        );
        sink.set_property("fd", file.as_raw_fd());
        sink.set_property("sync", false);
        let nodes: [&gst::Element; 8] = [
            source.upcast_ref(),
            &convert,
            &raw,
            &encoder,
            &parser,
            &compressed,
            &mux,
            &sink,
        ];
        pipeline
            .add_many(nodes)
            .map_err(|_| "无法准备固定录屏编码图")?;
        gst::Element::link_many(nodes).map_err(|_| "无法连接固定录屏编码图")?;
        let codec = Self {
            pipeline,
            source,
            file,
            expires,
            bytes,
            frames: 0,
        };
        codec
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "无法启动本机录屏编码图")?;
        Ok(codec)
    }

    fn check(&self, allowed: &impl Fn() -> bool) -> Result<(), String> {
        if Instant::now() >= self.expires || !allowed() {
            return Err("录屏原始预算或宿主授权已结束".into());
        }
        if let Some(message) = self
            .pipeline
            .bus()
            .ok_or("录屏总线不可用")?
            .pop_filtered(&[gst::MessageType::Error])
        {
            if matches!(message.view(), gst::MessageView::Error(_)) {
                return Err("本机录屏编码失败".into());
            }
        }
        Ok(())
    }

    // Sole producer is enforced by &mut self. appsrc's max settings alone are
    // insufficient with block=false; check actual levels before every push.
    pub(crate) fn push_rgba(
        &mut self,
        frame: Vec<u8>,
        allowed: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.check(allowed)?;
        if frame.len() != self.bytes || self.frames >= FPS * 600 {
            return Err("录屏原始帧长度或数量无效".into());
        }
        while self.source.current_level_buffers() >= QUEUED_FRAMES
            || self.source.current_level_bytes() >= QUEUED_FRAMES * self.bytes as u64
        {
            self.check(allowed)?;
            std::thread::sleep(Duration::from_millis(1));
        }
        self.check(allowed)?;
        let mut buffer = gst::Buffer::from_mut_slice(frame);
        let writable = buffer.get_mut().ok_or("无法准备录屏帧")?;
        writable.set_pts(gst::ClockTime::from_nseconds(
            self.frames * 1_000_000_000 / FPS,
        ));
        writable.set_duration(gst::ClockTime::from_nseconds(1_000_000_000 / FPS));
        self.source
            .push_buffer(buffer)
            .map_err(|_| "无法提交录屏帧")?;
        self.frames += 1;
        if self.source.current_level_buffers() > QUEUED_FRAMES
            || self.source.current_level_bytes() > QUEUED_FRAMES * self.bytes as u64
        {
            return Err("录屏帧队列超出固定上限".into());
        }
        Ok(())
    }

    pub(crate) fn finish(self, allowed: &impl Fn() -> bool) -> Result<u64, String> {
        self.check(allowed)?;
        if self.frames == 0 {
            return Err("录屏未产生任何帧".into());
        }
        self.source
            .end_of_stream()
            .map_err(|_| "无法结束录屏帧源")?;
        let bus = self.pipeline.bus().ok_or("录屏总线不可用")?;
        loop {
            self.check(allowed)?;
            if let Some(message) = bus.timed_pop(gst::ClockTime::from_mseconds(20)) {
                match message.view() {
                    gst::MessageView::Eos(_) => break,
                    gst::MessageView::Error(_) => return Err("本机录屏文件最终化失败".into()),
                    _ => {}
                }
            }
        }
        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|_| "无法停止本机录屏编码图")?;
        let (result, state, _) = self.pipeline.state(gst::ClockTime::ZERO);
        result.map_err(|_| "无法核对录屏编码图退出状态")?;
        if state != gst::State::Null {
            return Err("录屏编码图退出尚未证实".into());
        }
        self.check(allowed)?;
        self.file.sync_all().map_err(|_| "无法完成录屏文件写入")?;
        self.check(allowed)?;
        Ok(self.frames)
    }
}

impl Drop for Codec {
    fn drop(&mut self) {
        // Real synchronous native shutdown, including error paths. No detached
        // cleanup thread, timeout wrapper, terminal receipt or permit release.
        // Failure here must never be interpreted as proven worker-domain drain.
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
