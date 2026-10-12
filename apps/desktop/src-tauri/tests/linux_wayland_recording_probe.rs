// SPDX-License-Identifier: MPL-2.0
//! Explicit manual native probe. No production capability is admitted here.
#![cfg(target_os = "linux")]
#[path="../src/recording_portal_linux.rs"] mod recording_portal_linux;
#[path="../src/recording_pipewire_linux.rs"] mod recording_pipewire_linux;
#[path="../src/recording_system_audio_linux.rs"] mod recording_system_audio_linux;
#[path="../src/recording_codec_linux.rs"] mod recording_codec_linux;
#[path="../src/recording_aac_linux.rs"]mod recording_aac_linux;
#[path="../src/recording_mp4_audio_linux.rs"]mod recording_mp4_audio_linux;
use std::{fs::{File,OpenOptions}, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf, time::{Duration,Instant}};
fn run() -> Result<(),String> {
    let output=PathBuf::from(std::env::args_os().nth(1).ok_or("缺少测试输出目录")?);
    if !output.is_absolute() || !output.is_dir() { return Err("测试目录必须已经存在".into()); }
    let expires=Instant::now()+Duration::from_secs(90);
    let session=recording_portal_linux::PortalSession::open(expires,&mut ||true)?;
    let frames=recording_pipewire_linux::Frames::connect(&session)?;
    let first=loop { if Instant::now()>=expires{return Err("没有收到屏幕帧".into());}
        if let Some(frame)=frames.next(Duration::from_millis(20))? {break frame;} };
    image::save_buffer_with_format(output.join("screen.png"),&first.rgba,first.width,first.height,image::ColorType::Rgba8,image::ImageFormat::Png).map_err(|_|"无法保存测试图片")?;
    // Explicit whole approved source only. No old Screenshot bitmap rectangle is
    // guessed to correspond to this independently selected monitor.
    let width=first.width&!1; let height=first.height&!1;
    if width!=first.width || height!=first.height || u64::from(width)*u64::from(height)>4_000_000 {return Err("测试源尺寸超出编码预算".into());}
    let file=OpenOptions::new().create_new(true).read(true).write(true).mode(0o600).open(output.join("screen.mp4")).map_err(|_|"无法创建测试视频")?;
    let mut codec=recording_codec_linux::Codec::new(file,width,height,expires)?;
    let audio=recording_system_audio_linux::SystemAudio::connect()?;
    let mut pcm=OpenOptions::new().create_new(true).write(true).mode(0o600).open(output.join("system.s16le")).map_err(|_|"无法创建测试声音")?;
    let mut audio_bytes=0usize; let mut count=0u64; let start=Instant::now(); let mut next=start;
    while count<45 {
        if Instant::now()>=expires{return Err("测试录屏超时".into());}
        for _ in 0..8 {let Some(chunk)=audio.poll()? else {break;};audio_bytes+=chunk.len();
            if audio_bytes>48_000*4*10{return Err("测试声音超出预算".into());} pcm.write_all(&chunk).map_err(|_|"无法写入测试声音")?;}
        if Instant::now()>=next {if let Some(frame)=frames.next(Duration::from_millis(10))? {
            if frame.width!=width || frame.height!=height {return Err("测试屏幕尺寸已经改变".into());}
            codec.push_rgba(frame.rgba,&||Instant::now()<expires)?;count+=1;next=start+Duration::from_nanos(count*1_000_000_000/15);
        }} else {std::thread::sleep(Duration::from_millis(2));}
    }
    let encoded=codec.finish(&||Instant::now()<expires)?;pcm.sync_all().map_err(|_|"无法结束测试声音")?;
    drop(audio);drop(frames);drop(session);
    let receipt=serde_json::json!({"userApprovedMonitor":true,"width":width,"height":height,"frames":encoded,"systemPcmFrames":audio_bytes/4,"sampleRate":48000,"channels":2,"sessionClosed":true,"productionEnabled":false,"aacValidated":false});
    let mut receipt_file=File::create(output.join("receipt.json")).map_err(|_|"无法保存测试回执")?;
    serde_json::to_writer_pretty(&mut receipt_file,&receipt).map_err(|_|"无法保存测试回执")?;
    println!("{receipt}");Ok(())
}
fn main(){if let Err(error)=run(){eprintln!("{error}");std::process::exit(1);}}
