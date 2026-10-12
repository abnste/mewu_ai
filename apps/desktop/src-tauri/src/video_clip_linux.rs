// SPDX-License-Identifier: MPL-2.0
//! Local descriptor-bound MP4 work in the sealed, lifetime-owned video child.
//! Native demux/decode never accepts renderer paths or a remote media URI.
use super::check_cancel;
use crate::video_clip_contract::{self,ClipError,ClipJob,ClipJobResult,VideoMetadata,VideoRange,OverlayRaster,GifMetadata,MAX_SOURCE_BYTES};
use crate::video_annotation_render::{RasterPlan,SourcePlaybackTicks};
use super::unix_output::Destination;
use gstreamer as gst;
use gst::prelude::*;
use std::{fs::{File,OpenOptions},io::{Read,Seek,SeekFrom,Write},os::{fd::AsRawFd,unix::fs::{MetadataExt,OpenOptionsExt}},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::AtomicBool},time::{Duration,Instant}};

struct Source { file:File,path:PathBuf,dev:u64,ino:u64,len:u64,hash:[u8;32] }
fn digest(file:&mut File,cancel:&AtomicBool)->Result<[u8;32],ClipError>{
    use sha2::{Digest,Sha256};
    file.seek(SeekFrom::Start(0)).map_err(|_|ClipError::FileUnavailable)?;
    let mut hash=Sha256::new();let mut bytes=[0u8;64*1024];
    loop {check_cancel(cancel)?;let n=file.read(&mut bytes).map_err(|_|ClipError::FileUnavailable)?;if n==0{break;}hash.update(&bytes[..n]);}
    file.seek(SeekFrom::Start(0)).map_err(|_|ClipError::FileUnavailable)?;
    Ok(hash.finalize().into())
}
impl Source {
    fn open(path:&Path,cancel:&AtomicBool)->Result<Self,ClipError>{
        check_cancel(cancel)?;
        if !path.is_absolute(){return Err(ClipError::UnsupportedPath);}
        let entry=std::fs::symlink_metadata(path).map_err(|_|ClipError::FileUnavailable)?;
        if !entry.is_file()||entry.file_type().is_symlink(){return Err(ClipError::InvalidMedia);}
        let path=path.canonicalize().map_err(|_|ClipError::FileUnavailable)?;
        let mut file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(&path).map_err(|_|ClipError::FileUnavailable)?;
        let meta=file.metadata().map_err(|_|ClipError::FileUnavailable)?;
        if !meta.is_file()||!(12..=MAX_SOURCE_BYTES).contains(&meta.len()){return Err(ClipError::InvalidMedia);}
        let mut magic=[0u8;12];file.read_exact(&mut magic).map_err(|_|ClipError::InvalidMedia)?;
        if &magic[4..8]!=b"ftyp"{return Err(ClipError::InvalidMedia);}
        let hash=digest(&mut file,cancel)?;
        let mut source=Self{file,path,dev:meta.dev(),ino:meta.ino(),len:meta.len(),hash};source.check(cancel)?;Ok(source)
    }
    fn check(&mut self,cancel:&AtomicBool)->Result<(),ClipError>{
        check_cancel(cancel)?;
        let entry=std::fs::symlink_metadata(&self.path).map_err(|_|ClipError::SourceChanged)?;
        let opened=self.file.metadata().map_err(|_|ClipError::SourceChanged)?;
        if !entry.is_file()||entry.file_type().is_symlink()||[entry.dev(),opened.dev()]!=[self.dev,self.dev]
            ||[entry.ino(),opened.ino()]!=[self.ino,self.ino]||[entry.len(),opened.len()]!=[self.len,self.len]
            ||digest(&mut self.file,cancel)?!=self.hash{return Err(ClipError::SourceChanged);}
        Ok(())
    }
}
fn element(name:&str)->Result<gst::Element,ClipError>{gst::ElementFactory::make(name).build().map_err(|_|ClipError::Unsupported)}
struct Pipeline(gst::Pipeline);
impl Drop for Pipeline {fn drop(&mut self){let _=self.0.set_state(gst::State::Null);}}
#[derive(Default)]struct Streams{video:u32,audio:u32,invalid:bool}
fn inspect(source:&mut Source,cancel:&AtomicBool)->Result<VideoMetadata,ClipError>{
    gst::init().map_err(|_|ClipError::WorkerUnavailable)?;
    let pipeline=Pipeline(gst::Pipeline::new());
    let fd=element("fdsrc")?;fd.set_property("fd",source.file.as_raw_fd());
    let demux=element("qtdemux")?;let video_queue=element("queue")?;let audio_queue=element("queue")?;
    for queue in [&video_queue,&audio_queue]{queue.set_property("max-size-bytes",4*1024*1024u32);queue.set_property("max-size-buffers",0u32);queue.set_property("max-size-time",0u64);}
    let decoder=element("decodebin")?;
    let sink=element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_|ClipError::WorkerUnavailable)?;
    sink.set_max_buffers(1);sink.set_drop(true);sink.set_sync(false);sink.set_property("enable-last-sample",false);
    let audio=element("fakesink")?;audio.set_property("sync",false);audio.set_property("async",false);
    pipeline.0.add_many([&fd,&demux,&video_queue,&audio_queue,&decoder,sink.upcast_ref(),&audio]).map_err(|_|ClipError::WorkerUnavailable)?;
    fd.link(&demux).map_err(|_|ClipError::WorkerUnavailable)?;video_queue.link(&decoder).map_err(|_|ClipError::WorkerUnavailable)?;
    audio_queue.link(&audio).map_err(|_|ClipError::WorkerUnavailable)?;
    let streams=Arc::new(Mutex::new(Streams::default()));let state=streams.clone();
    let video_pad=video_queue.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;
    let audio_pad=audio_queue.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;
    demux.connect_pad_added(move|_,pad|{
        let mut state=state.lock().unwrap_or_else(|e|e.into_inner());
        let caps=pad.current_caps().unwrap_or_else(||pad.query_caps(None));
        let Some(kind)=caps.structure(0).map(|s|s.name().as_str())else{state.invalid=true;return;};
        let target=if kind.starts_with("video/"){state.video+=1;if state.video!=1{state.invalid=true;return;}&video_pad}
            else if kind.starts_with("audio/"){state.audio+=1;if state.audio!=1{state.invalid=true;return;}&audio_pad}
            else {state.invalid=true;return;};
        if pad.link(target).is_err(){state.invalid=true;}
    });
    let state=streams.clone();let sink_pad=sink.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;
    decoder.connect_pad_added(move|_,pad|{
        let caps=pad.current_caps().unwrap_or_else(||pad.query_caps(None));
        if caps.structure(0).is_none_or(|s|s.name().as_str()!="video/x-raw")||sink_pad.is_linked()||pad.link(&sink_pad).is_err(){state.lock().unwrap_or_else(|e|e.into_inner()).invalid=true;}
    });
    pipeline.0.set_state(gst::State::Paused).map_err(|_|ClipError::InvalidMedia)?;
    let bus=pipeline.0.bus().ok_or(ClipError::WorkerUnavailable)?;let until=Instant::now()+Duration::from_secs(10);
    let sample=loop{
        check_cancel(cancel)?;
        if streams.lock().map_err(|_|ClipError::InvalidMedia)?.invalid{return Err(ClipError::InvalidMedia);}
        for _ in 0..32{let Some(message)=bus.pop()else{break;};if matches!(message.view(),gst::MessageView::Error(_)|gst::MessageView::Eos(_)){return Err(ClipError::InvalidMedia);}}
        if let Some(sample)=sink.try_pull_preroll(gst::ClockTime::from_mseconds(20)){break sample;}
        if Instant::now()>=until{return Err(ClipError::TimedOut);}
    };
    let info=gstreamer_video::VideoInfo::from_caps(sample.caps().ok_or(ClipError::InvalidMedia)?).map_err(|_|ClipError::InvalidMedia)?;
    let duration=pipeline.0.query_duration::<gst::ClockTime>().ok_or(ClipError::InvalidMedia)?;
    let state=streams.lock().map_err(|_|ClipError::InvalidMedia)?;
    if state.invalid||state.video!=1||state.audio>1{return Err(ClipError::InvalidMedia);}
    let fps=info.fps();
    let metadata=VideoMetadata{duration_ticks:duration.nseconds()/100,width:info.width(),height:info.height(),
        frame_rate_numerator:u32::try_from(fps.numer()).map_err(|_|ClipError::InvalidMedia)?,frame_rate_denominator:u32::try_from(fps.denom()).map_err(|_|ClipError::InvalidMedia)?,has_audio:state.audio==1};
    metadata.validate()?;drop(state);drop(sample);drop(pipeline);source.check(cancel)?;Ok(metadata)
}
fn check(cancel:&AtomicBool,until:Instant)->Result<(),ClipError>{
    check_cancel(cancel)?;if Instant::now()>=until{Err(ClipError::TimedOut)}else{Ok(())}
}
fn bus_errors(pipeline:&gst::Pipeline)->Result<(),ClipError>{
    let bus=pipeline.bus().ok_or(ClipError::WorkerUnavailable)?;
    for _ in 0..32{let Some(message)=bus.pop()else{break;};if let gst::MessageView::Error(_)=message.view(){return Err(ClipError::InvalidMedia);}}
    Ok(())
}
fn add_chain(pipeline:&gst::Pipeline,nodes:&[&gst::Element])->Result<(),ClipError>{
    pipeline.add_many(nodes.iter().copied()).map_err(|_|ClipError::WorkerUnavailable)?;
    gst::Element::link_many(nodes.iter().copied()).map_err(|_|ClipError::WorkerUnavailable)
}
fn capsfilter(caps:gst::Caps)->Result<gst::Element,ClipError>{let filter=element("capsfilter")?;filter.set_property("caps",caps);Ok(filter)}
fn decode_link(decoder:&gst::Element,sink:&gst::Element,kind:&'static str,state:Arc<Mutex<Streams>>)->Result<(),ClipError>{
    let target=sink.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;
    decoder.connect_pad_added(move|_,pad|{
        let caps=pad.current_caps().unwrap_or_else(||pad.query_caps(None));
        if caps.structure(0).is_none_or(|s|s.name().as_str()!=kind)||target.is_linked()||pad.link(&target).is_err(){state.lock().unwrap_or_else(|e|e.into_inner()).invalid=true;}
    });Ok(())
}
/// Bounded native decoder. A descriptor, never a URI, is the only source.
struct Reader{pipeline:Pipeline,video:gstreamer_app::AppSink,audio:Option<gstreamer_app::AppSink>,streams:Arc<Mutex<Streams>>}
impl Reader{
    fn new(source:&mut Source,audio_enabled:bool)->Result<Self,ClipError>{
        source.file.seek(SeekFrom::Start(0)).map_err(|_|ClipError::FileUnavailable)?;
        let pipeline=Pipeline(gst::Pipeline::new());let fd=element("fdsrc")?;fd.set_property("fd",source.file.as_raw_fd());
        let demux=element("qtdemux")?;add_chain(&pipeline.0,&[&fd,&demux])?;
        let streams=Arc::new(Mutex::new(Streams::default()));
        let video_queue=element("queue")?;let audio_queue=element("queue")?;
        // Encoded MP4 chunks can precede the other track's first packet.
        // A two-packet queue deadlocks preroll before that other sink exists.
        // Bound bytes, retaining room for the existing mux's interleaved chunks.
        for queue in [&video_queue,&audio_queue]{queue.set_property("max-size-bytes",4*1024*1024u32);queue.set_property("max-size-buffers",0u32);queue.set_property("max-size-time",0u64);}
        let decoder=element("decodebin")?;let convert=element("videoconvert")?;
        let raw=capsfilter(gst::Caps::builder("video/x-raw").field("format","RGBA").build())?;
        let video=element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_|ClipError::WorkerUnavailable)?;
        video.set_sync(false);video.set_max_buffers(2);video.set_drop(false);video.set_property("enable-last-sample",false);
        add_chain(&pipeline.0,&[&video_queue,&decoder])?;add_chain(&pipeline.0,&[&convert,&raw,video.upcast_ref()])?;
        decode_link(&decoder,&convert,"video/x-raw",streams.clone())?;
        let audio=if audio_enabled{
            let decoder=element("decodebin")?;let convert=element("audioconvert")?;let resample=element("audioresample")?;
            let raw=capsfilter(gst::Caps::builder("audio/x-raw").field("format","S16LE").field("layout","interleaved").field("rate",48000i32).field("channels",2i32).build())?;
            let sink=element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_|ClipError::WorkerUnavailable)?;
            sink.set_sync(false);sink.set_max_buffers(4);sink.set_drop(false);sink.set_property("enable-last-sample",false);
            add_chain(&pipeline.0,&[&audio_queue,&decoder])?;add_chain(&pipeline.0,&[&convert,&resample,&raw,sink.upcast_ref()])?;
            decode_link(&decoder,&convert,"audio/x-raw",streams.clone())?;Some(sink)
        }else{
            let sink=element("fakesink")?;sink.set_property("sync",false);sink.set_property("async",false);
            add_chain(&pipeline.0,&[&audio_queue,&sink])?;None
        };
        let video_pad=video_queue.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;let audio_pad=audio_queue.static_pad("sink").ok_or(ClipError::WorkerUnavailable)?;
        let state=streams.clone();demux.connect_pad_added(move|_,pad|{
            let caps=pad.current_caps().unwrap_or_else(||pad.query_caps(None));let mut state=state.lock().unwrap_or_else(|e|e.into_inner());
            let Some(kind)=caps.structure(0).map(|s|s.name().as_str())else{state.invalid=true;return;};
            let target=if kind.starts_with("video/"){state.video+=1;if state.video!=1{state.invalid=true;return;}&video_pad}
                else if kind.starts_with("audio/"){state.audio+=1;if state.audio!=1{state.invalid=true;return;}&audio_pad}else{state.invalid=true;return;};
            if pad.link(target).is_err(){state.invalid=true;}
        });
        pipeline.0.set_state(gst::State::Paused).map_err(|_|ClipError::InvalidMedia)?;
        Ok(Self{pipeline,video,audio,streams})
    }
    fn ready(&self,cancel:&AtomicBool,until:Instant)->Result<(),ClipError>{
        loop{self.check(cancel,until)?;let (result,state,pending)=self.pipeline.0.state(gst::ClockTime::from_mseconds(20));
            result.map_err(|_|ClipError::InvalidMedia)?;if state==gst::State::Paused&&pending==gst::State::VoidPending{return Ok(());}}
    }
    fn check(&self,cancel:&AtomicBool,until:Instant)->Result<(),ClipError>{
        check(cancel,until)?;bus_errors(&self.pipeline.0)?;if self.streams.lock().map_err(|_|ClipError::InvalidMedia)?.invalid{Err(ClipError::InvalidMedia)}else{Ok(())}
    }
    fn start(&self,start:u64,end:Option<u64>,key:bool)->Result<(),ClipError>{
        let flags=gst::SeekFlags::FLUSH|if key{gst::SeekFlags::KEY_UNIT}else{gst::SeekFlags::ACCURATE};
        // Send one seek through the actual video leg to the common demuxer.
        // An absent audio track's unlinked fakesink must not veto that seek.
        self.video.seek(1.,flags,gst::SeekType::Set,gst::ClockTime::from_nseconds(start*100),
            if end.is_some(){gst::SeekType::Set}else{gst::SeekType::None},end.map(|e|gst::ClockTime::from_nseconds(e*100))).map_err(|_|ClipError::InvalidRange)?;
        self.pipeline.0.set_state(gst::State::Playing).map_err(|_|ClipError::InvalidMedia)?;Ok(())
    }
    fn next(&self,cancel:&AtomicBool,until:Instant)->Result<Option<gst::Sample>,ClipError>{
        loop{self.check(cancel,until)?;if let Some(sample)=self.video.try_pull_sample(gst::ClockTime::from_mseconds(20)){return Ok(Some(sample));}
            if self.video.is_eos(){return Ok(None);}}
    }
}
fn sample_time(sample:&gst::Sample)->Result<(u64,u64),ClipError>{
    let buffer=sample.buffer().ok_or(ClipError::InvalidMedia)?;
    let pts=buffer.pts().ok_or(ClipError::InvalidMedia)?.nseconds()/100;
    let duration=buffer.duration().ok_or(ClipError::InvalidMedia)?.nseconds()/100;
    if duration==0{Err(ClipError::InvalidMedia)}else{Ok((pts,duration))}
}
fn rgba(sample:&gst::Sample,expected:&VideoMetadata)->Result<image::RgbaImage,ClipError>{
    let info=gstreamer_video::VideoInfo::from_caps(sample.caps().ok_or(ClipError::InvalidMedia)?).map_err(|_|ClipError::InvalidMedia)?;
    if (info.width(),info.height())!=(expected.width,expected.height)||info.format()!=gstreamer_video::VideoFormat::Rgba{return Err(ClipError::SourceChanged);}
    let frame=gstreamer_video::VideoFrameRef::from_buffer_ref_readable(sample.buffer().ok_or(ClipError::InvalidMedia)?,&info).map_err(|_|ClipError::InvalidMedia)?;
    let plane=frame.plane_data(0).map_err(|_|ClipError::InvalidMedia)?;let stride=usize::try_from(frame.plane_stride()[0]).map_err(|_|ClipError::InvalidMedia)?;
    let row=expected.width as usize*4;let mut bytes=Vec::with_capacity(row*expected.height as usize);
    for y in 0..expected.height as usize{bytes.extend_from_slice(plane.get(y*stride..y*stride+row).ok_or(ClipError::InvalidMedia)?);}
    image::RgbaImage::from_raw(expected.width,expected.height,bytes).ok_or(ClipError::InvalidMedia)
}
fn for_frames(source:&mut Source,expected:&VideoMetadata,requested:&[u64],cancel:&AtomicBool,until:Instant,
    mut apply:impl FnMut(usize,&gst::Sample,Option<u64>)->Result<(),ClipError>)->Result<(),ClipError>{
    if requested.is_empty()||requested.windows(2).any(|w|w[0]>=w[1])||requested.last().is_some_and(|t|*t>=expected.duration_ticks){return Err(ClipError::InvalidRange);}
    let reader=Reader::new(source,false)?;reader.ready(cancel,until)?;reader.start(requested[0],None,true)?;
    let mut current=reader.next(cancel,until)?.ok_or(ClipError::InvalidMedia)?;let mut next=reader.next(cancel,until)?;
    for (index,&tick)in requested.iter().enumerate(){
        while next.as_ref().is_some_and(|s|sample_time(s).is_ok_and(|(pts,_)|pts<=tick)){
            current=next.take().unwrap();next=reader.next(cancel,until)?;
        }
        let (pts,duration)=sample_time(&current)?;let next_pts=next.as_ref().map(sample_time).transpose()?.map(|t|t.0);
        if pts>tick||next_pts.is_none()&&tick>=pts.saturating_add(duration){return Err(ClipError::InvalidRange);}
        check(cancel,until)?;apply(index,&current,next_pts)?;
    }
    drop(reader);source.check(cancel)
}
struct Limited<'a,W>{inner:W,written:u64,limit:u64,cancel:&'a AtomicBool}
impl<W:Write> Write for Limited<'_,W>{
    fn write(&mut self,bytes:&[u8])->std::io::Result<usize>{check_cancel(self.cancel).map_err(|_|std::io::Error::other("cancelled"))?;
        if bytes.len()as u64>self.limit.saturating_sub(self.written){return Err(std::io::Error::other("media budget"));}
        let n=self.inner.write(bytes)?;self.written+=n as u64;Ok(n)}
    fn flush(&mut self)->std::io::Result<()>{self.inner.flush()}
}
fn gif_render(source:&mut Source,output:&Path,expected:&VideoMetadata,range:Option<&VideoRange>,overlays:&[OverlayRaster],cancel:&AtomicBool,progress:&dyn Fn(u8))->Result<GifMetadata,ClipError>{
    let until=Instant::now()+Duration::from_secs(600);expected.validate_range(range)?;
    if inspect(source,cancel)?!=*expected{return Err(ClipError::SourceChanged);}
    let start=range.map_or(0,|r|r.start_ticks);let end=range.map_or(expected.duration_ticks,|r|r.end_ticks);
    let (metadata,requested,delays,plan)=if overlays.is_empty(){
        let m=GifMetadata::for_source(expected,range)?;
        let requested=(0..m.frame_count).map(|i|start+(u64::from(i)*(end-start)/u64::from(m.frame_count)).min(end-start-1)).collect::<Vec<_>>();
        let delays=(0..m.frame_count).map(|i|crate::video_gif_backend::frame_delay(i,&m)).collect();(m,requested,delays,None)
    }else{let schedule=video_clip_contract::AnnotatedGifSchedule::new(expected,range,overlays)?;
        (schedule.metadata,schedule.requested_ticks,schedule.delays,Some(RasterPlan::load(expected,range,overlays,cancel)?))};
    let mut destination=Destination::gif(output)?;let file=destination.create_file()?;
    let limited=Limited{inner:std::io::BufWriter::new(file.try_clone().map_err(|_|ClipError::FileUnavailable)?),written:0,limit:video_clip_contract::MAX_GIF_BYTES,cancel};
    let mut encoder=gif::Encoder::new(limited,metadata.width as u16,metadata.height as u16,&[]).map_err(|_|ClipError::EncodingFailed)?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(|_|ClipError::EncodingFailed)?;progress(0);let mut previous=0;
    for_frames(source,expected,&requested,cancel,until,|index,sample,_|{
        destination.check()?;let mut pixels=rgba(sample,expected)?;if let Some(plan)=&plan{plan.paint(&mut pixels,SourcePlaybackTicks(requested[index]),cancel)?;}
        let mut pixels=image::imageops::resize(&pixels,metadata.width,metadata.height,image::imageops::FilterType::Triangle).into_raw();
        let mut frame=gif::Frame::from_rgba_speed(metadata.width as u16,metadata.height as u16,&mut pixels,10);frame.delay=delays[index];frame.dispose=gif::DisposalMethod::Background;
        encoder.write_frame(&frame).map_err(|_|if check_cancel(cancel).is_err(){ClipError::Cancelled}else{ClipError::EncodingFailed})?;
        let percent=((index+1)*99/requested.len())as u8;if percent>previous{previous=percent;progress(percent);}Ok(())
    })?;
    let mut writer=encoder.into_inner().map_err(|_|ClipError::EncodingFailed)?;writer.flush().map_err(|_|ClipError::FileUnavailable)?;drop(writer);file.sync_all().map_err(|_|ClipError::FileUnavailable)?;
    if plan.is_some(){crate::video_gif_backend::validate_annotated_file(destination.path(),&metadata,&delays,cancel)?;}
    else{crate::video_gif_backend::validate_file(destination.path(),&metadata,cancel)?;}
    check(cancel,until)?;source.check(cancel)?;destination.publish(&file)?;check(cancel,until)?;source.check(cancel)?;destination.commit();progress(100);Ok(metadata)
}
struct Encoder{pipeline:Pipeline,video:Option<gstreamer_app::AppSrc>,audio:Option<gstreamer_app::AppSrc>}
impl Encoder{
    fn new(file:&File,expected:&VideoMetadata,mp3:bool)->Result<Self,ClipError>{
        let pipeline=Pipeline(gst::Pipeline::new());let sink=element("fdsink")?;sink.set_property("fd",file.as_raw_fd());sink.set_property("sync",false);
        let mux=if mp3{None}else{let mux=element("mp4mux")?;mux.set_property("movie-timescale",48000u32);add_chain(&pipeline.0,&[&mux,&sink])?;Some(mux)};
        let video=if !mp3{
            let source=element("appsrc")?.downcast::<gstreamer_app::AppSrc>().map_err(|_|ClipError::WorkerUnavailable)?;
            source.set_caps(Some(&gst::Caps::builder("video/x-raw").field("format","RGBA").field("width",expected.width as i32).field("height",expected.height as i32)
                .field("framerate",gst::Fraction::new(expected.frame_rate_numerator as i32,expected.frame_rate_denominator as i32)).build()));
            source.set_format(gst::Format::Time);source.set_max_buffers(2);source.set_max_bytes(u64::from(expected.width)*u64::from(expected.height)*8);source.set_property("block",false);source.set_property("emit-signals",false);
            let convert=element("videoconvert")?;let raw=capsfilter(gst::Caps::builder("video/x-raw").field("format","I420").build())?;
            let encoder=element("openh264enc")?;encoder.set_property("multi-thread",1u32);encoder.set_property("bitrate",video_clip_contract::H264_SCREEN_BITRATE);
            let parser=element("h264parse")?;let encoded=capsfilter(gst::Caps::builder("video/x-h264").field("stream-format","avc").field("alignment","au").build())?;
            add_chain(&pipeline.0,&[source.upcast_ref(),&convert,&raw,&encoder,&parser,&encoded])?;encoded.link(mux.as_ref().unwrap()).map_err(|_|ClipError::WorkerUnavailable)?;Some(source)
        }else{None};
        let audio=if expected.has_audio{
            let source=element("appsrc")?.downcast::<gstreamer_app::AppSrc>().map_err(|_|ClipError::WorkerUnavailable)?;
            source.set_caps(Some(&gst::Caps::builder("audio/x-raw").field("format","S16LE").field("layout","interleaved").field("rate",48000i32).field("channels",2i32).build()));
            source.set_format(gst::Format::Time);source.set_max_buffers(16);source.set_max_bytes(262144);source.set_property("block",false);source.set_property("emit-signals",false);
            let convert=element("audioconvert")?;let encoder=element(if mp3{"lamemp3enc"}else{"avenc_aac"})?;
            if mp3{encoder.set_property_from_str("target","bitrate");encoder.set_property("bitrate",192i32);encoder.set_property("cbr",true);}
            let parser=element(if mp3{"mpegaudioparse"}else{"aacparse"})?;
            add_chain(&pipeline.0,&[source.upcast_ref(),&convert,&encoder,&parser])?;
            if let Some(mux)=&mux{parser.link(mux).map_err(|_|ClipError::WorkerUnavailable)?;}else{pipeline.0.add(&sink).map_err(|_|ClipError::WorkerUnavailable)?;parser.link(&sink).map_err(|_|ClipError::WorkerUnavailable)?;}Some(source)
        }else{if mp3{return Err(ClipError::NoAudio);}None};
        pipeline.0.set_state(gst::State::Playing).map_err(|_|ClipError::EncodingFailed)?;Ok(Self{pipeline,video,audio})
    }
    fn finish(&self,cancel:&AtomicBool,until:Instant)->Result<(),ClipError>{
        let bus=self.pipeline.0.bus().ok_or(ClipError::WorkerUnavailable)?;
        loop{check(cancel,until)?;if let Some(message)=bus.timed_pop(gst::ClockTime::from_mseconds(20)){
            match message.view(){gst::MessageView::Eos(_)=>break,gst::MessageView::Error(_)=>return Err(ClipError::EncodingFailed),_=>{}}
        }}
        self.pipeline.0.set_state(gst::State::Null).map_err(|_|ClipError::EncodingFailed)?;
        let (result,state,_)=self.pipeline.0.state(gst::ClockTime::ZERO);result.map_err(|_|ClipError::EncodingFailed)?;
        if state!=gst::State::Null{return Err(ClipError::EncodingFailed);}Ok(())
    }
}
fn push(source:&gstreamer_app::AppSrc,bytes:Vec<u8>,pts:u64,duration:u64)->Result<(),ClipError>{
    let mut buffer=gst::Buffer::from_mut_slice(bytes);let writable=buffer.get_mut().ok_or(ClipError::EncodingFailed)?;
    writable.set_pts(gst::ClockTime::from_nseconds(pts*100));writable.set_duration(gst::ClockTime::from_nseconds(duration*100));
    source.push_buffer(buffer).map_err(|_|ClipError::EncodingFailed)?;Ok(())
}
fn frame_tick(index:u64,numerator:u32,denominator:u32)->Result<u64,ClipError>{
    if numerator==0||denominator==0{return Err(ClipError::InvalidMedia);}
    // A legal reduced fraction can have an almost-i32::MAX denominator.
    // Its intermediate product exceeds u64 even for a 30-minute clip,
    // though the final source time still fits the existing duration budget.
    (u128::from(index)*10_000_000*u128::from(denominator)/u128::from(numerator))
        .try_into().map_err(|_|ClipError::InvalidMedia)
}
/// Independently decode every exported MPEG audio packet through the curated
/// native MP3 decoder. Only the exact output inode can be opened here.
fn decode_mp3(path:&Path,owned:&File,metadata:&crate::mp3_file::Metadata,cancel:&AtomicBool,until:Instant)->Result<(u64,f64),ClipError>{
    let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW|libc::O_CLOEXEC).open(path).map_err(|_|ClipError::OutputInvalid)?;
    let actual=file.metadata().map_err(|_|ClipError::OutputInvalid)?;let expected=owned.metadata().map_err(|_|ClipError::OutputInvalid)?;
    if !actual.is_file()||(actual.dev(),actual.ino(),actual.len())!=(expected.dev(),expected.ino(),expected.len()){return Err(ClipError::OutputInvalid);}
    let pipeline=Pipeline(gst::Pipeline::new());let fd=element("fdsrc")?;fd.set_property("fd",file.as_raw_fd());
    let parser=element("mpegaudioparse")?;let decoder=element("avdec_mp3")?;let convert=element("audioconvert")?;
    let raw=capsfilter(gst::Caps::builder("audio/x-raw").field("format","S16LE").field("layout","interleaved")
        .field("rate",metadata.sample_rate as i32).field("channels",i32::from(metadata.channels)).build())?;
    let sink=element("appsink")?.downcast::<gstreamer_app::AppSink>().map_err(|_|ClipError::WorkerUnavailable)?;
    sink.set_sync(false);sink.set_max_buffers(2);sink.set_drop(false);sink.set_property("enable-last-sample",false);
    add_chain(&pipeline.0,&[&fd,&parser,&decoder,&convert,&raw,sink.upcast_ref()])?;
    pipeline.0.set_state(gst::State::Playing).map_err(|_|ClipError::OutputInvalid)?;let mut samples=0u64;let mut squares=0u64;
    loop{
        check(cancel,until)?;bus_errors(&pipeline.0)?;
        if let Some(sample)=sink.try_pull_sample(gst::ClockTime::from_mseconds(20)){
            let buffer=sample.buffer().ok_or(ClipError::OutputInvalid)?;let map=buffer.map_readable().map_err(|_|ClipError::OutputInvalid)?;
            if map.is_empty()||map.len()%(usize::from(metadata.channels)*2)!=0{return Err(ClipError::OutputInvalid);}
            for bytes in map.chunks_exact(2){let value=i64::from(i16::from_le_bytes([bytes[0],bytes[1]]));squares=squares.checked_add((value*value)as u64).ok_or(ClipError::OutputInvalid)?;}
            samples+=map.len()as u64/2;
            if samples>u64::from(metadata.sample_rate)*u64::from(metadata.channels)*1801{return Err(ClipError::OutputInvalid);}
        }else if sink.is_eos(){break;}
    }
    drop(pipeline);
    if samples==0{return Err(ClipError::OutputInvalid);}
    let duration=samples*10_000_000/u64::from(metadata.channels)/u64::from(metadata.sample_rate);
    if duration.abs_diff(metadata.duration_ticks)>video_clip_contract::MIN_RANGE_TICKS{return Err(ClipError::OutputInvalid);}
    Ok((duration,(squares as f64/samples as f64).sqrt()))
}
fn transcode(source:&mut Source,output:&Path,expected:&VideoMetadata,range:Option<&VideoRange>,overlays:&[OverlayRaster],mp3:bool,cancel:&AtomicBool,progress:&dyn Fn(u8))->Result<ClipJobResult,ClipError>{
    let until=Instant::now()+Duration::from_secs(600);expected.validate_range(range)?;
    if mp3&&!expected.has_audio{return Err(ClipError::NoAudio);}
    if inspect(source,cancel)?!=*expected{return Err(ClipError::SourceChanged);}
    let plan=if overlays.is_empty(){None}else{Some(RasterPlan::load(expected,range,overlays,cancel)?)};
    let start=range.map_or(0,|r|r.start_ticks);let end=range.map_or(expected.duration_ticks,|r|r.end_ticks);
    let reader=Reader::new(source,expected.has_audio)?;reader.ready(cancel,until.min(Instant::now()+Duration::from_secs(10)))?;reader.start(start,Some(end),false)?;
    let mut destination=Destination::with_extension(output,if mp3{"mp3"}else{"mp4"})?;let file=destination.create_file()?;
    let encoder=Encoder::new(&file,expected,mp3)?;let mut video_end=false;let mut audio_end=!expected.has_audio;let mut previous=0;let mut video_count=0u64;let mut audio_count=0u64;let mut pending=None;progress(0);
    while !video_end||!audio_end{
        reader.check(cancel,until)?;bus_errors(&encoder.pipeline.0)?;destination.check()?;
        let limit=if mp3{crate::mp3_file::MAX_BYTES}else{MAX_SOURCE_BYTES};
        if file.metadata().map_err(|_|ClipError::FileUnavailable)?.len()>limit{return Err(ClipError::OutputInvalid);}
        let mut fed=false;
        if !video_end&&encoder.video.as_ref().is_none_or(|s|s.current_level_buffers()<2){
            if pending.is_none(){pending=reader.video.try_pull_sample(gst::ClockTime::ZERO);}
            if let Some(sample)=pending.as_ref(){
                fed=true;let (pts,duration)=sample_time(sample)?;let right=pts.saturating_add(duration).min(end);
                if let Some(target)=&encoder.video{
                    // Decode the real covering sample at an explicit CFR clock.
                    // Clipping the first/last compressed sample durations would
                    // silently change MP4's reported rate (31 frames / 2s).
                    let position=frame_tick(video_count,expected.frame_rate_numerator,expected.frame_rate_denominator)?;
                    let next_position=frame_tick(video_count.checked_add(1).ok_or(ClipError::InvalidMedia)?,expected.frame_rate_numerator,expected.frame_rate_denominator)?;
                    let tick=start.checked_add(position).ok_or(ClipError::InvalidMedia)?;
                    let next_tick=start.checked_add(next_position).ok_or(ClipError::InvalidMedia)?;
                    if tick.saturating_add(2)<pts{return Err(ClipError::InvalidRange);}
                    if tick<end&&tick<right.saturating_add(2){
                        let mut pixels=rgba(sample,expected)?;if let Some(plan)=&plan{plan.paint(&mut pixels,SourcePlaybackTicks(tick),cancel)?;}
                        push(target,pixels.into_raw(),position,next_position-position)?;video_count+=1;
                    }
                    let pending_tick=if tick<end&&tick<right.saturating_add(2){next_tick}else{tick};
                    if pending_tick>=right.saturating_add(2)||pending_tick>=end{pending=None;}
                }else{pending=None;}
                let percent=((right.saturating_sub(start))*98/(end-start)).min(98)as u8;if percent>previous{previous=percent;progress(percent);}
            }else if reader.video.is_eos(){video_end=true;if let Some(s)=&encoder.video{s.end_of_stream().map_err(|_|ClipError::EncodingFailed)?;}}
        }
        if !audio_end&&encoder.audio.as_ref().is_some_and(|s|s.current_level_buffers()<16&&s.current_level_bytes()<262144){
            let sink=reader.audio.as_ref().ok_or(ClipError::InvalidMedia)?;
            if let Some(sample)=sink.try_pull_sample(gst::ClockTime::ZERO){
                fed=true;let (pts,_)=sample_time(&sample)?;let buffer=sample.buffer().ok_or(ClipError::InvalidMedia)?;
                let map=buffer.map_readable().map_err(|_|ClipError::InvalidMedia)?;if map.len()%4!=0{return Err(ClipError::InvalidMedia);}
                let frames=map.len()/4;
                let trim_start=if pts<start{((start-pts)*48000).div_ceil(10_000_000)as usize}else{0};
                let trim_end=if end>pts{((end-pts)*48000/10_000_000)as usize}else{0};
                let left=trim_start.min(frames);let right=trim_end.min(frames);
                if right>left{let actual=pts+left as u64*10_000_000/48000;
                    push(encoder.audio.as_ref().unwrap(),map[left*4..right*4].to_vec(),actual.saturating_sub(start),(right-left)as u64*10_000_000/48000)?;audio_count+=(right-left)as u64;}
            }else if sink.is_eos(){audio_end=true;encoder.audio.as_ref().unwrap().end_of_stream().map_err(|_|ClipError::EncodingFailed)?;}
        }
        if !fed{std::thread::sleep(Duration::from_millis(1));}
    }
    if !mp3&&video_count==0||expected.has_audio&&audio_count==0{return Err(ClipError::InvalidMedia);}
    drop(reader);encoder.finish(cancel,until)?;drop(encoder);file.sync_all().map_err(|_|ClipError::FileUnavailable)?;
    source.check(cancel)?;destination.check()?;
    let result=if mp3{let metadata=crate::mp3_file::inspect(destination.path(),cancel)?;metadata.validate_for(expected,range)?;
        decode_mp3(destination.path(),&file,&metadata,cancel,until)?;ClipJobResult::Mp3Rendered(metadata)}
    else{
        let mut rendered=Source::open(destination.path(),cancel)?;let metadata=inspect(&mut rendered,cancel)?;
        #[cfg(test)]eprintln!("native-transcode expected={expected:?} range={range:?} actual={metadata:?}");
        let retained=VideoRange{start_ticks:start,end_ticks:end};
        if plan.is_some(){video_clip_contract::validate_annotated_rendered(expected,&retained,&metadata)?;}
        else{video_clip_contract::validate_rendered(expected,&retained,&metadata)?;}ClipJobResult::Rendered(metadata)
    };
    check(cancel,until)?;source.check(cancel)?;destination.publish(&file)?;check(cancel,until)?;source.check(cancel)?;destination.commit();progress(100);Ok(result)
}
fn extract(source:&mut Source,directory:&Path,expected:&VideoMetadata,window:&VideoRange,requested:&[u64],cancel:&AtomicBool)->Result<ClipJobResult,ClipError>{
    use sha2::{Digest,Sha256};
    video_clip_contract::validate_extraction_request(expected,window,requested)?;
    if inspect(source,cancel)?!=*expected{return Err(ClipError::SourceChanged);}
    if std::fs::read_dir(directory).map_err(|_|ClipError::FileUnavailable)?.next().is_some(){return Err(ClipError::FileUnavailable);}
    let until=Instant::now()+Duration::from_secs(30);let (width,height)=video_clip_contract::extracted_dimensions(expected);let mut frames=Vec::new();let mut destinations=Vec::new();let mut total=0;
    for_frames(source,expected,requested,cancel,until,|index,sample,_|{
        let pixels=image::imageops::resize(&rgba(sample,expected)?,width,height,image::imageops::FilterType::Triangle);
        let rgb=image::DynamicImage::ImageRgba8(pixels).into_rgb8();
        let mut bytes=Limited{inner:Vec::new(),written:0,limit:video_clip_contract::MAX_FRAME_JPEG_BYTES,cancel};
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes,95).encode_image(&rgb).map_err(|_|ClipError::OutputInvalid)?;
        total+=bytes.written;if total>video_clip_contract::MAX_FRAME_TOTAL_BYTES{return Err(ClipError::OutputInvalid);}
        let (pts,duration)=sample_time(sample)?;let path=directory.join(format!("{index}.jpg"));let mut destination=Destination::with_extension(&path,"jpg")?;let mut file=destination.create_file()?;
        file.write_all(&bytes.inner).and_then(|_|file.sync_all()).map_err(|_|ClipError::FileUnavailable)?;destination.publish(&file)?;destinations.push(destination);
        frames.push(video_clip_contract::FrameDescriptor{index:index as u32,requested_ticks:requested[index],actual_pts_ticks:pts,sample_duration_ticks:duration,
            encoded_width:width,encoded_height:height,jpeg_sha256:format!("{:x}",Sha256::digest(&bytes.inner)),bytes_length:bytes.written});Ok(())
    })?;
    video_clip_contract::validate_frame_descriptors(expected,window,requested,&frames)?;
    // The parent decodes final JPEGs after this child and its staging guards
    // have exited; staging directories remain private until commit here.
    check(cancel,until)?;source.check(cancel)?;for d in &mut destinations{d.check()?;}
    for d in &mut destinations{d.commit();}Ok(ClipJobResult::FramesExtracted{frames})
}
pub(super) fn execute(job:&ClipJob,cancel:&AtomicBool,on_progress:&dyn Fn(u8))->Result<ClipJobResult,ClipError>{
    match job {
        ClipJob::Inspect{source}=>{let mut source=Source::open(source,cancel)?;Ok(ClipJobResult::Inspected(inspect(&mut source,cancel)?))},
        ClipJob::RenderGif{source,output,expected,range}=>gif_render(&mut Source::open(source,cancel)?,output,expected,range.as_ref(),&[],cancel,on_progress).map(ClipJobResult::GifRendered),
        ClipJob::RenderGifAnnotated{source,output,expected,range,overlays}=>gif_render(&mut Source::open(source,cancel)?,output,expected,range.as_ref(),overlays,cancel,on_progress).map(ClipJobResult::GifRendered),
        ClipJob::Render{source,output,expected,range}=>transcode(&mut Source::open(source,cancel)?,output,expected,Some(range),&[],false,cancel,on_progress),
        ClipJob::RenderAnnotated{source,output,expected,range,overlays}=>transcode(&mut Source::open(source,cancel)?,output,expected,range.as_ref(),overlays,false,cancel,on_progress),
        ClipJob::RenderMp3{source,output,expected,range}=>transcode(&mut Source::open(source,cancel)?,output,expected,range.as_ref(),&[],true,cancel,on_progress),
        ClipJob::ExtractFrames{source,output_directory,expected,window,requested_ticks}=>extract(&mut Source::open(source,cancel)?,output_directory,expected,window,requested_ticks,cancel)
    }
}

