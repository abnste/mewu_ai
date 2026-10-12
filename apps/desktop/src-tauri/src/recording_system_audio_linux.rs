// SPDX-License-Identifier: MPL-2.0
//! User-session default output monitor, never the default microphone. This
//! first probe yields real PCM; AAC acceptance remains an independent gate.
use gst::prelude::*;
use gstreamer as gst;
pub(crate) struct SystemAudio { pipeline: gst::Pipeline, sink: gstreamer_app::AppSink }
impl Drop for SystemAudio { fn drop(&mut self) { let _ = self.pipeline.set_state(gst::State::Null); } }
impl SystemAudio {
    pub(crate) fn connect() -> Result<Self, String> {
        gst::init().map_err(|_| "媒体组件不可用")?;
        let pipeline = gst::Pipeline::new();
        let element = |name| gst::ElementFactory::make(name).build().map_err(|_| "电脑声音组件不可用".to_owned());
        let source = element("pulsesrc")?;
        // PulseAudio and pipewire-pulse resolve this reserved name to the
        // default sink's monitor. An absent output is an error, not mic fallback.
        source.set_property("device", "@DEFAULT_MONITOR@");
        source.set_property("buffer-time", 200_000i64);
        source.set_property("latency-time", 20_000i64);
        let convert = element("audioconvert")?; let resample = element("audioresample")?;
        let caps = element("capsfilter")?;
        caps.set_property("caps", gst::Caps::builder("audio/x-raw").field("format", "S16LE").field("layout", "interleaved")
            .field("rate",48_000i32).field("channels",2i32).build());
        let sink = element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_| "声音输出类型无效")?;
        sink.set_max_buffers(8); sink.set_drop(false); sink.set_sync(false); sink.set_property("enable-last-sample",false);
        pipeline.add_many([&source,&convert,&resample,&caps,sink.upcast_ref()]).map_err(|_| "无法准备电脑声音")?;
        gst::Element::link_many([&source,&convert,&resample,&caps,sink.upcast_ref()]).map_err(|_| "无法连接电脑声音")?;
        let audio = Self { pipeline, sink };
        audio.pipeline.set_state(gst::State::Playing).map_err(|_| "无法启动电脑声音")?;
        Ok(audio)
    }
    pub(crate) fn poll(&self) -> Result<Option<Vec<u8>>,String> {
        let bus=self.pipeline.bus().ok_or("声音总线不可用")?;
        for _ in 0..16 { let Some(message)=bus.pop() else{break;};
            if matches!(message.view(),gst::MessageView::Error(_)|gst::MessageView::Eos(_)){ return Err("电脑声音已中断".into()); } }
        let Some(sample)=self.sink.try_pull_sample(gst::ClockTime::ZERO) else{return Ok(None);};
        let map=sample.buffer().ok_or("声音帧缺失")?.map_readable().map_err(|_|"无法读取电脑声音")?;
        if map.len()==0 || map.len()%4!=0 || map.len()>96_000 { return Err("电脑声音帧长度无效".into()); }
        Ok(Some(map.to_vec()))
    }
}
