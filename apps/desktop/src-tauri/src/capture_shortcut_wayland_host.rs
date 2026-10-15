// SPDX-License-Identifier: MPL-2.0
//! Trusted settings window adapter. GTK objects never leave the UI thread.
use crate::{capture_shortcut::CaptureShortcutState, capture_shortcut_portal_linux::Runtime};
use gtk::prelude::*;
use std::{ffi::{c_char,c_void,CStr}, sync::{Mutex,OnceLock}, time::Duration};
use tauri::{AppHandle,Manager,WebviewWindow};
use tokio::sync::oneshot;

pub(crate) fn is_wayland()->bool {
    gtk::gdk::Display::default().is_some_and(|d|d.type_().name()=="GdkWaylandDisplay")
}
#[derive(Default)]
struct Observed {widget:usize,generation:u64}
fn observed()->&'static Mutex<Observed>{static OBSERVED:OnceLock<Mutex<Observed>>=OnceLock::new();OBSERVED.get_or_init(||Mutex::new(Observed::default()))}
fn same_observer(widget:usize,generation:u64,unrealized:bool)->bool{
    let Ok(mut current)=observed().lock()else{return false};
    if current.widget!=widget||current.generation!=generation{return false}
    if unrealized{current.widget=0;}true
}
/// A system consent dialog changes focus, not the parent's mapping. Observe
/// true GTK hide/minimize/destruction independently of JS and the command queue.
pub(crate) fn observe_settings(window:&WebviewWindow)->Result<(),String>{
    if window.label()!="settings"{return Ok(())}
    let target=window.clone();
    window.run_on_main_thread(move||{
        let Some(runtime)=target.app_handle().try_state::<Runtime>()else{return};
        let Ok(widget)=target.gtk_window()else{return};
        // Weak runtime captures avoid a GtkWindow→AppHandle→window cycle.
        observe_widget(&widget,runtime.cancellation_observer());
    }).map_err(|_|"无法观察设置窗口生命周期".into())
}
fn observe_widget(widget:&impl gtk::glib::IsA<gtk::Widget>,cancel:impl Fn()+Clone+'static){
    let pointer=widget.as_ptr() as usize;
    let generation={
        let Ok(mut state)=observed().lock()else{cancel();return};
        if state.widget==pointer{return}
        if state.generation>=4096{cancel();return}
        state.generation+=1;state.widget=pointer;state.generation
    };
    let on_unmap=cancel.clone();
    widget.connect_unmap(move |_|{if same_observer(pointer,generation,false){on_unmap();}});
    widget.connect_unrealize(move |_|{if same_observer(pointer,generation,true){cancel();}});
}
struct Api {
    _library:libloading::Library,
    export:unsafe extern "C" fn(*mut c_void,unsafe extern "C" fn(*mut c_void,*const c_char,*mut c_void),*mut c_void,unsafe extern "C" fn(*mut c_void))->i32,
    unexport:unsafe extern "C" fn(*mut c_void),
}
fn api()->Result<&'static Api,String>{
    static API:OnceLock<Result<Api,String>>=OnceLock::new();
    API.get_or_init(||unsafe{
        let library=libloading::Library::new("libgdk-3.so.0").map_err(|_|"Wayland 窗口接口不可用")?;
        let export=*library.get(b"gdk_wayland_window_export_handle\0").map_err(|_|"Wayland 父窗口接口不可用")?;
        let unexport=*library.get(b"gdk_wayland_window_unexport_handle\0").map_err(|_|"Wayland 父窗口接口不可用")?;
        Ok(Api{_library:library,export,unexport})
    }).as_ref().map_err(Clone::clone)
}
pub(crate) struct ExportedParent { handle:String,window:WebviewWindow,weak:gtk::glib::SendWeakRef<gtk::gdk::Window>,owner:usize,label:&'static str }
impl ExportedParent {
    pub(crate) fn identifier(&self)->&str{&self.handle}
    pub(crate) fn check_mapped(&self)->Result<(),String>{
        if crate::window_actions_linux::current_for(self.owner,self.label){Ok(())}else{Err("授权父窗口已关闭".into())}
    }
}
impl Drop for ExportedParent {
    fn drop(&mut self){let weak=self.weak.clone();let _=self.window.run_on_main_thread(move||{
        if let Some(window)=weak.upgrade(){if let Ok(api)=api(){unsafe{(api.unexport)(window.as_ptr() as _)}}}
    });}
}
struct Callback { sender:Option<oneshot::Sender<Result<String,String>>> }
unsafe extern "C" fn exported(_: *mut c_void,handle:*const c_char,data:*mut c_void){
    let callback=&mut *(data as *mut Callback);
    if let Some(sender)=callback.sender.take(){
        let result=if handle.is_null(){Err("系统未提供父窗口标识".into())}else{
            CStr::from_ptr(handle).to_str().ok().filter(|h|!h.is_empty()&&h.len()<=512&&!h.chars().any(char::is_control)).map(|h|format!("wayland:{h}")).ok_or_else(||"系统父窗口标识无效".into())
        };let _=sender.send(result);
    }
}
unsafe extern "C" fn destroyed(data:*mut c_void){drop(Box::from_raw(data as *mut Callback));}
async fn parent(window:&WebviewWindow,owner:usize,label:&'static str)->Result<ExportedParent,String>{
    if !matches!(label,"settings"|"space")||window.label()!=label{return Err("授权父窗口无效".into())}
    let (setup,ready)=oneshot::channel::<Result<ExportedParent,String>>();let (result,handle)=oneshot::channel();let target=window.clone();
    window.run_on_main_thread(move||{
        let output=(||{
            let widget=crate::window_actions_linux::parent(&target,owner)?;
            if target.label()!=label||!crate::window_actions_linux::current_for(owner,label){return Err("授权父窗口已关闭".into())}
            let drawable=widget.window().ok_or("设置窗口尚未显示")?;
            if drawable.display().type_().name()!="GdkWaylandDisplay"{return Err("此窗口不属于 Wayland 桌面".into())}
            let api=api()?;
            let callback=Box::into_raw(Box::new(Callback{sender:Some(result)}));
            if unsafe{(api.export)(drawable.as_ptr() as _,exported,callback as _,destroyed)}==0{
                unsafe{destroyed(callback as _)};return Err("无法导出设置父窗口".into())
            }
            Ok(ExportedParent { handle:String::new(), window:target.clone(), weak:gtk::glib::SendWeakRef::from(drawable.downgrade()),owner,label })
        })();let _=setup.send(output);
    }).map_err(|_|"无法读取设置窗口")?;
    let mut guard=tokio::time::timeout(Duration::from_secs(5),ready).await.map_err(|_|"读取设置窗口超时")?.map_err(|_|"读取设置窗口中止")??;
    guard.handle=tokio::time::timeout(Duration::from_secs(5),handle).await.map_err(|_|"导出设置父窗口超时")?.map_err(|_|"导出设置父窗口中止")??;
    guard.check_mapped()?;Ok(guard)
}
/// Host-only entry: renderer cannot provide a parent handle or another label.
pub(crate) async fn recording_parent(window:&WebviewWindow)->Result<ExportedParent,String>{
    if window.label()!="space"{return Err("此窗口不能开始录屏".into())}
    let owner=crate::window_actions_linux::window_handle(window)?;
    parent(window,owner,"space").await
}
fn settings(window:&WebviewWindow)->Result<usize,String>{
    if window.label()!="settings"{return Err("请在设置中修改截图快捷键".into())}
    let owner=crate::window_actions_linux::window_handle(window)?;
    if !crate::window_actions_linux::current_for(owner,"settings"){return Err("设置窗口已关闭".into())}Ok(owner)
}
pub(crate) async fn configure(app:AppHandle,window:WebviewWindow,revision:u64,id:String)->Result<CaptureShortcutState,String>{
    let runtime=app.try_state::<Runtime>().ok_or("此平台不使用系统快捷键授权")?.inner().clone();
    if !uuid::Uuid::parse_str(&id).is_ok_and(|v|v.to_string()==id){return Err("快捷键请求标识无效".into())}
    let owner=settings(&window)?;let parent=parent(&window,owner,"settings").await?;runtime.configure(revision,id,owner,parent).await
}
pub(crate) async fn cancel(app:&AppHandle,window:&WebviewWindow,id:&str)->Result<(),String>{
    // Hide/destroy cancellation is intentionally allowed after GTK unmap. The
    // immutable canonical IPC label is enough to retire its own bounded nonce.
    if window.label()!="settings"{return Err("此窗口不能取消快捷键设置".into())}
    app.try_state::<Runtime>().ok_or("此平台不使用系统快捷键授权")?.inner().clone().cancel(id).await
}
pub(crate) async fn disable(app:AppHandle,window:WebviewWindow,revision:u64)->Result<CaptureShortcutState,String>{
    let owner=settings(&window)?;let runtime=app.try_state::<Runtime>().ok_or("系统快捷键尚未初始化")?.inner().clone();runtime.disable(revision,owner).await
}
pub(crate) fn system_settings_available()->bool{
    let path=std::path::Path::new("/usr/bin/gnome-control-center");
    std::fs::symlink_metadata(path).is_ok_and(|m|m.is_file()&&!m.file_type().is_symlink())
}
pub(crate) fn open_system_settings(app:&AppHandle,window:&WebviewWindow)->Result<(),String>{
    if app.try_state::<Runtime>().is_none(){return Err("此平台不使用系统快捷键授权".into())}
    let owner=settings(window)?;
    if !crate::window_actions_linux::current_for(owner,"settings")||!app.state::<crate::Host>().exit.accepts_new_operations()||!system_settings_available(){return Err("系统键盘设置不可用".into())}
    let mut child=std::process::Command::new("/usr/bin/gnome-control-center").arg("keyboard").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|_|"无法打开系统键盘设置")?;
    tauri::async_runtime::spawn_blocking(move||{let _=child.wait();});Ok(())
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn real_gtk_hide_and_unrealize_cancel_without_focus_loss_dependency(){
        use std::{cell::Cell,rc::Rc};
        gtk::init().expect("requires isolated xvfb-run GTK gate");
        let window=gtk::Window::new(gtk::WindowType::Toplevel);window.set_default_size(160,120);
        let calls=Rc::new(Cell::new(0));let callback=calls.clone();observe_widget(&window,move||callback.set(callback.get()+1));
        window.show_all();let until=std::time::Instant::now()+Duration::from_secs(2);
        while !window.is_mapped(){assert!(std::time::Instant::now()<until);gtk::main_iteration_do(false);}
        assert_eq!(calls.get(),0);window.hide();assert!(!window.is_mapped());assert_eq!(calls.get(),1);
        window.unrealize();assert_eq!(calls.get(),2);assert_eq!(observed().lock().unwrap().widget,0);
    }
}
