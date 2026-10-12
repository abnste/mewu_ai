// SPDX-License-Identifier: MPL-2.0
//! Two-phase recording over one user-approved portal session. The preview and
//! subsequent crop belong to this exact session, never a previous screenshot.
use crate::{recording_backend::{AudioMode, Progress, Report}, recording_codec_linux::Codec,
    recording_pipewire_linux::{Frame, Frames}, recording_portal_linux::PortalSession,
    recording_codec_linux::FinalizeStage};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::{BufRead, BufReader, Write}, os::unix::fs::OpenOptionsExt,
    path::PathBuf, sync::mpsc, time::{Duration, Instant}};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct PrepareConfig { pub nonce: String, pub preview: PathBuf }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Prepared { pub nonce: String, pub width: u32, pub height: u32 }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase", deny_unknown_fields)]
pub(crate) struct Selection { pub nonce: String, pub output: PathBuf, pub x:u32, pub y:u32,
    pub width:u32, pub height:u32, pub audio:AudioMode }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case", deny_unknown_fields)]
pub(crate) enum Control { Start(Selection), Pause{}, Resume{}, Stop{}, Cancel{} }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case", deny_unknown_fields)]
pub(crate) enum Event { Prepared(Prepared), Progress(Progress), Completed(Report),
    Finalizing{stage:FinalizeStage,frames:u64,pcm_frames:u64},
    Canceled{}, Failed{error:String} }
