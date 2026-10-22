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
        configure_target(&source, session.node, session.pipewire_serial)?;
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
        self.next_after(wait,None)
    }
    pub(crate) fn barrier(&self)->Result<u64,String>{
        self.pipeline.current_running_time().map(|v|v.nseconds()).ok_or_else(||"无法确认屏幕帧时间".into())
    }
    /// A feedback frame must be captured after the caller's barrier, rather
    /// than merely dequeued after input. Existing recording callers retain
    /// their ordinary next() semantics.
    pub(crate) fn next_after(&self, wait:Duration, minimum:Option<u64>)->Result<Option<Frame>,String>{
        let bus = self.pipeline.bus().ok_or("屏幕采集总线不可用")?;
        for _ in 0..16 {
            let Some(message) = bus.pop() else { break; };
            if matches!(message.view(), gst::MessageView::Error(_) | gst::MessageView::Eos(_)) {
                return Err("屏幕共享已结束".into());
            }
        }
        let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos().min(u64::MAX as u128) as u64)) else { return Ok(None); };
        if let Some(minimum)=minimum {
            let pts=sample.buffer().and_then(|b|b.pts()).ok_or("屏幕帧时间缺失")?;
            let segment=sample.segment().and_then(|s|s.downcast_ref::<gst::ClockTime>()).ok_or("屏幕帧时间轴无效")?;
            let time=segment.to_running_time(pts).ok_or("屏幕帧时间轴无效")?;
            if time.nseconds()<minimum{return Ok(None)}
        }
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

fn configure_target(source: &gst::Element, node: u32, serial: Option<u64>) -> Result<(), String> {
    if let Some(serial) = serial {
        if serial == 0 || source.find_property("target-object").is_none() {
            return Err("屏幕采集组件不支持授权流标识".into());
        }
        // Do not also set the deprecated numerical path: it can name a different
        // stream after hotplug or suspend. The serial comes only from this
        // portal response, never a plugin argument or the unrestricted daemon.
        source.set_property("target-object", serial.to_string());
    } else {
        if node == 0 { return Err("屏幕共享流标识无效".into()); }
        source.set_property("path", node.to_string());
    }
    Ok(())
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
    #[test] fn fresh_frame_barrier_discards_queued_pre_hide_pixels_on_a_real_pipeline() {
        gst::init().unwrap();
        let pipeline=gst::Pipeline::new();
        let source=element("appsrc").unwrap().downcast::<gstreamer_app::AppSrc>().unwrap();
        source.set_format(gst::Format::Time);source.set_property("is-live",true);
        source.set_caps(Some(&gst::Caps::builder("video/x-raw").field("format","RGBA")
            .field("width",2i32).field("height",2i32).field("framerate",gst::Fraction::new(30,1)).build()));
        let sink=element("appsink").unwrap().downcast::<gstreamer_app::AppSink>().unwrap();
        sink.set_sync(false);sink.set_property("async",false);
        pipeline.add_many([source.upcast_ref::<gst::Element>(),sink.upcast_ref::<gst::Element>()]).unwrap();source.link(&sink).unwrap();
        let frames=Frames{pipeline,sink};frames.pipeline.set_state(gst::State::Playing).unwrap();
        let push=|pts,pixel:&[u8]|{let mut buffer=gst::Buffer::from_mut_slice(pixel.repeat(4));
            let writable=buffer.get_mut().unwrap();writable.set_pts(gst::ClockTime::from_nseconds(pts));
            writable.set_duration(gst::ClockTime::from_mseconds(33));source.push_buffer(buffer).unwrap();};
        push(0,&[255,0,0,255]);
        std::thread::sleep(Duration::from_millis(2));
        let barrier=frames.barrier().unwrap();assert!(barrier>0);
        assert!(frames.next_after(Duration::from_secs(1),Some(barrier)).unwrap().is_none());
        push(barrier+1,&[0,0,255,255]);
        let fresh=frames.next_after(Duration::from_secs(1),Some(barrier)).unwrap().unwrap();
        assert_eq!((fresh.width,fresh.height),(2,2));assert_eq!(fresh.rgba,[0,0,255,255].repeat(4));
        assert!(frames.barrier().unwrap()>=barrier);
    }
    #[test] fn authorized_serial_is_used_without_a_numeric_node_fallback() {
        gst::init().unwrap();
        let source = element("pipewiresrc").unwrap();
        configure_target(&source, 42, Some(u64::from(u32::MAX) + 1)).unwrap();
        assert_eq!(source.property::<Option<String>>("target-object").as_deref(), Some("4294967296"));
        assert_eq!(source.property::<Option<String>>("path"), None);
        assert!(configure_target(&source, 42, Some(0)).is_err());
        let legacy = element("pipewiresrc").unwrap();
        configure_target(&legacy, 42, None).unwrap();
        assert_eq!(legacy.property::<Option<String>>("path").as_deref(), Some("42"));
        assert!(configure_target(&legacy, 0, None).is_err());
        let unrelated = element("fakesrc").unwrap();
        assert!(configure_target(&unrelated, 42, Some(7)).is_err());
    }
    #[test] fn padding_is_not_interpreted_as_pixels() {
        assert_eq!(packed_rows(&[1,2,3,4,99,99,5,6,7,8],6,4,2).unwrap(),[1,2,3,4,5,6,7,8]);
        assert!(packed_rows(&[0;10],3,4,2).is_err());
        assert!(packed_rows(&[0;9],6,4,2).is_err());
    }
}
