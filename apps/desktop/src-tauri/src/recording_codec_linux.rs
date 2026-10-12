// SPDX-License-Identifier: MPL-2.0
//! Fixed native codec used by the supervised Linux Wayland recording worker.
//! Synthetic codec tests exercise the same bounded production pipeline.
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
#[derive(Clone,Copy,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub(crate) enum FinalizeStage { AudioTail, VideoEos, AacTail, MuxEos, FileSync, CloseCapture }
const QUEUED_FRAMES: u64 = 2;

pub(crate) struct Codec {
    pipeline: gst::Pipeline,
    source: gstreamer_app::AppSrc,
    file: File,
    expires: Instant,
    bytes: usize,
    frames: u64,
    audio: Option<Audio>,
}

struct Audio { source:gstreamer_app::AppSrc, encoder:crate::recording_aac_linux::Encoder, pcm:u64, packets:u64 }

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
        Self::with_audio(file,width,height,expires,false)
    }
    pub(crate) fn with_audio(file:File,width:u32,height:u32,expires:Instant,audio_enabled:bool)->Result<Self,String>{
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
        mux.set_property("movie-timescale",48000u32);
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
        let audio=if audio_enabled {
            let encoder=crate::recording_aac_linux::Encoder::new()?;
            let source=element("appsrc")?.downcast::<gstreamer_app::AppSrc>().map_err(|_|"AAC帧源类型无效")?;
            source.set_caps(Some(&gst::Caps::builder("audio/mpeg").field("mpegversion",4i32).field("stream-format","raw")
                .field("framed",true).field("rate",48000i32).field("channels",2i32)
                .field("codec_data",gst::Buffer::from_mut_slice(encoder.configuration().to_vec())).build()));
            source.set_format(gst::Format::Time);source.set_max_buffers(16);source.set_max_bytes(262144);
            source.set_property("block",false);source.set_property("emit-signals",false);source.set_property("handle-segment-change",true);
            pipeline.add(&source).map_err(|_|"无法准备AAC帧源")?;source.link(&mux).map_err(|_|"无法连接AAC帧源")?;
            Some(Audio{source,encoder,pcm:0,packets:0})
        } else {None};
        let codec = Self {
            pipeline,
            source,
            file,
            expires,
            bytes,
            frames: 0,
            audio,
        };
        codec
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| "无法启动本机录屏编码图")?;
        Ok(codec)
    }

    pub(crate) fn pcm_frames(&self)->u64 {self.audio.as_ref().map_or(0,|a|a.pcm)}
    #[cfg(debug_assertions)]
    pub(crate) fn input_levels(&self)->(u64,u64){
        (self.source.current_level_buffers(),self.audio.as_ref().map_or(0,|audio|audio.source.current_level_buffers()))
    }
    pub(crate) fn push_pcm(&mut self,bytes:&[u8],allowed:&impl Fn()->bool)->Result<(),String>{
        self.check_feeding(allowed)?;
        if bytes.is_empty()||bytes.len()%4!=0||bytes.len()>96000{return Err("电脑声音帧长度无效".into());}
        let audio=self.audio.as_mut().ok_or("录屏没有音轨")?;
        if audio.pcm+bytes.len()as u64/4>48000*600+48000{return Err("电脑声音超出预算".into());}
        let samples:Vec<_>=bytes.chunks_exact(2).map(|v|i16::from_le_bytes([v[0],v[1]])).collect();
        let source=&audio.source;let delay=audio.encoder.info.nDelayCore;let frame=audio.encoder.info.frameLength;
        let packets=&mut audio.packets;let expires=self.expires;
        audio.encoder.feed(&samples,|encoded|{
            push_aac(source,encoded,packets,delay,frame,expires,allowed)
        })?;audio.pcm+=bytes.len()as u64/4;Ok(())
    }

    fn check(&self, allowed: &impl Fn() -> bool) -> Result<(), String> {
        if Instant::now() >= self.expires || !allowed() {
            return Err("录屏原始预算或宿主授权已结束".into());
        }
        Ok(())
    }

    fn check_feeding(&self, allowed: &impl Fn() -> bool) -> Result<(), String> {
        self.check(allowed)?;
        let bus = self.pipeline.bus().ok_or("录屏总线不可用")?;
        // pop_filtered discards other messages, including EOS. Never use it
        // while finalizing: the finish loop must consume the real EOS itself.
        // Bound progress polling so native message traffic cannot starve the
        // original deadline or host cancellation checks.
        for _ in 0..16 {
            let Some(message) = bus.pop() else { break };
            match message.view() {
                gst::MessageView::Error(_) => return Err("本机录屏编码失败".into()),
                gst::MessageView::Eos(_) => return Err("录屏编码图提前结束".into()),
                _ => {}
            }
        }
        Ok(())
    }

    // Sole producer is enforced by &mut self. appsrc's max settings alone are
    // insufficient with block=false; check actual levels before every push.
    pub(crate) fn video_ready(&self,allowed:&impl Fn()->bool)->Result<bool,String>{
        self.check_feeding(allowed)?;
        Ok(self.source.current_level_buffers()<QUEUED_FRAMES
            && self.source.current_level_bytes()<QUEUED_FRAMES*self.bytes as u64)
    }
    pub(crate) fn push_rgba(
        &mut self,
        frame: Vec<u8>,
        allowed: &impl Fn() -> bool,
    ) -> Result<(), String> {
        self.check_feeding(allowed)?;
        if frame.len() != self.bytes || self.frames >= FPS * 600 {
            return Err("录屏原始帧长度或数量无效".into());
        }
        while !self.video_ready(allowed)? {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.check_feeding(allowed)?;
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
        self.finish_with_progress(allowed, |_|Ok(()))
    }
    pub(crate) fn finish_with_progress(mut self, allowed: &impl Fn() -> bool,
        mut progress:impl FnMut(FinalizeStage)->Result<(),String>) -> Result<u64, String> {
        self.check(allowed)?;
        if self.frames == 0 {
            return Err("录屏未产生任何帧".into());
        }
        let mut audio_end=None;
        // mp4mux is an aggregator: draining AAC while its video input is still
        // open can fill the bounded AAC queue and wait for a video buffer that
        // this same producer will never submit. Queue video EOS first.
        progress(FinalizeStage::VideoEos)?;
        self.source.end_of_stream().map_err(|_|"无法结束录屏帧源")?;
        if let Some(audio)=self.audio.as_mut(){
            let required=self.frames*48000/FPS;
            if audio.pcm<required{return Err("电脑声音未覆盖录屏尾点".into());}
            let source=&audio.source;let delay=audio.encoder.info.nDelayCore;let frame=audio.encoder.info.frameLength;
            let packets=&mut audio.packets;let expires=self.expires;
            progress(FinalizeStage::AacTail)?;
            audio.encoder.drain(|encoded|push_aac(source,encoded,packets,delay,frame,expires,allowed))?;
            source.end_of_stream().map_err(|_|"无法结束AAC帧源")?;
            audio_end=Some((required,delay,audio.packets*frame as u64));
        }
        progress(FinalizeStage::MuxEos)?;
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
        if let Some((pcm,delay,packets))=audio_end{crate::recording_mp4_audio_linux::finalize_edit(&self.file,pcm,delay,packets)?;}
        progress(FinalizeStage::FileSync)?;
        self.file.sync_all().map_err(|_| "无法完成录屏文件写入")?;
        self.check(allowed)?;
        Ok(self.frames)
    }
}

