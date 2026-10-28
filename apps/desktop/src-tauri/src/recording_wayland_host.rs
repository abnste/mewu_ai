// SPDX-License-Identifier: MPL-2.0
//! A host-only source lease binds the portal's preview to its same live worker.
//! The renderer cannot submit a portal path, monitor ID or remembered permission.
use crate::{Host,HostSnapshot,recording,recording_backend as backend,recording_wayland_worker as worker};
use mewu_core::{Asset,AssetKind,Region,Snapshot};
use std::{sync::{Arc,Mutex,OnceLock},time::Duration};
use std::os::unix::fs::OpenOptionsExt;
use tauri::{AppHandle,Emitter,Manager,WebviewWindow};
#[derive(Default)]
struct Controls { start:Option<worker::Selection>, paused:Option<bool>, terminal:Option<bool>, terminal_delivered:bool }
impl Controls {
    fn take(&mut self)->Option<worker::Control> {
        if let Some(cancel)=self.terminal{if self.terminal_delivered{return None;}
            self.terminal_delivered=true;return Some(if cancel{worker::Control::Cancel{}}else{worker::Control::Stop{}});}
        if let Some(start)=self.start.take(){return Some(worker::Control::Start(start));}
        self.paused.take().map(|paused|if paused{worker::Control::Pause{}}else{worker::Control::Resume{}})
    }
}
struct Lease { nonce:String, scene_id:String, background:Option<Asset>, region:Option<Region>,
    transition:Arc<crate::SpaceTransition>,
    controls:Arc<Mutex<Controls>>, status:Option<recording::Status>, finalized:bool, suppress_reveal:bool,
    grant:Option<crate::plugins::RecordingGrant>, audio_grant:Option<crate::recording_audio_host::RecordingAudioGrant> }
