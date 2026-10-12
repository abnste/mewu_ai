// SPDX-License-Identifier: MPL-2.0
//! Session-restricted PipeWire frames, copied off the streaming thread. Native
//! GStreamer owns negotiation/DMABuf conversion; no raw SPA memory is reinterpreted.
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_video as video;
use std::{os::fd::AsRawFd, time::Duration};
use crate::recording_portal_linux::PortalSession;

pub(crate) struct Frame { pub width: u32, pub height: u32, pub rgba: Vec<u8> }
pub(crate) struct Frames { pipeline: gst::Pipeline, sink: gstreamer_app::AppSink }
fn element(name: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(name).build().map_err(|_| "屏幕采集组件不可用".into())
}
impl Drop for Frames { fn drop(&mut self) { let _ = self.pipeline.set_state(gst::State::Null); } }
impl Frames {
    pub(crate) fn connect(session: &PortalSession) -> Result<Self, String> {
        gst::init().map_err(|_| "媒体组件不可用")?;
        let pipeline = gst::Pipeline::new();
        let source = element("pipewiresrc")?;
        // libpipewire's context retains/duplicates this FD for its own connection.
        // The caller must keep the original session+FD alive until Frames drops.
        source.set_property("fd", session.remote.as_raw_fd());
        source.set_property("path", session.node.to_string());
        source.set_property("always-copy", true);
        source.set_property("max-buffers", 4i32);
        let convert = element("videoconvert")?;
        let caps = element("capsfilter")?;
        caps.set_property("caps", gst::Caps::builder("video/x-raw").field("format", "RGBA").build());
        let sink = element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_| "帧输出类型无效")?;
        sink.set_max_buffers(2); sink.set_drop(true); sink.set_sync(false);
        sink.set_property("enable-last-sample", false);
        pipeline.add_many([&source, &convert, &caps, sink.upcast_ref()]).map_err(|_| "无法准备屏幕采集")?;
        gst::Element::link_many([&source, &convert, &caps, sink.upcast_ref()]).map_err(|_| "无法连接屏幕采集")?;
        let frames = Self { pipeline, sink };
        frames.pipeline.set_state(gst::State::Playing).map_err(|_| "无法启动屏幕采集")?;
        Ok(frames)
    }
    pub(crate) fn next(&self, wait: Duration) -> Result<Option<Frame>, String> {
        let bus = self.pipeline.bus().ok_or("屏幕采集总线不可用")?;
        for _ in 0..16 {
            let Some(message) = bus.pop() else { break; };
            if matches!(message.view(), gst::MessageView::Error(_) | gst::MessageView::Eos(_)) {
                return Err("屏幕共享已结束".into());
            }
        }
        let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos().min(u64::MAX as u128) as u64)) else { return Ok(None); };
        let info = video::VideoInfo::from_caps(sample.caps().ok_or("屏幕帧格式缺失")?).map_err(|_| "屏幕帧格式无效")?;
        if info.format() != video::VideoFormat::Rgba || info.width() < 2 || info.height() < 2
            || info.width() > 16_384 || info.height() > 16_384
            || u64::from(info.width()) * u64::from(info.height()) > 32_000_000 { return Err("屏幕帧尺寸无效".into()); }
        let frame = video::VideoFrameRef::from_buffer_ref_readable(sample.buffer().ok_or("屏幕帧缺失")?, &info)
            .map_err(|_| "无法读取屏幕帧")?;
        let stride = frame.plane_stride()[0];
        if stride <= 0 { return Err("屏幕帧方向无效".into()); }
        let row = info.width() as usize * 4;
        let data = frame.plane_data(0).map_err(|_| "屏幕帧平面无效")?;
        let rgba = packed_rows(data, stride as usize, row, info.height() as usize)?;
        Ok(Some(Frame { width: info.width(), height: info.height(), rgba }))
    }
}
fn packed_rows(data: &[u8], stride: usize, row: usize, height: usize) -> Result<Vec<u8>, String> {
    let end = height.checked_sub(1).and_then(|h| h.checked_mul(stride)).and_then(|v| v.checked_add(row)).ok_or("屏幕帧尺寸无效")?;
    if row == 0 || stride < row || data.len() < end { return Err("屏幕帧长度无效".into()); }
    let mut out = Vec::with_capacity(row.checked_mul(height).ok_or("屏幕帧尺寸无效")?);
    for y in 0..height { out.extend_from_slice(&data[y * stride..y * stride + row]); }
    Ok(out)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn padding_is_not_interpreted_as_pixels() {
        assert_eq!(packed_rows(&[1,2,3,4,99,99,5,6,7,8],6,4,2).unwrap(),[1,2,3,4,5,6,7,8]);
        assert!(packed_rows(&[0;10],3,4,2).is_err());
        assert!(packed_rows(&[0;9],6,4,2).is_err());
    }
}