fn push_aac(source:&gstreamer_app::AppSrc,encoded:Vec<u8>,packets:&mut u64,delay:u32,frame:u32,
    expires:Instant,allowed:&impl Fn()->bool)->Result<(),String>{
    while source.current_level_buffers()>=16||source.current_level_bytes()>=262144{
        if Instant::now()>=expires||!allowed(){return Err("AAC编码已取消或超时".into());}
        std::thread::sleep(Duration::from_millis(1));
    }
    let pts=*packets*frame as u64*1_000_000_000/48000;
    let dts=*packets*frame as u64*1_000_000_000/48000;
    let mut buffer=gst::Buffer::from_mut_slice(encoded);let writable=buffer.get_mut().ok_or("AAC帧不可写")?;
    writable.set_pts(gst::ClockTime::from_nseconds(pts));writable.set_dts(gst::ClockTime::from_nseconds(dts));
    writable.set_duration(gst::ClockTime::from_nseconds(frame as u64*1_000_000_000/48000));
    let _=delay;
    source.push_buffer(buffer).map_err(|_|"无法提交AAC帧")?;*packets+=1;Ok(())
}

impl Drop for Codec {
    fn drop(&mut self) {
        // Real synchronous native shutdown, including error paths. No detached
        // cleanup thread, timeout wrapper, terminal receipt or permit release.
        // Failure here must never be interpreted as proven worker-domain drain.
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]mod audio_tests{
    use super::*;
    #[test]fn recording_wayland_mux_preserves_real_aac_delay_full_pcm_and_tail(){
        roundtrip("faad");
    }
    #[test]fn recording_wayland_libav_mux_preserves_real_aac_delay_full_pcm_and_tail(){
        roundtrip("avdec_aac");
    }
    #[test]fn recording_wayland_libav_parsed_sample_lattice_preserves_head_body_and_tail(){
        for (frames,extra) in [(15u32,0usize),(30,37),(31,0)]{roundtrip_case("avdec_aac",frames,extra,true);}
    }
    fn roundtrip(decoder:&'static str){
        roundtrip_case(decoder,30,0,false);
    }
    fn roundtrip_case(decoder:&'static str,frame_count:u32,extra:usize,quantized:bool){
        use std::os::unix::fs::OpenOptionsExt;
        let path=std::env::temp_dir().join(format!("mewu-aac-{}.mp4",uuid::Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}
        let _cleanup=Cleanup(path.clone());
        let file=std::fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path).unwrap();
        let expires=Instant::now()+Duration::from_secs(15);
        let mut codec=Codec::with_audio(file,64,48,expires,true).unwrap();
        let actual_delay=codec.audio.as_ref().unwrap().encoder.info.nDelayCore as u64;
        let frame_samples=codec.audio.as_ref().unwrap().encoder.info.frameLength as u64;
        let expected_samples=frame_count as usize*3200;
        let input_samples=expected_samples+extra;
        let mut expected=Vec::new();
        for index in 0..frame_count{
            let mut pcm=Vec::new();for sample in index*3200..(index+1)*3200{
                let frequency=if (sample as usize)<expected_samples-4800{440.}else{880.};
                let value=((sample as f64*frequency*std::f64::consts::TAU/48000.).sin()*12000.)as i16;
                expected.push(value);pcm.extend_from_slice(&value.to_le_bytes());pcm.extend_from_slice(&value.to_le_bytes());
            }
            codec.push_pcm(&pcm,&||true).unwrap();
            codec.push_rgba([20,120,220,255].repeat(64*48),&||true).unwrap();
        }
        if extra>0{let pcm:Vec<_>=(expected_samples..input_samples).flat_map(|sample|{
            let value=((sample as f64*880.*std::f64::consts::TAU/48000.).sin()*12000.)as i16;
            [value.to_le_bytes(),value.to_le_bytes()].concat()}).collect();codec.push_pcm(&pcm,&||true).unwrap();}
        assert_eq!(codec.finish(&||true).unwrap(),frame_count as u64);
        std::fs::copy(&path,format!("/tmp/last-aac-roundtrip-{decoder}.mp4")).unwrap();
        let pipeline=gst::Pipeline::new();
        struct Native(gst::Pipeline);impl Drop for Native{fn drop(&mut self){let _=self.0.set_state(gst::State::Null);}}
        let _native=Native(pipeline.clone());
        let input=element("filesrc").unwrap();input.set_property("location",path.to_str().unwrap());
        let demux=element("qtdemux").unwrap();let parse=element("aacparse").unwrap();let decode=element(decoder).unwrap();
        let convert=element("audioconvert").unwrap();let caps=element("capsfilter").unwrap();
        caps.set_property("caps",gst::Caps::builder("audio/x-raw").field("format","S16LE").field("rate",48000i32).field("channels",2i32).build());
        let sink=element("appsink").unwrap().downcast::<gstreamer_app::AppSink>().unwrap();sink.set_max_buffers(8);sink.set_sync(false);
        pipeline.add_many([&input,&demux,&parse,&decode,&convert,&caps,sink.upcast_ref()]).unwrap();input.link(&demux).unwrap();
        gst::Element::link_many([&parse,&decode,&convert,&caps,sink.upcast_ref()]).unwrap();
        let target=parse.static_pad("sink").unwrap();demux.connect_pad_added(move|_,pad|{
            if pad.current_caps().is_some_and(|caps|caps.structure(0).is_some_and(|s|s.name()=="audio/mpeg")){pad.link(&target).unwrap();}
        });
        // An independent native demux reads actual STTS/EDTS into compressed
        // packet timestamps and a TIME segment. No decoded count is used to
        // choose expectations, and a missing packet is rejected separately.
        use std::sync::{Arc,Mutex};
        let timing=Arc::new(Mutex::new((Vec::<(u64,u64)>::new(),None::<(u64,u64)>)));
        let observe=timing.clone();parse.static_pad("sink").unwrap().add_probe(gst::PadProbeType::BUFFER|gst::PadProbeType::EVENT_DOWNSTREAM,move|_,probe|{
            let mut data=observe.lock().unwrap();
            if let Some(buffer)=probe.buffer(){assert!(data.0.len()<128);data.0.push((buffer.pts().unwrap().nseconds(),buffer.duration().unwrap().nseconds()));}
            if let Some(event)=probe.event(){if let gst::EventView::Segment(segment)=event.view(){
                let segment=segment.segment().downcast_ref::<gst::ClockTime>().unwrap();
                data.1=Some((segment.start().unwrap().nseconds(),segment.stop().unwrap().nseconds()));}}
            gst::PadProbeReturn::Ok
        });
        pipeline.set_state(gst::State::Playing).unwrap();let mut decoded=Vec::new();
        while !sink.is_eos(){assert!(Instant::now()<expires,"AAC decode deadline");
            if let Some(sample)=sink.try_pull_sample(gst::ClockTime::from_mseconds(20)){
                let buffer=sample.buffer().unwrap();let map=buffer.map_readable().unwrap();
                println!("DECODED_AAC pts={:?} duration={:?} bytes={} cumulative={}",buffer.pts(),buffer.duration(),map.len(),decoded.len());
                for bytes in map.chunks_exact(4){decoded.push(i16::from_le_bytes([bytes[0],bytes[1]]));}
            }
            if let Some(message)=pipeline.bus().unwrap().pop_filtered(&[gst::MessageType::Error]){
                if let gst::MessageView::Error(error)=message.view(){panic!("AAC decoder {}",error.error());}
            }
        }
        let (packets,segment)=timing.lock().unwrap().clone();
        let (segment_start,segment_stop)=segment.unwrap();
        assert_eq!(segment_start,actual_delay*1_000_000_000/48000,"actual parsed EDTS start");
        assert_eq!(segment_stop,segment_start+expected_samples as u64*1_000_000_000/48000,"actual parsed EDTS start plus independently scaled edit duration");
        assert_eq!(frame_samples,1024,"fixed AAC LC frame contract");
        assert_eq!(packets.len(),(input_samples as u64+actual_delay).div_ceil(frame_samples)as usize,"all native encoded AAC packets must reach decoder");
        // Compressed last-packet duration can be shortened by the container.
        // AAC LC still decodes a full frame. GstAudioDecoder derives raw
        // duration from decoded sample count (gstaudiodecoder.c 1.24.2,
        // finish_frame lines 1422-1432), then gst_audio_buffer_clip floors
        // nanoseconds back to samples. With its default zero tolerance the
        // parsed PTS is the timestamp basis; never infer natural PCM length
        // from the demuxer's shortened compressed duration or decoded.len().
        let natural_duration=frame_samples*1_000_000_000/48000;
        let quantized_expected:usize=packets.iter().map(|&(pts,_compressed_duration)|{
            let stop=pts+natural_duration;
            if stop<=segment_start||pts>=segment_stop{return 0;}
            let left=((segment_start.saturating_sub(pts))as u128*48000/1_000_000_000)as usize;
            let right=((stop.saturating_sub(segment_stop))as u128*48000/1_000_000_000)as usize;
            frame_samples as usize-left-right
        }).sum();
        println!("AAC parsed last packet={:?} naturalDecodedDuration={natural_duration} segment=({segment_start},{segment_stop})",packets.last());
        let target=if quantized{quantized_expected}else{expected_samples};
        assert_eq!(decoded.len(),target,"exact independent decoded PCM sample-lattice prediction");
        for range in [0..4800,4800..expected_samples-4800,expected_samples-4800..expected_samples]{
            let (mut xy,mut xx,mut yy)=(0.,0.,0.);
            for (x,y) in expected[range.clone()].iter().zip(&decoded[range]){let(x,y)=(*x as f64,*y as f64);xy+=x*y;xx+=x*x;yy+=y*y;}
            assert!(xy/(xx*yy).sqrt()>0.94,"PCM leading/body/tail correlation");
        }
        pipeline.set_state(gst::State::Null).unwrap();
        println!("AAC native requestedPCM={expected_samples} actualInputPCM={input_samples} parsedPackets={} predictedConsumerPCM={quantized_expected} decodedPCM={} nativeDelay={actual_delay} head/body/tail independently correlated=true",packets.len(),decoded.len());
    }
}