impl Lease {
    fn countdown_transition(&self,nonce:&str)->Result<Arc<crate::SpaceTransition>,String>{
        let status=self.status.as_ref().ok_or("录屏尚未开始")?;
        let terminal=self.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some();
        if !tray_can_begin(&self.nonce,nonce,status.phase,terminal){return Err("录屏准备已结束".into());}
        Ok(self.transition.clone())
    }
}
#[derive(Default)] struct State { lease:Option<Lease>, finished:Option<(String,Result<(),String>)> }
fn state()->&'static Mutex<State> {static STATE:OnceLock<Mutex<State>>=OnceLock::new();STATE.get_or_init(||Mutex::new(State::default()))}
struct PreparingGuard { nonce:String, armed:bool }
struct StartingGuard { nonce:String, armed:bool }
impl Drop for StartingGuard {
    fn drop(&mut self){if self.armed{let _=control(&self.nonce,"cancel");}}
}
impl Drop for PreparingGuard {
    fn drop(&mut self){if self.armed{if let Ok(s)=state().lock(){
        if let Some(l)=s.lease.as_ref().filter(|l|l.nonce==self.nonce&&l.status.is_none()){
            let _=control_locked(l,true);
        }
    }}}
}
fn preparation_current(app:&AppHandle,nonce:&str)->Result<(),String>{
    let host=app.state::<Host>();host.exit.ensure_new_operation()?;
    let (scene_id,background)={let s=state().lock().map_err(|_|"录屏状态不可用")?;
        let l=s.lease.as_ref().filter(|l|l.nonce==nonce&&l.status.is_none()).ok_or("录屏准备已结束")?;
        if l.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some(){return Err("录屏准备已取消".into());}
        (l.scene_id.clone(),l.background.clone().ok_or("录屏预览未完成")?)
    };
    let engine=host.lock()?;let scene=engine.store.active_scene().map_err(|e|e.to_string())?;
    if scene.id!=scene_id||scene.closed||scene.frozen||scene.background.as_ref()!=Some(&background){
        return Err("录屏会话已经改变".into());
    }Ok(())
}
fn pending_preparation_canceled(app:&AppHandle,nonce:&str)->Result<bool,String>{
    let host=app.state::<Host>();let s=state().lock().map_err(|_|"录屏状态不可用")?;
    let Some(lease)=s.lease.as_ref().filter(|l|l.nonce==nonce&&l.status.is_none())else{return Ok(false)};
    // Reservation precedes stop_for_exit. Make its cancellation sticky for this
    // exact existing lease; a later aborted exit cannot revive its handshake.
    if !host.exit.accepts_new_operations(){control_locked(lease,true)?;}
    let canceled=lease.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some();Ok(canceled)
}
fn pending_source_current(app:&AppHandle,nonce:&str,source:&mewu_core::Scene)->Result<(),String>{
    let host=app.state::<Host>();
    {let s=state().lock().map_err(|_|"录屏状态不可用")?;
        let lease=s.lease.as_ref().filter(|l|l.nonce==nonce&&l.scene_id==source.id&&l.status.is_none()&&l.background.is_none()).ok_or("录屏准备已结束")?;
        if !host.exit.accepts_new_operations(){control_locked(lease,true)?;}
        if lease.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some(){return Err("录屏准备已取消".into())}
    }
    let engine=host.lock()?;let current=engine.store.active_scene().map_err(|e|e.to_string())?;
    if current.id!=source.id||current.updated_at!=source.updated_at||current.background!=source.background
        ||current.closed||current.frozen{return Err("录屏会话已经改变".into())}Ok(())
}
async fn restore_preparation_failure(app:&AppHandle,window:&WebviewWindow,nonce:&str,
    previous_scene:&str,previous_background:Option<Asset>,transition:Arc<crate::SpaceTransition>,fallback:String)->String{
    cancel_pending();
    eprintln!("mewu-recording-restore stage=waiting-for-worker");
    let until=std::time::Instant::now()+Duration::from_secs(25);
    let error=loop {
        let result=state().lock().ok().and_then(|s|s.finished.as_ref().filter(|(id,_)|id==nonce).map(|(_,r)|r.clone()));
        if let Some(result)=result.filter(|_|!crate::recording_worker::busy()){break result.err().unwrap_or(fallback);}
        if std::time::Instant::now()>=until{
            eprintln!("mewu-recording-restore stage=worker-timeout busy={}",crate::recording_worker::busy());
            return "录屏进程尚未退出，请稍后重试".into();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    eprintln!("mewu-recording-restore stage=worker-drained");
    let checked_app=app.clone();let checked_nonce=nonce.to_owned();let previous_scene=previous_scene.to_owned();
    let allowed:crate::capture::CaptureCheck=Arc::new(move||{
        let host=checked_app.state::<Host>();host.exit.ensure_new_operation()?;
        if crate::recording_worker::busy(){return Err("录屏进程尚未退出".into());}
        {let s=state().lock().map_err(|_|"录屏状态不可用")?;
            if s.lease.is_some()||!s.finished.as_ref().is_some_and(|(id,_)|id==&checked_nonce){return Err("录屏会话已变化".into());}}
        let engine=host.lock()?;let scene=engine.store.active_scene().map_err(|e|e.to_string())?;
        let unchanged=scene.id==previous_scene&&scene.background==previous_background;
        let adopted=scene.background.as_ref().is_some_and(|asset|asset.id==checked_nonce);
        if scene.closed||scene.frozen||(!unchanged&&!adopted){return Err("录屏会话已变化".into());}
        Ok(())
    });
    // Failure before consent can leave the original mapped canvas untouched.
    // Do not send it through the hidden-surface-only presentation path.
    match crate::capture_wayland_host::recording_surface_mapped(window,allowed.clone()).await{
        Ok(true)=>{let _=app.emit_to("space","host-error",&error);return error;},
        Ok(false)=>{},Err(_)=>return error,
    }
    match crate::capture_wayland_host::present_recording(window,allowed,transition).await{
        Ok(())=>{eprintln!("mewu-recording-restore stage=presented");let _=app.emit_to("space","host-error",&error);},
        Err(_)=>{eprintln!("mewu-recording-restore stage=presentation-rejected");},
    }
    error
}
pub(crate) fn selecting()->bool { state().lock().is_ok_and(|s|s.lease.as_ref().is_some_and(|l|
    l.status.is_none()&&l.controls.lock().is_ok_and(|c|c.terminal.is_none()))) }
pub(crate) fn has_lease()->bool { state().lock().is_ok_and(|s|s.lease.is_some()) }
pub(crate) fn status()->Result<Option<recording::Status>,String>{Ok(state().lock().map_err(|_|"录屏状态不可用")?.lease.as_ref().and_then(|l|l.status.clone()))}
pub(crate) fn completed_result(id:&str)->Result<Option<Result<(),String>>,String>{
    Ok(state().lock().map_err(|_|"录屏状态不可用")?.finished.as_ref()
        .filter(|(finished,_)|finished==id).map(|(_,result)|result.clone()))
}
fn emit(app:&AppHandle,value:&Option<recording::Status>){
    if value.is_none(){
        let target=app.clone();
        let _=app.run_on_main_thread(move||{
            use gtk::prelude::*;
            // A queued terminal event must not undo its successor's input mask.
            if status().is_ok_and(|value|value.is_none()){
                if let Some(window)=target.get_webview_window("space"){
                    if let Ok(widget)=window.gtk_window(){
                        widget.input_shape_combine_region(None);
                        widget.set_accept_focus(true);
                        widget.display().flush();
                    }
                }
            }
        });
    }
    crate::recording_tray_linux::refresh(app,value.as_ref().map(|status|status.phase));
    let _=app.emit_to("space","recording-status",value);
}
pub(crate) fn reconcile(snapshot:&Snapshot){
    if let Ok(s)=state().lock(){if let Some(l)=s.lease.as_ref().filter(|l|l.status.is_none()) {
        if l.background.as_ref().is_some_and(|background|!snapshot.scenes.iter().any(|scene|
            scene.id==l.scene_id&&!scene.closed&&!scene.frozen&&scene.background.as_ref()==Some(background)))
            ||l.background.is_some()&&snapshot.active_scene_id!=l.scene_id {
            if let Ok(mut c)=l.controls.lock(){c.terminal=Some(true);c.start=None;c.paused=None;}
        }
    }}
}
pub(crate) fn cancel_pending(){if let Ok(s)=state().lock(){if let Some(l)=s.lease.as_ref().filter(|l|l.status.is_none()){
    if let Ok(mut c)=l.controls.lock(){c.terminal=Some(true);c.start=None;c.paused=None;}
}}}
pub(crate) fn control(id:&str,action:&str)->Result<bool,String> {
    let s=state().lock().map_err(|_|"录屏状态不可用")?;
    let Some(l)=s.lease.as_ref().filter(|l|l.nonce==id)else{return Ok(false)};
    let mut c=l.controls.lock().map_err(|_|"录屏控制不可用")?;
    if c.terminal.is_some(){return Ok(true);}
    match action{"pause"=>c.paused=Some(true),"resume"=>c.paused=Some(false),"stop"=>{c.terminal=Some(false);c.paused=None;},
        "cancel"=>{c.terminal=Some(true);c.paused=None;},_=>return Err("无效录屏操作".into())}Ok(true)
}
pub(crate) fn request_stop(){if let Ok(s)=state().lock(){if let Some(l)=&s.lease{let _=control_locked(l,false);}}}
pub(crate) fn revoke_plugins(plugins:&crate::plugins::PluginSnapshot){
    if let Ok(s)=state().lock(){if let Some(l)=&s.lease{
        if l.grant.as_ref().is_some_and(|g|!crate::plugins::recording_grant_is_current(g,plugins))
            ||l.audio_grant.as_ref().is_some_and(|g|!crate::recording_audio_host::grant_is_current(g,plugins)){
            let _=control_locked(l,false);
        }
    }}
}
fn control_locked(l:&Lease,cancel:bool)->Result<(),String>{let mut c=l.controls.lock().map_err(|_|"录屏控制不可用")?;
    if c.terminal.is_none(){c.terminal=Some(cancel);c.start=None;c.paused=None;}Ok(())}
pub(crate) fn begin_exit<T>(begin:impl FnOnce()->Option<T>)->Result<Option<(T,Option<String>)>,String>{
    let mut s=state().lock().map_err(|_|"录屏状态不可用")?;let Some(value)=begin()else{return Ok(None)};
    let target=s.lease.as_mut().map(|l|{l.suppress_reveal=true;l.nonce.clone()});Ok(Some((value,target)))
}
pub(crate) async fn stop_for_exit(id:&str)->Result<bool,String>{
    {let s=state().lock().map_err(|_|"录屏状态不可用")?;
        if let Some(l)=s.lease.as_ref().filter(|l|l.nonce==id){control_locked(l,l.status.is_none())?;}
        else if !s.finished.as_ref().is_some_and(|(nonce,_)|nonce==id){return Ok(false);}}
    let until=std::time::Instant::now()+Duration::from_secs(45);
    loop {if let Some(result)=state().lock().map_err(|_|"录屏状态不可用")?.finished.as_ref().filter(|(nonce,_)|nonce==id).map(|(_,r)|r.clone()){
        result?;return Ok(true);}
        if std::time::Instant::now()>=until{return Err("等待录屏退出超时".into());}
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn wayland(window:&WebviewWindow)->Result<bool,String>{
    use gtk::prelude::*;
    let (tx,rx)=tokio::sync::oneshot::channel();let target=window.clone();
    window.run_on_main_thread(move||{let result=target.gtk_window().map(|_|crate::desktop_session_linux::uses_portal())
        .map_err(|_|"无法读取桌面类型".to_string());let _=tx.send(result);}).map_err(|_|"无法读取桌面类型")?;
    rx.await.map_err(|_|"桌面类型检查中断")?
}
#[tauri::command]
pub(crate) async fn prepare_wayland_recording(app:AppHandle,window:WebviewWindow,scene_id:String)->Result<Option<HostSnapshot>,String>{
    if window.label()!="space"{return Err("此窗口不能开始录屏".into());}
    if !wayland(&window).await?{return Ok(None);}
    let host=app.state::<Host>();host.exit.ensure_new_operation()?;
    let before=host.lock()?.store.snapshot();
    if before.active_scene_id!=scene_id{return Err("会话已变化".into());}
    let old=before.scenes.iter().find(|s|s.id==scene_id).ok_or("会话不存在")?;
    if old.closed||old.frozen||old.run.as_ref().is_some_and(|r|r.status==mewu_core::RunStatus::Running){return Err("请先完成当前会话".into());}
    let quick_continuation=crate::quick_capture::capture_continuation(&scene_id);
    {
        let s=state().lock().map_err(|_|"录屏状态不可用")?;
        if let Some(l)=&s.lease {
            if l.status.is_none()&&l.scene_id==scene_id&&l.background.as_ref()==old.background.as_ref()&&l.background.is_some()
                &&l.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_none(){return Ok(None);}
            return Err("请先完成当前录屏准备".into());
        }
    }
    crate::scroll_host::ensure_idle(&app)?;
    if recording::busy(&app){return Err("请先停止录屏".into());}
    let transition=Arc::new(crate::begin_space_transition(&app,&host)?);
    let nonce=uuid::Uuid::new_v4().to_string();let controls=Arc::new(Mutex::new(Controls::default()));
    let preview=host.assets.join(format!("{nonce}.png"));
    // A mapped parent gives GNOME a real transient relationship. Keep its
    // export guard in the supervised task through the matching Portal reply.
    let parent=crate::capture_shortcut_wayland_host::recording_parent(&window).await?;
    let config=worker::PrepareConfig{nonce:nonce.clone(),preview:preview.clone(),parent_window:parent.identifier().into()};
    {let mut s=state().lock().map_err(|_|"录屏状态不可用")?;host.exit.ensure_new_operation()?;
        if s.lease.is_some(){return Err("请先完成当前录屏准备".into());}
        s.lease=Some(Lease{nonce:nonce.clone(),scene_id:scene_id.clone(),background:None,region:None,
            transition:transition.clone(),
            controls:controls.clone(),status:None,finalized:false,suppress_reveal:false,grant:None,audio_grant:None});s.finished=None;
    }
    let mut preparing=PreparingGuard{nonce:nonce.clone(),armed:true};
    let (ready_tx,ready_rx)=tokio::sync::oneshot::channel::<Result<HostSnapshot,String>>();
    let owner=app.clone();let worker_nonce=nonce.clone();let source_scene=old.clone();let present_nonce=nonce.clone();
    let consent_window=window.clone();let consent_transition=transition.clone();
    tauri::async_runtime::spawn(async move{
        let run_app=owner.clone();let output=run_app.state::<Host>().root.join("recording-staging").join(format!("{worker_nonce}.mp4"));
        let run_preview=preview.clone();let run_output=output.clone();let mut ready_tx=Some(ready_tx);
        let result=tauri::async_runtime::spawn_blocking(move||{
            use std::sync::atomic::{AtomicBool,Ordering};
            let consent_received=AtomicBool::new(false);let preview_received=AtomicBool::new(false);
            worker::supervise(config,
            ||{
                let next=controls.lock().map(|mut c|c.take()).map_err(|_|"录屏控制不可用".to_string())?;
                if matches!(next,Some(worker::Control::Cancel{}|worker::Control::Stop{})){return Ok(next)}
                if !preview_received.load(Ordering::Acquire){
                    let result=worker::preparation_step(||pending_preparation_canceled(&run_app,&worker_nonce),||{
                        pending_source_current(&run_app,&worker_nonce,&source_scene)?;
                        if !consent_received.load(Ordering::Acquire){parent.check_mapped()?;}Ok(())
                    })?;
                    if result==worker::PreparationOutcome::Canceled{
                        return controls.lock().map(|mut c|c.take()).map_err(|_|"录屏控制不可用".into());
                    }
                }
                Ok(next)
            },
            ||worker::preparation_step(||pending_preparation_canceled(&run_app,&worker_nonce),||{
                pending_source_current(&run_app,&worker_nonce,&source_scene)?;parent.check_mapped()?;
                // Native unmap intentionally revokes the mapped action token.
                consent_received.store(true,Ordering::Release);
                let checked_app=run_app.clone();let checked_nonce=worker_nonce.clone();let checked_source=source_scene.clone();
                let allowed:crate::capture::CaptureCheck=Arc::new(move||pending_source_current(&checked_app,&checked_nonce,&checked_source));
                tauri::async_runtime::block_on(crate::capture_wayland_host::hide_recording_parent(&consent_window,allowed,consent_transition.clone()))
            }),
            |prepared|worker::preparation_step(||pending_preparation_canceled(&run_app,&worker_nonce),||{
                use std::os::unix::fs::MetadataExt;
                let file=std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&run_preview).map_err(|_|"录屏预览不存在")?;
                let metadata=file.metadata().map_err(|_|"无法检查录屏预览")?;
                if !metadata.is_file()||metadata.uid()!=unsafe{libc::geteuid()}||metadata.mode()&0o077!=0
                    ||metadata.len()>128*1024*1024{return Err("录屏预览身份无效".into());}
                let (width,height)=image::ImageReader::new(std::io::BufReader::new(file)).with_guessed_format()
                    .map_err(|_|"录屏预览格式无效")?.into_dimensions().map_err(|_|"录屏预览尺寸无效")?;
                if (width,height)!=(prepared.width,prepared.height){return Err("录屏预览尺寸不匹配".into());}
                let host=run_app.state::<Host>();host.exit.ensure_new_operation()?;
                let asset=Asset{id:prepared.nonce.clone(),name:"截图.png".into(),kind:AssetKind::Image,path:run_preview.to_string_lossy().into_owned(),
                    width:Some(width),height:Some(height),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None};
                {let s=state().lock().map_err(|_|"录屏状态不可用")?;
                    let lease=s.lease.as_ref().filter(|l|l.nonce==prepared.nonce).ok_or("录屏准备已取消")?;
                    if lease.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some(){return Err("录屏准备已取消".into());}
                }
                let snapshot={let mut engine=host.lock()?;let current=engine.store.snapshot();
                    if current.active_scene_id!=source_scene.id||current.scenes.iter().find(|s|s.id==source_scene.id)!=Some(&source_scene){return Err("录屏会话已经改变".into());}
                    engine.store.adopt_capture_background(asset.clone()).map_err(|e|e.to_string())?};
                {let mut s=state().lock().unwrap_or_else(|e|e.into_inner());
                    if let Some(lease)=s.lease.as_mut().filter(|l|l.nonce==prepared.nonce){
                        lease.scene_id=snapshot.active_scene_id.clone();lease.background=Some(asset);
                    }else{return Err("录屏准备已取消".into());}
                }
                // The Portal may replace the source scene after monitor consent.
                // Carry only this exact invocation's presentation intent before
                // normal publish reconciles it; never revive a retired mode.
                crate::quick_capture::continue_capture(&run_app,quick_continuation.as_ref(),&snapshot.active_scene_id);
                let published=crate::publish(&run_app,&snapshot);
                preview_received.store(true,Ordering::Release);
                if let Some(tx)=ready_tx.take(){let _=tx.send(Ok(published));}Ok(())
            }),|progress|{if let Ok(mut s)=state().lock(){if let Some(status)=s.lease.as_mut().and_then(|l|l.status.as_mut()){
                status.phase=if progress.paused{"paused"}else{"recording"};status.countdown=0;status.elapsed_ms=progress.elapsed_ms;emit(&run_app,&Some(status.clone()));}}},
            |_stage,_frames,_pcm|{if let Ok(mut s)=state().lock(){if let Some(status)=s.lease.as_mut().and_then(|l|l.status.as_mut()){
                status.phase="stopping";status.countdown=0;emit(&run_app,&Some(status.clone()));
            }}},
            ||{if let Ok(mut s)=state().lock(){if let Some(l)=s.lease.as_mut(){l.finalized=true;}}}
        )}).await.map_err(|_|"录屏任务中断".to_string()).and_then(|r|r);
        while crate::recording_worker::busy(){tokio::time::sleep(Duration::from_millis(25)).await;}
        let receipt=result.and_then(|report|{
            if let Some(report)=report {
                let (scene_id,background,region)={let s=state().lock().map_err(|_|"录屏状态不可用")?;let l=s.lease.as_ref().ok_or("录屏会话已变化")?;
                    (l.scene_id.clone(),l.background.clone().ok_or("录屏预览缺失")?,l.region.clone().ok_or("录屏选区缺失")?)};
                let host=owner.state::<Host>();let video_id=uuid::Uuid::new_v4().to_string();
                let final_path=host.assets.join(format!("{video_id}.mp4"));
                let asset=Asset{id:video_id,name:"录屏.mp4".into(),kind:AssetKind::Video,path:final_path.to_string_lossy().into_owned(),
                    width:Some(report.width),height:Some(report.height),origin_x:None,origin_y:None,scale_factor:None,capture_coordinates:None};
                let snapshot=recording::retain_completed_output(&run_output,&final_path,||{
                    crate::assets::open_video(&asset,&host.assets)?;
                    host.lock()?.store.finish_recording(&scene_id,&background.id,&region,asset).map_err(|e|e.to_string())
                })?;crate::publish(&owner,&snapshot);
            } Ok(())
        });
        // An adopted preview remains a persistent asset. Only an unadopted own
        // preview and unfinalized recording are eligible for exact cleanup.
        let (adopted,finalized,reveal)={let mut s=state().lock().unwrap_or_else(|e|e.into_inner());
            let l=s.lease.take();s.finished=Some((nonce.clone(),receipt.clone()));
            l.map_or((false,false,false),|l|(l.background.is_some(),l.finalized,l.status.is_some()&&!l.suppress_reveal))};
        if !adopted{let _=std::fs::remove_file(&preview);}
        if !finalized{let _=std::fs::remove_file(&output);}else if receipt.is_err(){let _=std::fs::rename(&output,output.with_extension("completed.mp4"));}
        emit(&owner,&None);
        if reveal{crate::show(&owner);}
        if let Err(error)=receipt{let _=owner.emit_to("space","host-error",error);}
    });
    let prepared=match ready_rx.await {Ok(result)=>result,Err(_)=>Err("录屏授权未完成".into())};
    let snapshot=match prepared {Ok(snapshot)=>snapshot,Err(error)=>{
        return Err(restore_preparation_failure(&app,&window,&present_nonce,&scene_id,old.background.clone(),transition,error).await);
    }};
    let checked_app=app.clone();let checked_nonce=present_nonce.clone();
    let allowed:crate::capture::CaptureCheck=Arc::new(move||preparation_current(&checked_app,&checked_nonce));
    if let Err(error)=crate::capture_wayland_host::present_recording(&window,allowed,transition.clone()).await{
        return Err(restore_preparation_failure(&app,&window,&present_nonce,&scene_id,old.background.clone(),transition,error).await);
    }
    preparation_current(&app,&present_nonce)?;
    preparing.armed=false;
    Ok(Some(snapshot))
}

fn tray_can_begin(actual:&str,expected:&str,phase:&str,terminal:bool)->bool{
    actual==expected&&phase=="countdown"&&!terminal
}
pub(crate) fn tray_admission(app:&AppHandle,nonce:&str)->Result<(),String>{
    app.state::<Host>().exit.ensure_new_operation()?;
    let s=state().lock().map_err(|_|"录屏状态不可用")?;
    let lease=s.lease.as_ref().ok_or("录屏准备已结束")?;
    let status=lease.status.as_ref().ok_or("录屏尚未开始")?;
    let controls=lease.controls.lock().map_err(|_|"录屏控制不可用")?;
    if !tray_can_begin(&lease.nonce,nonce,status.phase,controls.terminal.is_some()){
        return Err("录屏准备已结束".into());
    }
    Ok(())
}
async fn prepare_countdown_input(app:&AppHandle,window:&WebviewWindow,id:&str)->Result<(),String>{
    let (send,receive)=tokio::sync::oneshot::channel();
    let target=window.clone();let owner=app.clone();let id=id.to_owned();
    window.run_on_main_thread(move||{
        use gtk::prelude::*;
        let result=(||{
            if send.is_closed(){return Err("录屏倒计时已取消".to_string());}
            tray_admission(&owner,&id)?;
            if target.label()!="space"{return Err("录屏父窗口已变化".into());}
            let widget=target.gtk_window().map_err(|_|"录屏父窗口不可用")?;
            if !widget.is_mapped(){return Err("录屏父窗口已隐藏".into());}
            // CSS pointer-events cannot pass input to another application.
            // Use GTK's native Wayland input region for this owned surface;
            // keep the live mask/countdown painted until the normal unmap ack.
            widget.input_shape_combine_region(Some(&gtk::cairo::Region::create()));
            widget.set_accept_focus(false);
            widget.display().flush();
            Ok(())
        })();
        let _=send.send(result);
    }).map_err(|_|"无法释放倒计时期间的桌面输入")?;
    tokio::time::timeout(Duration::from_secs(2),receive).await
        .map_err(|_|"录屏倒计时输入准备超时".to_string())?
        .map_err(|_|"录屏倒计时输入准备中断".to_string())?
}

pub(crate) async fn start(app:AppHandle,window:WebviewWindow,scene_id:String,region_id:String,
    audio:crate::recording_audio_host::RecordingAudioSelection,grant:crate::plugins::RecordingGrant)->Result<bool,String>{
    if !wayland(&window).await?{return Ok(false);}
    let host=app.state::<Host>();host.exit.ensure_new_operation()?;
    let plugins=host.plugins.lock().map_err(|_|"插件存储不可用")?.store.snapshot()?;
    recording::validate_recording_grants(&grant,&audio,&plugins)?;
    let mode=crate::recording_audio_host::validate_selection(&app,&audio,&plugins)?;
    let snapshot=host.lock()?.store.snapshot();
    let scene=snapshot.scenes.iter().find(|s|s.id==scene_id&&snapshot.active_scene_id==scene_id&&!s.closed&&!s.frozen).ok_or("录屏会话已变化")?;
    let region=scene.regions.iter().find(|r|r.id==region_id&&r.image_override.is_none()).ok_or("录屏选区不存在")?.clone();
    if scene.run.as_ref().is_some_and(|r|r.status==mewu_core::RunStatus::Running){return Err("请等待当前回答完成".into());}
    let (id,controls,status,selection)={let mut s=state().lock().map_err(|_|"录屏状态不可用")?;
        let l=s.lease.as_mut().filter(|l|l.scene_id==scene_id&&l.status.is_none()&&l.background.as_ref()==scene.background.as_ref()).ok_or("请先授权录屏并重新框选")?;
        if l.controls.lock().map_err(|_|"录屏控制不可用")?.terminal.is_some(){return Err("录屏授权已结束，请重新授权".into());}
        let background=l.background.as_ref().ok_or("录屏预览未完成")?;
        let (mw,mh)=(background.width.ok_or("预览尺寸无效")?,background.height.ok_or("预览尺寸无效")?);
        if ![region.x,region.y,region.width,region.height].iter().all(|v|v.is_finite())||region.x<0.||region.y<0.
            ||region.width<2.||region.height<2.||region.x+region.width>mw as f64||region.y+region.height>mh as f64{return Err("录屏选区无效".into());}
        let selection=worker::Selection{nonce:l.nonce.clone(),output:host.root.join("recording-staging").join(format!("{}.mp4",l.nonce)),
            x:region.x.floor()as u32,y:region.y.floor()as u32,width:(region.width.floor()as u32)&!1,height:(region.height.floor()as u32)&!1,audio:mode};
        if u64::from(selection.width)*u64::from(selection.height)>4_000_000{return Err("录制区域不能超过400万像素".into());}
        let status=recording::Status{id:l.nonce.clone(),scene_id:scene_id.clone(),phase:"countdown",countdown:3,elapsed_ms:0,
            rect:recording::Rect{x:region.x/mw as f64,y:region.y/mh as f64,width:region.width/mw as f64,height:region.height/mh as f64},stop_hotkey:String::new(),audio:mode};
        l.region=Some(region);l.status=Some(status.clone());l.grant=Some(grant);l.audio_grant=audio.grant.clone();
        (l.nonce.clone(),l.controls.clone(),status,selection)};
    // The prepared worker is already alive. Every early error or dropped
    // invoke must cancel this exact lease until the countdown task owns it.
    let mut starting=StartingGuard{nonce:id.clone(),armed:true};
    // Keep the same capture surface visible for 3/2/1. It must be unmapped and
    // acknowledged before Start; a monitor stream cannot exclude this window.
    crate::recording_tray_linux::present(&app,&id).await?;
    prepare_countdown_input(&app,&window,&id).await?;
    // Preparation owns the transition through the live Portal worker. Starting
    // must share that exact guard, not try to acquire its own held lock again.
    let transition={let s=state().lock().map_err(|_|"录屏状态不可用")?;
        s.lease.as_ref().ok_or("录屏准备已结束")?.countdown_transition(&id)?};
    emit(&app,&Some(status));
    let owner=app.clone();tauri::async_runtime::spawn(async move {
        let result=async {
            for number in (1..=3).rev(){
                tray_admission(&owner,&id)?;
                let status={let mut s=state().lock().map_err(|_|"录屏状态不可用")?;
                    let status=s.lease.as_mut().filter(|l|l.nonce==id).and_then(|l|l.status.as_mut()).ok_or("录屏准备已结束")?;
                    status.countdown=number;status.clone()};
                emit(&owner,&Some(status));
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            let checked_owner=owner.clone();let checked_id=id.clone();
            let allowed:crate::capture::CaptureCheck=Arc::new(move||tray_admission(&checked_owner,&checked_id));
            crate::capture_wayland_host::hide_recording_parent(&window,allowed.clone(),transition).await?;
            allowed()?;
            let mut c=controls.lock().map_err(|_|"录屏控制不可用")?;
            if c.terminal.is_some(){return Err("录屏准备已结束".to_string());}
            c.start=Some(selection);Ok::<(),String>(())
        }.await;
        if let Err(error)=result {
            // Preserve an explicit Stop/Cancel. A failed hide must never start
            // recording the countdown or leave an unstarted worker alive.
            let report=controls.lock().map(|mut c|{if c.terminal.is_some(){false}else{c.terminal=Some(true);c.start=None;true}}).unwrap_or(true);
            if report&&owner.state::<Host>().exit.accepts_new_operations(){let _=owner.emit_to("space","host-error",error);}
        }
    });
    starting.armed=false;
    Ok(true)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    struct RestoreState(Option<State>);
    impl Drop for RestoreState {
        fn drop(&mut self){if let Some(previous)=self.0.take(){*state().lock().unwrap()=previous;}}
    }
    fn lease(nonce:&str)->(Lease,Arc<Mutex<Controls>>){
        let controls=Arc::new(Mutex::new(Controls::default()));
        let transition=Arc::new(crate::SpaceTransition::try_begin(&Arc::new(std::sync::atomic::AtomicBool::new(false))).unwrap());
        (Lease{nonce:nonce.into(),scene_id:"scene".into(),background:None,region:None,transition,
            controls:controls.clone(),status:Some(recording::Status{id:nonce.into(),scene_id:"scene".into(),
                phase:"countdown",countdown:3,elapsed_ms:0,rect:recording::Rect{x:0.,y:0.,width:1.,height:1.},
                stop_hotkey:String::new(),audio:backend::AudioMode::Mute}),finalized:false,suppress_reveal:false,grant:None,audio_grant:None},controls)
    }
    #[test]
    fn tray_admission_requires_the_exact_uncanceled_countdown(){
        assert!(tray_can_begin("current","current","countdown",false));
        assert!(!tray_can_begin("replacement","current","countdown",false));
        assert!(!tray_can_begin("current","current","countdown",true));
        for phase in ["starting","recording","paused","stopping"]{
            assert!(!tray_can_begin("current","current",phase,false));
        }
    }
    #[test]
    fn countdown_shares_prepared_transition_until_the_last_owner_finishes(){
        use std::sync::atomic::{AtomicBool,Ordering};
        let active=Arc::new(AtomicBool::new(false));
        let prepared=Arc::new(crate::SpaceTransition::try_begin(&active).unwrap());
        let (mut lease,controls)=lease("current");lease.transition=prepared.clone();
        assert!(crate::SpaceTransition::try_begin(&active).is_none());
        assert!(lease.countdown_transition("old").is_err());
        let countdown=lease.countdown_transition("current").unwrap();
        assert!(Arc::ptr_eq(&prepared,&countdown));
        controls.lock().unwrap().terminal=Some(true);
        assert!(lease.countdown_transition("current").is_err());
        drop(prepared);drop(lease);
        assert!(active.load(Ordering::Acquire));
        drop(countdown);
        assert!(!active.load(Ordering::Acquire));
        assert!(crate::SpaceTransition::try_begin(&active).is_some());
    }
    #[test]
    fn starting_invoke_drop_cancels_only_its_live_lease_and_preserves_stop(){
        let saved=std::mem::take(&mut *state().lock().unwrap());let _restore=RestoreState(Some(saved));
        let (current,commands)=lease("current");state().lock().unwrap().lease=Some(current);
        drop(StartingGuard{nonce:"current".into(),armed:true});
        let mut commands=commands.lock().unwrap();
        assert!(matches!(commands.take(),Some(worker::Control::Cancel{})));
        assert!(commands.take().is_none());drop(commands);

        // A replaced source must never be canceled by an old invoke's destructor.
        let (replacement,commands)=lease("replacement");state().lock().unwrap().lease=Some(replacement);
        drop(StartingGuard{nonce:"current".into(),armed:true});
        assert!(commands.lock().unwrap().take().is_none());
        // Likewise, cancellation of a late HUD invoke cannot discard an already
        // requested real Stop and change its completed video into Cancel.
        control("replacement","stop").unwrap();
        control("replacement","stop").unwrap();
        drop(StartingGuard{nonce:"replacement".into(),armed:true});
        assert!(matches!(commands.lock().unwrap().take(),Some(worker::Control::Stop{})));
        assert!(commands.lock().unwrap().take().is_none());

        let (transferred,commands)=lease("transferred");state().lock().unwrap().lease=Some(transferred);
        drop(StartingGuard{nonce:"transferred".into(),armed:false});
        assert!(commands.lock().unwrap().take().is_none());
    }
}