#[cfg(test)]mod tests{
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]fn large_reduced_frame_rate_keeps_exact_time_without_overflow(){
        // Near1fps, already reduced, and inside the admitted source duration.
        // Golden values are exact rational long division, not float rounding.
        assert_eq!(frame_tick(1800,2_147_483_647,2_147_483_646),Ok(17_999_999_991));
        assert_eq!(frame_tick(1801,2_147_483_647,2_147_483_646),Ok(18_009_999_991));
        assert_eq!(frame_tick(1800,15,1),Ok(1_200_000_000));
        assert_eq!(frame_tick(0,0,1),Err(ClipError::InvalidMedia));
        assert_eq!(frame_tick(1,1,0),Err(ClipError::InvalidMedia));
        assert_eq!(frame_tick(u64::MAX,1,u32::MAX),Err(ClipError::InvalidMedia));
    }
    fn fixture(audio:bool)->(PathBuf,PathBuf){
        let root=std::env::temp_dir().join(format!("mewu-linux-video-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let path=root.join("source.mp4");let file=OpenOptions::new().write(true).read(true).create_new(true).mode(0o600).open(&path).unwrap();
        let mut codec=crate::recording_codec_linux::Codec::with_audio(file,160,96,Instant::now()+Duration::from_secs(60),audio).unwrap();
        for i in 0..60{
            let mut bytes=vec![0u8;160*96*4];for pixel in bytes.chunks_exact_mut(4){pixel.copy_from_slice(&[i*3,40,80,255]);}
            codec.push_rgba(bytes,&||true).unwrap();
            if audio{let mut pcm=Vec::with_capacity(3200*4);for frame in 0..3200{
                let time=(usize::from(i)*3200+frame)as f64/48000.;let value=(12000.*(std::f64::consts::TAU*700.*time).sin())as i16;
                pcm.extend_from_slice(&value.to_le_bytes());pcm.extend_from_slice(&value.to_le_bytes());}
                codec.push_pcm(&pcm,&||true).unwrap();}
        }
        assert_eq!(codec.finish(&||true).unwrap(),60);(root,path)
    }
    #[test]fn actual_linux_video_metadata_trim_gif_frames_and_annotations(){
        let (root,path)=fixture(true);let cancel=AtomicBool::new(false);let mut source=Source::open(&path,&cancel).unwrap();let hash=source.hash;
        let expected=inspect(&mut source,&cancel).unwrap();assert_eq!((expected.width,expected.height),(160,96));assert!(expected.has_audio);assert!(expected.duration_ticks.abs_diff(40_000_000)<500_000);
        let range=VideoRange{start_ticks:12_345_678,end_ticks:32_345_678};let output=root.join("trim.mp4");
        let events=std::cell::RefCell::new(Vec::new());let result=execute(&ClipJob::Render{source:path.clone(),output:output.clone(),expected:expected.clone(),range:range.clone()},&cancel,&|p|events.borrow_mut().push(p)).unwrap();
        let ClipJobResult::Rendered(actual)=result else{panic!("native rendered output");};video_clip_contract::validate_rendered(&expected,&range,&actual).unwrap();assert_eq!(events.borrow().last(),Some(&100));assert!(events.borrow().windows(2).all(|p|p[0]<p[1]));
        assert_eq!(execute(&ClipJob::Render{source:path.clone(),output:output.clone(),expected:expected.clone(),range:range.clone()},&cancel,&|_|{}),Err(ClipError::FileUnavailable));
        let gif=root.join("output.gif");let result=execute(&ClipJob::RenderGif{source:path.clone(),output:gif.clone(),expected:expected.clone(),range:Some(range.clone())},&cancel,&|_|{}).unwrap();
        let ClipJobResult::GifRendered(meta)=result else{panic!("real GIF");};crate::video_gif_backend::validate_file(&gif,&meta,&cancel).unwrap();
        let frames=root.join("frames");std::fs::create_dir(&frames).unwrap();let requested=vec![range.start_ticks,range.start_ticks+1_000_000,range.end_ticks-1];
        let result=execute(&ClipJob::ExtractFrames{source:path.clone(),output_directory:frames.clone(),expected:expected.clone(),window:range.clone(),requested_ticks:requested.clone()},&cancel,&|_|{}).unwrap();
        let ClipJobResult::FramesExtracted{frames:descriptors}=result else{panic!("decoded frames");};assert_eq!(descriptors.len(),3);
        crate::video_annotation_render::validate_extracted_files(&frames,&expected,&range,&requested,&descriptors,&cancel).unwrap();
        use sha2::{Digest,Sha256};let overlay=root.join("overlay.png");image::RgbaImage::from_pixel(8,8,image::Rgba([255,0,0,255])).save(&overlay).unwrap();
        let overlays=vec![OverlayRaster{png:overlay.clone(),png_sha256:format!("{:x}",Sha256::digest(std::fs::read(&overlay).unwrap())),width:8,height:8,
            bounds:video_clip_contract::OverlayBounds{x:0.,y:0.,width:32.,height:32.},interval:VideoRange{start_ticks:15_000_000,end_ticks:25_000_000}}];
        let annotated=root.join("annotated.mp4");let ClipJobResult::Rendered(annotated_meta)=execute(&ClipJob::RenderAnnotated{source:path.clone(),output:annotated.clone(),expected:expected.clone(),range:Some(range.clone()),overlays:overlays.clone()},&cancel,&|_|{}).unwrap()else{panic!("native annotated video");};
        let mut annotated_source=Source::open(&annotated,&cancel).unwrap();let mut colors=Vec::new();
        for_frames(&mut annotated_source,&annotated_meta,&[1_000_000,5_000_000,15_000_000],&cancel,Instant::now()+Duration::from_secs(10),|_,sample,_|{colors.push(*rgba(sample,&annotated_meta)?.get_pixel(4,4));Ok(())}).unwrap();
        assert!(colors[1][0]>220&&colors[1][1]<40&&colors[1][2]<40,"annotation actually paints middle source interval");
        assert!(colors[0][2]>45&&colors[2][2]>45,"outside interval retains original media pixels");
        let annotated_gif=root.join("annotated.gif");assert!(matches!(execute(&ClipJob::RenderGifAnnotated{source:path.clone(),output:annotated_gif.clone(),expected:expected.clone(),range:Some(range.clone()),overlays},&cancel,&|_|{}).unwrap(),ClipJobResult::GifRendered(_)));
        assert_eq!(Source::open(&path,&cancel).unwrap().hash,hash);
        println!("Linux native metadata/trim/AAC/GIF/extracted JPEG/annotated MP4/GIF all decoded; real interval pixels; source unchanged");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]fn actual_linux_source_and_cancel_keep_output_private_and_unchanged(){
        let (root,path)=fixture(false);let cancel=AtomicBool::new(false);let mut source=Source::open(&path,&cancel).unwrap();let metadata=inspect(&mut source,&cancel).unwrap();assert!(!metadata.has_audio);
        let output=root.join("cancel.mp4");let job=ClipJob::Render{source:path.clone(),output:output.clone(),expected:metadata,range:VideoRange{start_ticks:5_000_000,end_ticks:25_000_000}};
        assert_eq!(execute(&job,&cancel,&|p|if p==0{cancel.store(true,Ordering::Release)}),Err(ClipError::Cancelled));assert!(!output.exists());assert_eq!(std::fs::read_dir(&root).unwrap().count(),1);
        cancel.store(false,Ordering::Release);let replacement=root.join("replacement.mp4");std::fs::copy(&path,&replacement).unwrap();std::fs::rename(&replacement,&path).unwrap();assert_eq!(source.check(&cancel),Err(ClipError::SourceChanged));
        let link=root.join("link.mp4");std::os::unix::fs::symlink(&path,&link).unwrap();assert_eq!(Source::open(&link,&cancel).err(),Some(ClipError::InvalidMedia));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]fn actual_linux_mp3_pcm_decode_and_retained_audio_duration(){
        let (root,path)=fixture(true);let cancel=AtomicBool::new(false);let mut source=Source::open(&path,&cancel).unwrap();let expected=inspect(&mut source,&cancel).unwrap();
        let range=VideoRange{start_ticks:12_345_678,end_ticks:32_345_678};let output=root.join("sound.mp3");
        let ClipJobResult::Mp3Rendered(metadata)=execute(&ClipJob::RenderMp3{source:path,output:output.clone(),expected:expected.clone(),range:Some(range.clone())},&cancel,&|_|{}).unwrap()else{panic!("real MP3");};
        metadata.validate_for(&expected,Some(&range)).unwrap();assert_eq!(metadata.channels,2);assert_eq!(metadata.sample_rate,48000);
        let file=File::open(&output).unwrap();let (duration,rms)=decode_mp3(&output,&file,&metadata,&cancel,Instant::now()+Duration::from_secs(10)).unwrap();
        assert!(duration.abs_diff(20_000_000)<1_000_000);assert!((1000.0..20000.0).contains(&rms),"actual retained sine PCM survives AAC→MP3");
        println!("independent native MP3 decoded PCM duration={duration} rms={rms:.2}");
        println!("native MP3 framing and duration {metadata:?}");std::fs::remove_dir_all(root).unwrap();
    }
}