const LINE:usize=16*1024;
fn read(reader:&mut impl BufRead)->Result<Option<String>,String> {
    use std::io::Read;
    let mut line=String::new();
    let count=reader.take((LINE+1) as u64).read_line(&mut line).map_err(|_|"录屏连接读取失败")?;
    if count==0{return Ok(None);}
    if count>LINE || !line.ends_with('\n'){return Err("无效录屏通信".into());}
    Ok(Some(line))
}
fn write(writer:&mut impl Write, value:&impl Serialize)->Result<(),String> {
    let mut bytes=serde_json::to_vec(value).map_err(|_|"无效录屏通信")?;
    if bytes.len()>=LINE{return Err("无效录屏通信".into());}
    bytes.push(b'\n');writer.write_all(&bytes).and_then(|_|writer.flush()).map_err(|_|"录屏连接已断开".into())
}
fn valid_nonce(value:&str)->bool { uuid::Uuid::parse_str(value).is_ok_and(|id|id.hyphenated().to_string()==value) }
fn selection_valid(selection:&Selection, prepared:&Prepared)->bool {
    selection.nonce==prepared.nonce && selection.output.is_absolute()
        && selection.output.extension().is_some_and(|v|v=="mp4")
        && selection.width>=2 && selection.height>=2 && selection.width%2==0 && selection.height%2==0
        && u64::from(selection.width)*u64::from(selection.height)<=4_000_000
        && selection.x.checked_add(selection.width).is_some_and(|v|v<=prepared.width)
        && selection.y.checked_add(selection.height).is_some_and(|v|v<=prepared.height)
}
fn crop(frame:&Frame, selection:&Selection, prepared:&Prepared)->Result<Vec<u8>,String> {
    if (frame.width,frame.height)!=(prepared.width,prepared.height) || !selection_valid(selection,prepared){
        return Err("屏幕共享尺寸或录制选区已变化".into());
    }
    let stride=frame.width as usize*4;
    if frame.rgba.len()!=stride*frame.height as usize{return Err("屏幕帧长度无效".into());}
    let row=selection.width as usize*4;let x=selection.x as usize*4;
    let mut output=Vec::with_capacity(row*selection.height as usize);
    for y in selection.y as usize..(selection.y+selection.height) as usize {
        output.extend_from_slice(&frame.rgba[y*stride+x..y*stride+x+row]);
    }
    Ok(output)
}
#[derive(Default)]
struct PendingPcm { bytes:Vec<u8>, offset:usize }
impl PendingPcm { fn clear(&mut self){self.bytes.clear();self.offset=0;} }
fn feed_audio_to(codec:&mut Codec,pending:&mut PendingPcm,target:u64,
    mut poll:impl FnMut()->Result<Option<Vec<u8>>,String>,allowed:&impl Fn()->bool)->Result<(),String>{
    while codec.pcm_frames()<target {
        if pending.offset==pending.bytes.len(){
            let Some(bytes)=poll()?else{break};
            if bytes.is_empty()||bytes.len()%4!=0||bytes.len()>96_000{return Err("电脑声音帧长度无效".into());}
            pending.bytes=bytes;pending.offset=0;
        }
        let remaining=(target-codec.pcm_frames())as usize*4;
        let end=(pending.offset+remaining).min(pending.bytes.len());
        codec.push_pcm(&pending.bytes[pending.offset..end],allowed)?;pending.offset=end;
    }
    Ok(())
}
fn run(config:PrepareConfig, controls:&mpsc::Receiver<Control>, mut emit:impl FnMut(Event)->Result<(),String>)
    ->Result<Option<Report>,String> {
    if !valid_nonce(&config.nonce) || !config.preview.is_absolute()
        || config.preview.file_stem().and_then(|s|s.to_str())!=Some(&config.nonce)
        || config.preview.extension().is_none_or(|v|v!="png") {return Err("录屏预览凭据无效".into());}
    let consent_deadline=Instant::now()+Duration::from_secs(90);
    let mut canceled=false;
    let session=PortalSession::open(consent_deadline,&mut ||{
        match controls.try_recv(){Ok(Control::Cancel{}|Control::Stop{})|Err(mpsc::TryRecvError::Disconnected)=>canceled=true,
            Ok(_)=>canceled=true, Err(mpsc::TryRecvError::Empty)=>{}}
        !canceled
    });
    if canceled{return Ok(None);}
    let session=session?;let frames=Frames::connect(&session)?;
    let first=loop {
        if Instant::now()>=consent_deadline{return Err("等待屏幕共享画面超时".into());}
        match controls.try_recv(){Ok(Control::Cancel{}|Control::Stop{})|Err(mpsc::TryRecvError::Disconnected)=>return Ok(None),
            Ok(_)=>return Err("录屏控制顺序无效".into()),Err(mpsc::TryRecvError::Empty)=>{}}
        if let Some(frame)=frames.next(Duration::from_millis(20))?{break frame;}
    };
    let prepared=Prepared{nonce:config.nonce.clone(),width:first.width,height:first.height};
    let mut preview=OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&config.preview)
        .map_err(|_|"无法创建录屏预览")?;
    image::codecs::png::PngEncoder::new(&mut preview).write_image(&first.rgba,first.width,first.height,image::ExtendedColorType::Rgba8)
        .map_err(|_|"无法保存录屏预览")?;
    preview.sync_all().map_err(|_|"无法完成录屏预览")?;drop(preview);
    let mut latest=first;
    emit(Event::Prepared(prepared.clone()))?;
    let select_deadline=Instant::now()+Duration::from_secs(120);
    let selection=loop {
        if Instant::now()>=select_deadline{return Err("录屏选区等待超时".into());}
        match controls.recv_timeout(Duration::from_millis(20)) {
            Ok(Control::Start(value)) if selection_valid(&value,&prepared)=>break value,
            Ok(Control::Cancel{}|Control::Stop{})|Err(mpsc::RecvTimeoutError::Disconnected)=>return Ok(None),
            Ok(_)=>return Err("录屏选区凭据无效".into()),Err(mpsc::RecvTimeoutError::Timeout)=>{}}
        if let Some(frame)=frames.next(Duration::ZERO)?{latest=frame;}
    };
    if !matches!(selection.audio,AudioMode::Mute|AudioMode::System){return Err("此平台目前支持电脑声音或静音".into());}
    let expires=Instant::now()+Duration::from_secs(600);
    let file=OpenOptions::new().create_new(true).read(true).write(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(&selection.output).map_err(|_|"无法创建录屏文件")?;
    let audio=if selection.audio==AudioMode::System{Some(crate::recording_system_audio_linux::SystemAudio::connect()?)}else{None};
    let mut codec=Codec::with_audio(file,selection.width,selection.height,expires,audio.is_some())?;
    let mut paused=false;let mut count=0u64;let mut next=Instant::now();let mut heartbeat=next;
    let mut pending_pcm=PendingPcm::default();
    emit(Event::Progress(Progress{elapsed_ms:0,paused:false}))?;
    loop {
        match controls.try_recv(){Ok(Control::Stop{})=>break,Ok(Control::Cancel{})|Err(mpsc::TryRecvError::Disconnected)=>return Ok(None),
            Ok(Control::Pause{})=>paused=true,Ok(Control::Resume{})=>{paused=false;next=Instant::now();},
            Ok(Control::Start(_))=>return Err("录屏已经开始".into()),Err(mpsc::TryRecvError::Empty)=>{}}
        if Instant::now()>=expires || count>=9000{break;}
        if let Some(audio)=&audio {
            if paused {pending_pcm.clear();for _ in 0..8{if audio.poll()?.is_none(){break;}}}
            else {
                // Keep the two mux inputs within one video frame. Feeding an
                // entire Pulse burst first can deadlock bounded appsrc queues
                // before this same thread reaches its next video/Stop poll.
                feed_audio_to(&mut codec,&mut pending_pcm,(count+1)*3200,||audio.poll(),&||Instant::now()<expires)?;
            }
        }
        if let Some(frame)=frames.next(Duration::ZERO)?{latest=frame;}
        if paused{std::thread::sleep(Duration::from_millis(10));}
        else if Instant::now()>=next {
            codec.push_rgba(crop(&latest,&selection,&prepared)?,&||Instant::now()<expires)?;
            count+=1;next+=Duration::from_nanos(1_000_000_000/15);
            if next.elapsed()>Duration::from_millis(300){return Err("录屏编码积压超出限制".into());}
        } else {std::thread::sleep(Duration::from_millis(2));}
        if heartbeat.elapsed()>=Duration::from_millis(250){emit(Event::Progress(Progress{elapsed_ms:count*1000/15,paused}))?;heartbeat=Instant::now();}
    }
    if count==0{return Ok(None);}
    let final_deadline=(Instant::now()+Duration::from_secs(10)).min(expires);
    emit(Event::Finalizing{stage:FinalizeStage::AudioTail,frames:count,pcm_frames:codec.pcm_frames()})?;
    if let Some(audio)=&audio {let drain=(Instant::now()+Duration::from_secs(1)).min(final_deadline);
        while codec.pcm_frames()<count*3200 {
            if Instant::now()>=drain{return Err("电脑声音未覆盖录屏尾点".into());}
            if matches!(controls.try_recv(),Ok(Control::Cancel{})|Err(mpsc::TryRecvError::Disconnected)){return Ok(None);}
            feed_audio_to(&mut codec,&mut pending_pcm,count*3200,||audio.poll(),&||Instant::now()<drain)?;
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let pcm_frames=codec.pcm_frames();
    let encoded=codec.finish_with_progress(&||Instant::now()<final_deadline,
        |stage|emit(Event::Finalizing{stage,frames:count,pcm_frames}))?;
    emit(Event::Finalizing{stage:FinalizeStage::CloseCapture,frames:count,pcm_frames})?;
    drop(frames);drop(session);
    Ok(Some(Report{width:selection.width,height:selection.height,duration_ms:encoded*1000/15,
        frames:encoded,audio:selection.audio,stop_reason:None}))
}
pub(crate) fn worker_main()->i32 {
    let mut input=BufReader::new(std::io::stdin());
    let config=match read(&mut input).and_then(|l|serde_json::from_str::<PrepareConfig>(&l.ok_or("缺少录屏配置")?).map_err(|_|"无效录屏配置".into())){
        Ok(v)=>v,Err(_)=>return 2};
    let (tx,rx)=mpsc::sync_channel(8);
    let thread=match std::thread::Builder::new().name("mewu-wayland-controls".into()).spawn(move||{
        while let Ok(Some(line))=read(&mut input){match serde_json::from_str::<Control>(&line){Ok(v)=>if tx.send(v).is_err(){return},Err(_)=>break}}
        let _=tx.send(Control::Cancel{});
    }){Ok(v)=>v,Err(_)=>return 2};
    let mut output=std::io::stdout();
    let result=run(config,&rx,|event|write(&mut output,&event));
    let (event,code)=match result{Ok(Some(v))=>(Event::Completed(v),0),Ok(None)=>(Event::Canceled{},0),Err(error)=>(Event::Failed{error},1)};
    let sent=write(&mut output,&event).is_ok();let _=thread.join();if sent{code}else{1}
}

use image::ImageEncoder;

/// Admission uses the same frozen ELF, bubblewrap PID1, parent-death bridge and
/// tracked pipe lifetimes as other production Linux workers. No arbitrary CLI.
pub(crate) fn supervise(config:PrepareConfig,
    mut control:impl FnMut()->Result<Option<Control>,String>,
    mut prepared:impl FnMut(Prepared)->Result<(),String>,
    mut progress:impl FnMut(Progress), mut finalized:impl FnMut()) ->Result<Option<Report>,String> {
    use crate::owned_process::{ManagedProcess,linux_worker::{TrustedCommand,Worker}};
    use std::{process::Stdio, sync::atomic::AtomicBool};
    let permit=crate::recording_worker::Permit::acquire()?;
    let mut command=TrustedCommand::new(Worker::Recording).map_err(|_|"无法准备录屏进程")?;
    command.env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for name in ["HOME","XDG_RUNTIME_DIR","DBUS_SESSION_BUS_ADDRESS","PULSE_SERVER","PULSE_COOKIE"] {
        if let Some(value)=std::env::var_os(name){command.env(name,value);}
    }
    let started=Instant::now();
    let mut process=ManagedProcess::spawn_trusted_with_diagnostics(command,permit,started+Duration::from_secs(5),&AtomicBool::new(false))
        .map_err(|error|{eprintln!("mewu-recording-startup stage=admission kind={:?} errno={:?}",error.kind(),error.raw_os_error());
            "无法启动隔离录屏进程，请检查 bwrap 和系统授权配置".to_string()})?;
    let mut input=process.child_mut().stdin.take().ok_or("无法控制录屏进程")?;
    let output=process.child_mut().stdout.take().ok_or("无法读取录屏进程")?;
    let (tx,rx)=mpsc::sync_channel::<Control>(8);let (bad_tx,bad_rx)=mpsc::sync_channel(1);
    let wire=config.clone();
    process.track_io(std::thread::Builder::new().name("mewu-wayland-writer".into()).spawn(move||{
        let result=(||{write(&mut input,&wire)?;while let Ok(value)=rx.recv(){write(&mut input,&value)?;}Ok::<(),String>(())})();
        if result.is_err(){let _=bad_tx.try_send(());}
    }).map_err(|_|"无法建立录屏连接")?);
    enum Output{Event(Event),Eof,Invalid}
    let (event_tx,events)=mpsc::sync_channel(8);
    process.track_io(std::thread::Builder::new().name("mewu-wayland-reader".into()).spawn(move||{
        let mut input=BufReader::new(output);let mut total=0usize;
        loop {let output=match read(&mut input){Ok(Some(line))=>{total+=line.len();if total>4*1024*1024{Output::Invalid}
            else{serde_json::from_str(&line).map(Output::Event).unwrap_or(Output::Invalid)}},Ok(None)=>Output::Eof,Err(_)=>Output::Invalid};
            let ended=matches!(output,Output::Eof|Output::Invalid);if event_tx.send(output).is_err()||ended{break;}}
    }).map_err(|_|"无法读取录屏连接")?);
    let mut sender=Some(tx);let mut selection:Option<Selection>=None;let mut has_preview=false;
    let mut last=started;let mut stopped:Option<Instant>=None;let mut terminal=None;let mut eof=false;
    let mut canceled=false;let mut elapsed=0u64;
    let result=(||{
        loop {
            if terminal.is_none() && stopped.is_none(){if let Some(value)=control()? {
                match &value {
                    Control::Start(value)=>{if !has_preview||selection.is_some()||value.nonce!=config.nonce{return Err("录屏启动凭据无效".into());}
                        selection=Some(value.clone());last=Instant::now();},
                    Control::Cancel{}|Control::Stop{}=>{canceled=matches!(&value,Control::Cancel{});stopped=Some(Instant::now());},
                    _=>if selection.is_none(){return Err("录屏尚未开始".into());}
                }
                sender.as_ref().ok_or("录屏连接已断开")?.try_send(value).map_err(|_|"录屏控制未能送达")?;
            }}
            match events.recv_timeout(Duration::from_millis(20)) {
                Ok(Output::Event(event))=>{
                    if terminal.is_some()||eof{return Err("无效录屏通信顺序".into());}
                    match event {
                        Event::Prepared(value)=>{
                            if has_preview||selection.is_some()||value.nonce!=config.nonce||value.width<2||value.height<2
                                ||value.width>16384||value.height>16384||u64::from(value.width)*u64::from(value.height)>32_000_000{
                                return Err("录屏预览凭据无效".into());}
                            if stopped.is_none(){prepared(value)?;}has_preview=true;last=Instant::now();
                        },
                        Event::Progress(value)=>{if selection.is_none()||value.elapsed_ms<elapsed||value.elapsed_ms>600_100{return Err("无效录屏进度".into());}
                            elapsed=value.elapsed_ms;last=Instant::now();if stopped.is_none(){progress(value);}},
                        Event::Finalizing{stage,frames,pcm_frames}=>{
                            if selection.is_none()||frames==0||frames>9000||pcm_frames>48000*601{return Err("无效录屏终结进度".into());}
                            // Static stages/counters only: never print paths,
                            // nonce, screen pixels, PCM bytes or environment.
                            eprintln!("mewu-recording-finalize stage={stage:?} frames={frames} pcm={pcm_frames}");
                            if stopped.is_none(){stopped=Some(Instant::now());}
                        },
                        Event::Completed(report)=>{let value=selection.as_ref().ok_or("录屏尚未开始")?;
                            if report.width!=value.width||report.height!=value.height||report.audio!=value.audio
                                ||report.frames==0||report.frames>9000||report.duration_ms!=report.frames*1000/15{return Err("无效录屏结果".into());}
                            finalized();terminal=Some(Ok(Some(report)));sender.take();stopped=Some(Instant::now());},
                        Event::Canceled{}=>{if stopped.is_none(){return Err("无效录屏取消回执".into());}
                            terminal=Some(Ok(None));sender.take();stopped=Some(Instant::now());},
                        Event::Failed{error}=>{if error.is_empty()||error.len()>4096{return Err("无效录屏错误".into());}
                            terminal=Some(Err(error));sender.take();stopped=Some(Instant::now());}
                    }
                },
                Ok(Output::Eof)=>{eof=true;if terminal.is_none(){return Err("录屏进程已经中断".into());}},
                Ok(Output::Invalid)=>return Err("无效录屏通信".into()),
                Err(mpsc::RecvTimeoutError::Disconnected) if !eof=>return Err("录屏进程已经中断".into()),
                Err(mpsc::RecvTimeoutError::Disconnected)=>std::thread::sleep(Duration::from_millis(10)),
                Err(mpsc::RecvTimeoutError::Timeout)=>{}
            }
            if eof && process.fully_exited(){let status=process.try_wait().map_err(|_|"无法核对录屏退出")?.ok_or("录屏尚未退出")?;
                let value=terminal.take().ok_or("录屏未完成")?;if status.success()!=value.is_ok(){return Err("录屏进程异常退出".into());}
                return if canceled{Ok(None)}else{value};}
            if terminal.is_none()&&bad_rx.try_recv().is_ok(){return Err("录屏控制未能送达".into());}
            if (!has_preview&&started.elapsed()>Duration::from_secs(95))
                ||(has_preview&&selection.is_none()&&last.elapsed()>Duration::from_secs(125))
                ||(selection.is_some()&&stopped.is_none()&&last.elapsed()>Duration::from_secs(20))
                ||stopped.is_some_and(|v|v.elapsed()>Duration::from_secs(20))||started.elapsed()>Duration::from_secs(900){
                return Err("录屏响应超时，已停止".into());}
            if let Some(value)=&selection {if std::fs::metadata(&value.output).is_ok_and(|m|m.len()>512*1024*1024){return Err("录屏文件超出安全上限".into());}}
        }
    })();
    drop(sender);drop(events);
    if result.is_err(){process.kill();let until=Instant::now()+Duration::from_secs(2);
        while !process.fully_exited()&&Instant::now()<until{std::thread::sleep(Duration::from_millis(20));}}
    // Reaper keeps the permit when native IO or PID1 has not actually exited.
    drop(process);result
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn recording_wayland_real_mux_handles_long_pcm_bursts_with_bounded_lead(){
        use std::collections::VecDeque;
        let output=PathBuf::from(format!("/tmp/recording-burst-{}.mp4",uuid::Uuid::new_v4()));
        let file=OpenOptions::new().create_new(true).read(true).write(true).mode(0o600).open(&output).unwrap();
        let expires=Instant::now()+Duration::from_secs(12);
        let mut codec=Codec::with_audio(file,64,48,expires,true).unwrap();
        // A real native source is allowed to return half-second PCM blocks.
        // Feed them through the production split/budget logic into the actual
        // AAC/H264/mp4mux pipeline, rather than mocking appsrc or its queues.
        let mut bursts=VecDeque::new();
        for block in 0..64u64 {
            let bytes:Vec<_>=(block*24000..(block+1)*24000).flat_map(|sample|{
                let frequency=if sample>=480*3200-4800{880.}else{440.};
                let value=((sample as f64*frequency*std::f64::consts::TAU/48000.).sin()*10000.)as i16;
                [value.to_le_bytes(),value.to_le_bytes()].concat()
            }).collect();bursts.push_back(bytes);
        }
        let mut pending=PendingPcm::default();
        for index in 0..480u64 {
            feed_audio_to(&mut codec,&mut pending,(index+1)*3200,||Ok(bursts.pop_front()),&||Instant::now()<expires).unwrap();
            assert_eq!(codec.pcm_frames(),(index+1)*3200);
            assert!(pending.bytes.len()<=96_000);
            codec.push_rgba([20,120,220,255].repeat(64*48),&||Instant::now()<expires).unwrap();
        }
        assert!(bursts.is_empty());
        let mut stages=Vec::new();
        assert_eq!(codec.finish_with_progress(&||Instant::now()<expires,|stage|{stages.push(stage);Ok(())}).unwrap(),480);
        assert_eq!(stages,[FinalizeStage::VideoEos,FinalizeStage::AacTail,FinalizeStage::MuxEos,FinalizeStage::FileSync]);
        assert!(std::fs::metadata(&output).unwrap().len()>1000);
        std::fs::copy(&output,"/tmp/last-recording-burst-32s.mp4").unwrap();
        std::fs::remove_file(output).unwrap();
    }
    #[test]fn recording_wayland_nonce_crop_and_bounds_are_one_source(){
        let prepared=Prepared{nonce:uuid::Uuid::new_v4().to_string(),width:6,height:4};
        let mut selection=Selection{nonce:prepared.nonce.clone(),output:PathBuf::from("/tmp/test.mp4"),x:2,y:1,width:2,height:2,audio:AudioMode::Mute};
        let bytes:Vec<_>=(0..6*4*4).collect();
        assert_eq!(crop(&Frame{width:6,height:4,rgba:bytes.clone()},&selection,&prepared).unwrap(),[bytes[32..40].to_vec(),bytes[56..64].to_vec()].concat());
        selection.nonce=uuid::Uuid::new_v4().to_string();assert!(!selection_valid(&selection,&prepared));
        selection.nonce=prepared.nonce.clone();selection.x=u32::MAX;assert!(!selection_valid(&selection,&prepared));
        selection.x=5;assert!(!selection_valid(&selection,&prepared));
    }
    #[test]fn recording_wayland_control_wire_rejects_unknown_fields_and_unbounded_input(){
        assert!(serde_json::from_str::<Control>(r#"{"type":"pause","extra":1}"#).is_err());
        assert!(read(&mut &b"{}"[..]).is_err());assert!(read(&mut vec![b'x';LINE+1].as_slice()).is_err());
        let nonce=uuid::Uuid::new_v4().to_string();assert!(valid_nonce(&nonce));assert!(!valid_nonce("opaque-source"));
    }
}
