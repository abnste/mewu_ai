// SPDX-License-Identifier: MPL-2.0
//! Native Wayland surfaces stay on the GTK thread. A portal bitmap carries no
//! desktop geometry; it is adopted as an ordinary image in the existing space.
use crate::{capture, capture_delay_work::Work, capture_wayland_portal, Host, HostSnapshot, SpaceTransition};
use gtk::prelude::*;
use mewu_core::{Asset, AssetKind};
use std::{collections::HashMap, sync::{Arc, Mutex, OnceLock}, time::{Duration, Instant}};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::oneshot;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity { widget: usize, drawable: usize }
#[derive(Clone, Debug)]
struct Witness { label: String, generation: u64, revision: u64, identity: Identity, visible: bool, fullscreen: bool, resizable: bool, default_size: (i32,i32) }
#[derive(Default)]
struct Owner { generation: u64, revision: u64, identity: Option<Identity> }
#[derive(Default)]
struct Registry { owners: HashMap<String, Owner>, failed: bool }
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
fn eligible(label: &str) -> bool { matches!(label, "space" | "frozen-widget") || crate::pin_host::is_pin_label(label) }
impl Registry {
    fn bind(&mut self, label: &str, identity: Identity, visible: bool) -> Result<(Witness, bool), String> {
        if self.failed || !eligible(label) || identity.widget == 0 || identity.drawable == 0 {
            return Err("截图窗口身份不可用".into());
        }
        let owner = self.owners.entry(label.into()).or_default();
        let changed = owner.identity.as_ref() != Some(&identity);
        if changed {
            if owner.generation >= 4096 { self.failed = true; return Err("截图窗口服务需要重新启动".into()); }
            owner.generation += 1;
            owner.revision = 0;
            owner.identity = Some(identity.clone());
        }
        Ok((Witness { label: label.into(), generation: owner.generation, revision: owner.revision, identity, visible, fullscreen:false, resizable:false, default_size:(-1,-1) }, changed))
    }
    fn current(&self, witness: &Witness, hidden: bool) -> Result<(), String> {
        if !self.failed && self.owners.get(&witness.label).is_some_and(|o| {
            o.generation == witness.generation && o.identity.as_ref() == Some(&witness.identity)
                && (!hidden || o.revision == witness.revision)
        }) { Ok(()) } else { Err("截图窗口已改变，请重新截图".into()) }
    }
    fn revoke(&mut self, label: &str, generation: Option<u64>) {
        if let Some(owner) = self.owners.get_mut(label) {
            if generation.is_none_or(|g| g == owner.generation) { owner.identity = None; }
        }
    }
    fn mapped(&mut self, label: &str, generation: u64) {
        if let Some(owner) = self.owners.get_mut(label) {
            if generation == owner.generation && owner.identity.is_some() {
                if let Some(next) = owner.revision.checked_add(1) { owner.revision = next; }
                else { self.failed = true; }
            }
        }
    }
}
pub(crate) fn revoke_owner(label: &str) {
    if let Ok(mut r) = registry().lock() { r.revoke(label, None); }
}
fn witness(window: &WebviewWindow) -> Result<Witness, String> {
    let widget = window.gtk_window().map_err(|_| "无法读取截图窗口")?;
    if widget.window().is_none() { widget.realize(); }
    let drawable = widget.window().ok_or("截图窗口尚未创建")?;
    if drawable.display().type_().name() != "GdkWaylandDisplay" {
        return Err("截图窗口不属于当前 Wayland 桌面".into());
    }
    let identity = Identity { widget: widget.as_ptr() as usize, drawable: drawable.as_ptr() as usize };
    let (mut w, subscribe) = registry().lock().map_err(|_| "截图窗口服务不可用")?.bind(window.label(), identity, widget.is_visible())?;
    w.fullscreen=drawable.state().contains(gtk::gdk::WindowState::FULLSCREEN);
    w.resizable=widget.is_resizable();
    w.default_size=widget.default_size();
    if subscribe {
        let label = w.label.clone(); let generation = w.generation;
        widget.connect_unrealize(move |_| { if let Ok(mut r) = registry().lock() { r.revoke(&label, Some(generation)); } });
        let label = w.label.clone();
        widget.connect_map(move |_| { if let Ok(mut r) = registry().lock() { r.mapped(&label, generation); } });
    }
    Ok(w)
}
fn current(witnesses: &[Witness], hidden: bool) -> Result<(), String> {
    let r = registry().lock().map_err(|_| "截图窗口服务不可用")?;
    for w in witnesses { r.current(w, hidden)?; }
    Ok(())
}
fn base_check(app: &AppHandle, work: &Work) -> Result<(), String> {
    work.check()?;
    app.state::<Host>().exit.ensure_new_operation()?;
    crate::recording::ensure_idle(app)
}
fn owned_set_current(app: &AppHandle, owned: &[Witness]) -> Result<(), String> {
    let mut now:Vec<_>=app.webview_windows().into_keys().filter(|label|eligible(label)).collect();
    let mut expected:Vec<_>=owned.iter().map(|w|w.label.clone()).collect();
    now.sort(); expected.sort();
    if now==expected {Ok(())} else {Err("截图窗口集合已改变，请重新截图".into())}
}
pub(crate) async fn is_wayland(window: &WebviewWindow, work: Arc<Work>, transition: Arc<SpaceTransition>) -> Result<bool, String> {
    let (tx, rx) = oneshot::channel(); let target = window.clone();
    window.run_on_main_thread(move || {
        let _transition = transition;
        let result = base_check(target.app_handle(), &work).and_then(|_| {
            let widget = target.gtk_window().map_err(|_| "无法读取桌面窗口")?;
            Ok(widget.display().type_().name() == "GdkWaylandDisplay")
        });
        let _ = tx.send(result);
    }).map_err(|_| "无法检查桌面类型")?;
    rx.await.map_err(|_| "桌面类型检查中断")?
}

// wl_display.sync is asynchronous. GTK dispatches the default Wayland queue;
// never read/dispatch its fd on a worker and never use blocking roundtrip here.
mod barrier {
    use super::*;
    use std::{cell::Cell, ffi::c_void, rc::Rc};
    type Proxy = *mut c_void;
    struct Api {
        _gdk: libloading::Library, _wl: libloading::Library,
        display: unsafe extern "C" fn(Proxy) -> Proxy,
        marshal: unsafe extern "C" fn(Proxy,u32,*const c_void,u32,u32,...) -> Proxy,
        listener: unsafe extern "C" fn(Proxy,*const c_void,*mut c_void) -> i32,
        destroy: unsafe extern "C" fn(Proxy), interface: usize,
    }
    fn api() -> Result<&'static Api, String> {
        static API: OnceLock<Result<Api,String>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let gdk = libloading::Library::new("libgdk-3.so.0").map_err(|_| "Wayland 窗口库不可用")?;
            let wl = libloading::Library::new("libwayland-client.so.0").map_err(|_| "Wayland 客户端库不可用")?;
            let display = *gdk.get(b"gdk_wayland_display_get_wl_display\0").map_err(|_| "Wayland 显示接口不可用")?;
            let marshal = *wl.get(b"wl_proxy_marshal_flags\0").map_err(|_| "Wayland 同步接口不可用")?;
            let listener = *wl.get(b"wl_proxy_add_listener\0").map_err(|_| "Wayland 回调接口不可用")?;
            let destroy = *wl.get(b"wl_proxy_destroy\0").map_err(|_| "Wayland 回调接口不可用")?;
            let interface = *wl.get::<*const c_void>(b"wl_callback_interface\0").map_err(|_| "Wayland 回调类型不可用")? as usize;
            Ok(Api { _gdk:gdk, _wl:wl, display, marshal, listener, destroy, interface })
        }).as_ref().map_err(Clone::clone)
    }
    struct Data { slot: Rc<Cell<*mut Data>>, proxy: Proxy, tx: Option<oneshot::Sender<Result<(),String>>>, allowed: capture::CaptureCheck, _work: Arc<Work>, _transition: Arc<SpaceTransition> }
    #[repr(C)] struct Listener { done: unsafe extern "C" fn(*mut c_void,Proxy,u32) }
    static LISTENER: Listener = Listener { done };
    unsafe extern "C" fn done(data: *mut c_void, proxy: Proxy, _: u32) {
        let mut data = Box::from_raw(data.cast::<Data>());
        data.slot.set(std::ptr::null_mut());
        // The compositor destroys its resource; the client proxy still needs destruction.
        if let Ok(api) = api() { (api.destroy)(proxy); }
        let result = (data.allowed)();
        if let Some(tx) = data.tx.take() { let _ = tx.send(result); }
    }
    pub(super) fn start(display: &gtk::gdk::Display, tx: oneshot::Sender<Result<(),String>>, allowed: capture::CaptureCheck, work: Arc<Work>, transition: Arc<SpaceTransition>) -> Result<(),String> {
        let api = api()?;
        let wl_display = unsafe { (api.display)(display.as_ptr().cast()) };
        if wl_display.is_null() { return Err("Wayland 显示连接不可用".into()); }
        let proxy = unsafe { (api.marshal)(wl_display,0,api.interface as *const c_void,1,0,std::ptr::null::<c_void>()) };
        if proxy.is_null() { return Err("无法确认桌面窗口隐藏".into()); }
        let slot = Rc::new(Cell::new(std::ptr::null_mut()));
        let data = Box::into_raw(Box::new(Data { slot:slot.clone(), proxy, tx:Some(tx), allowed, _work:work, _transition:transition }));
        slot.set(data);
        if unsafe { (api.listener)(proxy, (&LISTENER as *const Listener).cast(), data.cast()) } != 0 {
            unsafe { (api.destroy)(proxy); drop(Box::from_raw(data)); }
            return Err("无法订阅桌面窗口状态".into());
        }
        display.flush();
        let until = Instant::now()+Duration::from_secs(2);
        gtk::glib::timeout_add_local(Duration::from_millis(25), move || {
            let ptr = slot.get();
            if ptr.is_null() { return gtk::glib::ControlFlow::Break; }
            let result = unsafe { ((*ptr).allowed)() };
            if result.is_err() || Instant::now() >= until {
                slot.set(std::ptr::null_mut());
                unsafe {
                    let mut data = Box::from_raw(ptr);
                    (api.destroy)(data.proxy);
                    if let Some(tx) = data.tx.take() { let _ = tx.send(result.and(Err("桌面窗口隐藏确认超时".into()))); }
                }
                gtk::glib::ControlFlow::Break
            } else { gtk::glib::ControlFlow::Continue }
        });
        Ok(())
    }
}

async fn hide(app: &AppHandle, window: &WebviewWindow, work: Arc<Work>, transition: Arc<SpaceTransition>) -> Result<(Vec<Witness>,Result<(),String>),String> {
    let (tx, rx) = oneshot::channel(); let target = window.clone(); let app = app.clone();
    let hide_work=work.clone(); let hide_transition=transition.clone();
    window.run_on_main_thread(move || {
        let _transition=hide_transition; let work=hide_work;
        let result: Result<(Vec<Witness>, Result<(), String>), String> = (|| {
            base_check(&app,&work)?;
            let mut owned = Vec::new();
            for (label, candidate) in app.webview_windows() {
                if eligible(&label) { owned.push(witness(&candidate)?); }
            }
            let hidden=(|| {
                for w in &owned {
                    let candidate = app.get_webview_window(&w.label).ok_or("截图窗口已关闭")?;
                    candidate.hide().map_err(|_| "无法隐藏截图窗口")?;
                    if candidate.gtk_window().map_err(|_| "截图窗口不可用")?.is_mapped() { return Err("截图窗口尚未取消映射".into()); }
                }
                current(&owned,true)
            })();
            Ok((owned,hidden))
        })();
        let _ = tx.send(result);
    }).map_err(|_| "无法准备截图窗口")?;
    let (owned,hidden) = rx.await.map_err(|_| "截图准备中断")??;
    if hidden.is_err(){return Ok((owned,hidden));}
    let (tx,rx) = oneshot::channel(); let target=target.clone(); let witnesses=owned.clone(); let app=window.app_handle().clone(); let check_work=work.clone();
    let allowed: capture::CaptureCheck = Arc::new(move || { base_check(&app,&check_work)?; owned_set_current(&app,&witnesses)?; current(&witnesses,true) });
    window.run_on_main_thread(move || {
        let result = (|| {
            allowed()?;
            let widget=target.gtk_window().map_err(|_| "截图窗口不可用")?;
            barrier::start(&widget.display(),tx,allowed,work,transition)
        })();
        // A start failure closes its sender; receiver rejects without adopting an image.
        let _ = result;
    }).map_err(|_| "无法确认截图窗口隐藏")?;
    let hidden=rx.await.map_err(|_| "截图窗口隐藏确认中断".to_string()).and_then(|result|result);
    Ok((owned,hidden))
}

#[cfg(debug_assertions)]
fn log_layout(widget: &gtk::ApplicationWindow, stage: &str) {
    let allocation = widget.allocation();
    eprintln!("mewu-wayland-layout stage={stage} allocation={},{},{}x{} surface={:?} default={:?} resizable={} decorated={} fullscreen={} mapped={}",
        allocation.x(), allocation.y(), allocation.width(), allocation.height(),
        widget.window().map(|window| (window.width(), window.height())), widget.default_size(), widget.is_resizable(), widget.is_decorated(),
        widget.window().is_some_and(|window| window.state().contains(gtk::gdk::WindowState::FULLSCREEN)), widget.is_mapped());
    let style = widget.style_context();
    eprintln!("mewu-wayland-layout stage={stage} active={} transient-parent={:?}", widget.is_active(),
        widget.transient_for().map(|parent|(parent.is_mapped(),parent.is_active(),parent.allocated_width(),parent.allocated_height())));
    eprintln!("mewu-wayland-layout stage={stage} classes={:?} padding={:?} border={:?} margin={:?}",
        style.list_classes(), style.padding(style.state()), style.border(style.state()), style.margin(style.state()));
    fn webview_layout(widget: &gtk::Widget, stage: &str, depth: u8) {
        let allocation = widget.allocation();
        eprintln!("mewu-wayland-layout stage={stage} own-child-depth={depth} class={} allocation={},{},{}x{} requested={}x{} preferred-width={:?} preferred-height={:?}",
            widget.type_().name(), allocation.x(), allocation.y(), allocation.width(), allocation.height(), widget.width_request(), widget.height_request(), widget.preferred_width(), widget.preferred_height());
        if depth < 4 {
            if let Some(container) = widget.dynamic_cast_ref::<gtk::Container>() {
                for child in container.children().into_iter().take(16) { webview_layout(&child, stage, depth + 1); }
            }
        }
    }
    webview_layout(widget.upcast_ref(), stage, 0);
}

fn webview_size(widget: &gtk::Widget, depth: u8) -> Option<(i32,i32)> {
    if widget.type_().name()=="WebKitWebView" {return Some((widget.allocated_width(),widget.allocated_height()));}
    if depth<4 {
        if let Some(container)=widget.dynamic_cast_ref::<gtk::Container>() {
            for child in container.children().into_iter().take(16) {
                if let Some(size)=webview_size(&child,depth+1){return Some(size);}
            }
        }
    }
    None
}

fn monitor_size(drawable: &gtk::gdk::Window) -> Option<(i32,i32)> {
    drawable.display().monitor_at_window(drawable).map(|monitor| {
        let geometry=monitor.geometry();
        (geometry.width(),geometry.height())
    }).filter(|size|size.0>0 && size.1>0)
}

// Only this presentation's own GTK events, at most32 and at most the existing
// three-second presentation lifetime. Disconnect on every completion/error.
struct PresentationTrace {
    #[cfg(debug_assertions)] widget: gtk::ApplicationWindow,
    #[cfg(debug_assertions)] handlers: Vec<gtk::glib::SignalHandlerId>,
}
impl PresentationTrace {
    fn new(_widget: &gtk::ApplicationWindow) -> Self {
        #[cfg(debug_assertions)] {
            use std::{cell::Cell,rc::Rc};
            let remaining=Rc::new(Cell::new(32u8));
            let quota=remaining.clone();
            let resizable=_widget.connect_notify_local(Some("resizable"),move |widget,_| {
                if quota.get()>0 {
                    quota.set(quota.get()-1);
                    eprintln!("mewu-wayland-layout stage=resizable-notify resizable={} default={:?} allocation={}x{} mapped={}",
                        widget.is_resizable(),widget.default_size(),widget.allocated_width(),widget.allocated_height(),widget.is_mapped());
                }
            });
            let configure=_widget.connect_configure_event(move |widget,event| {
                if remaining.get()>0 {
                    remaining.set(remaining.get()-1);
                    eprintln!("mewu-wayland-layout stage=configure-event event-size={:?} resizable={} default={:?} fullscreen={} mapped={}",
                        event.size(),widget.is_resizable(),widget.default_size(),
                        widget.window().is_some_and(|drawable|drawable.state().contains(gtk::gdk::WindowState::FULLSCREEN)),widget.is_mapped());
                }
                false
            });
            Self {widget:_widget.clone(),handlers:vec![resizable,configure]}
        }
        #[cfg(not(debug_assertions))] {Self {}}
    }
}
#[cfg(debug_assertions)]
impl Drop for PresentationTrace {
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {self.widget.disconnect(handler);}
    }
}

fn presentation_timeout(_widget: &gtk::ApplicationWindow, _monitor_size: Option<(i32,i32)>, _final_size: Option<(i32,i32)>) -> String {
    #[cfg(debug_assertions)] {
        log_layout(_widget,"fullscreen-timeout");
        eprintln!("mewu-wayland-layout stage=fullscreen-timeout monitor-logical-size={_monitor_size:?} final-show-size={_final_size:?}");
    }
    "桌面未确认全屏空间".into()
}

enum PresentationPermit {
    Screenshot(Arc<Work>),
    Recording(capture::CaptureCheck),
}
impl PresentationPermit {
    fn check(&self, app: &AppHandle, owned: &[Witness]) -> Result<(),String> {
        match self {
            Self::Screenshot(work) => {base_check(app,work)?;owned_set_current(app,owned)},
            Self::Recording(allowed) => {app.state::<Host>().exit.ensure_new_operation()?;allowed()},
        }
    }
}

async fn present(window: &WebviewWindow, owned: Vec<Witness>, permit: PresentationPermit, transition: Arc<SpaceTransition>) -> Result<Witness,String> {
    let (tx,rx)=oneshot::channel(); let target=window.clone();
    window.run_on_main_thread(move || {
        let mut tx=Some(tx);
        let start=(|| {
            permit.check(target.app_handle(),&owned)?; current(&owned,true)?;
            let widget=target.gtk_window().map_err(|_| "截图窗口不可用")?;
            #[cfg(debug_assertions)] log_layout(&widget, "before-fullscreen");
            let trace=PresentationTrace::new(&widget);
            let drawable=widget.window().ok_or("截图窗口尚未创建")?;
            let initial_size=monitor_size(&drawable).ok_or("截图显示器不可用")?;
            widget.set_opacity(0.0);
            // GTK fixes a nonresizable window's minimum/maximum at its default
            // width (1200 here), even when the compositor's output is smaller.
            // Real fullscreen already disables resize edges and decorations.
            widget.set_resizable(true);
            // Prepare the owned monitor size BEFORE the first map/configure.
            // Tao's first-configure callback may restore resizable(false).
            // GTK then constrains fullscreen against this default and caches
            // that fixed size; resizing only after configure cannot undo it.
            // This is a request, never proof: the timer still checks the actual
            // monitor and every native allocation after the final show.
            widget.set_default_size(initial_size.0,initial_size.1);
            widget.resize(initial_size.0,initial_size.1);
            #[cfg(debug_assertions)] eprintln!("mewu-wayland-layout stage=before-first-show monitor-logical-size={initial_size:?} default={:?} resizable={}",widget.default_size(),widget.is_resizable());
            widget.fullscreen();
            target.show().map_err(|_| "无法显示截图空间")?;
            // The approved image is already acquired. Present this actual
            // space now: a visible settings toplevel may otherwise keep focus,
            // while waiting for fullscreen before focus creates a deadlock.
            // No settings window is hidden/destroyed and no permission anchor
            // is created before the user's portal consent.
            target.unminimize().map_err(|_| "无法显示截图空间")?;
            target.set_focus().map_err(|_| "无法显示截图空间")?;
            Ok((widget,initial_size,trace))
        })();
        match start {
            Err(error)=>{ if let Some(tx)=tx.take(){let _=tx.send(Err(error));} }
            Ok((widget,initial_size,trace))=> {
                let until=Instant::now()+Duration::from_secs(3);
                let mut final_size=None;
                let mut expected_monitor=Some(initial_size);
                let mut stable_ticks=0;
                gtk::glib::timeout_add_local(Duration::from_millis(15),move || {
                    let _transition=&transition;
                    let _trace=&trace;
                    let result=(|| {
                        permit.check(target.app_handle(),&owned)?; current(&owned,false)?;
                        for w in owned.iter().filter(|w|w.label!="space") {current(std::slice::from_ref(w),true)?;}
                        let drawable=widget.window().ok_or("截图窗口已关闭")?;
                        if Instant::now()>=until { return Err(presentation_timeout(&widget,expected_monitor,final_size)); }
                        // Tao applies its initial resizable flag on the first
                        // configure event. Reassert after that event if needed,
                        // then wait for GTK to allocate the fullscreen contents.
                        if !widget.is_resizable() {
                            if Instant::now()>=until { return Err(presentation_timeout(&widget,expected_monitor,final_size)); }
                            widget.set_resizable(true);
                            stable_ticks=0;
                            return Ok(None);
                        }
                        // FULLSCREEN may arrive before the new allocation. In
                        // that interval the startup1200x780 is still positive,
                        // so it cannot establish the fullscreen target. Read
                        // only logical dimensions of this own surface's monitor;
                        // no monitor origin or screenshot dimensions are used.
                        let expected=monitor_size(&drawable);
                        if expected_monitor!=expected {
                            expected_monitor=expected;
                            final_size=None;
                            stable_ticks=0;
                            #[cfg(debug_assertions)] eprintln!("mewu-wayland-layout stage=monitor-target logical-size={expected:?}");
                            if let Some((width,height))=expected {
                                // GTK remembers the previous mapped size. A
                                // fullscreen flag alone need not replace its
                                // startup width; default_size alone also does
                                // not resize an already realized window. Request
                                // the owned monitor's logical size once, then
                                // await actual allocations instead of assuming
                                // the request was accepted by the compositor.
                                widget.set_default_size(width,height);
                                widget.resize(width,height);
                                return Ok(None);
                            }
                        }
                        let size=(widget.allocated_width(),widget.allocated_height());
                        let ready=drawable.state().contains(gtk::gdk::WindowState::FULLSCREEN) && widget.is_mapped()
                            && expected==Some(size) && (drawable.width(),drawable.height())==size
                            && webview_size(widget.upcast_ref(),0)==Some(size);
                        if ready {
                            stable_ticks+=1;
                            if stable_ticks>=2 {
                                if final_size.is_none() {
                                    #[cfg(debug_assertions)] log_layout(&widget, "fullscreen-confirmed");
                                    // Rebase only after GTK, GDK and WebKit all
                                    // match the actual monitor twice. The final
                                    // show/deiconify can then negotiate safely.
                                    widget.set_default_size(size.0,size.1);
                                    crate::show(target.app_handle());
                                    final_size=Some(size);
                                    stable_ticks=0;
                                    return Ok(None);
                                }
                                #[cfg(debug_assertions)] log_layout(&widget, "after-final-show-stable");
                                return witness(&target).map(Some);
                            }
                        } else {stable_ticks=0;}
                        if Instant::now()>=until { return Err(presentation_timeout(&widget,expected_monitor,final_size)); }
                        Ok(None)
                    })();
                    match result {
                        Ok(None)=>gtk::glib::ControlFlow::Continue,
                        result=>{
                            if let Some(tx)=tx.take(){let _=tx.send(result.and_then(|v|v.ok_or("截图空间不可用".into())));}
                            gtk::glib::ControlFlow::Break
                        }
                    }
                });
            }
        }
    }).map_err(|_| "无法准备截图空间")?;
    rx.await.map_err(|_| "截图空间配置中断")?
}

// Recording has an already granted, nonce-bound live session. It cannot use
// Screenshot's idle admission, nor authorize any other application's surface.
pub(crate) async fn present_recording(window: &WebviewWindow, allowed: capture::CaptureCheck, transition: Arc<SpaceTransition>) -> Result<(),String> {
    if window.label()!="space" {return Err("此窗口不能开始录屏".into());}
    let (tx,rx)=oneshot::channel();let target=window.clone();let check=allowed.clone();let initial_transition=transition.clone();
    window.run_on_main_thread(move || {
        let _transition=initial_transition;
        let result=(|| {
            target.app_handle().state::<Host>().exit.ensure_new_operation()?;check()?;
            let widget=target.gtk_window().map_err(|_|"截图窗口不可用")?;
            if widget.is_mapped(){return Err("截图窗口尚未取消映射".into());}
            witness(&target)
        })();
        let _=tx.send(result);
    }).map_err(|_|"无法准备截图空间")?;
    let previous=rx.await.map_err(|_|"截图空间配置中断")??;
    let configured=present(window,vec![previous.clone()],PresentationPermit::Recording(allowed.clone()),transition.clone()).await;
    let (tx,rx)=oneshot::channel();let target=window.clone();
    window.run_on_main_thread(move || {
        let _transition=transition;
        let result=(|| {
            current(std::slice::from_ref(&previous),false)?;
            let widget=target.gtk_window().map_err(|_|"截图窗口不可用")?;
            let result=configured.and_then(|configured| {
                target.app_handle().state::<Host>().exit.ensure_new_operation()?;allowed()?;
                current(std::slice::from_ref(&configured),true)
            });
            if result.is_err(){
                widget.hide();
                widget.set_default_size(previous.default_size.0,previous.default_size.1);
                widget.set_resizable(previous.resizable);
                if !previous.fullscreen{widget.unfullscreen();}
            }
            // No second show/deiconify: stable native allocation is already
            // established. Cancellation restores opacity but keeps it hidden.
            widget.set_opacity(1.0);
            result
        })();
        let _=tx.send(result);
    }).map_err(|_|"无法准备截图空间")?;
    rx.await.map_err(|_|"截图空间配置中断")?
}

pub(crate) async fn capture(app: AppHandle, window: WebviewWindow, work: Arc<Work>, transition: Arc<SpaceTransition>, preferences: crate::capture_preferences::CapturePreferences) -> Result<HostSnapshot,String> {
    crate::pin_window::capture_begin(&app);
    {
        let host=app.state::<Host>(); let engine=host.lock()?;
        engine.ocr_jobs.cancel_owner("space");
        engine.translation_jobs.cancel_scene("space",&engine.store.snapshot().active_scene_id);
    }
    let (owned,hidden)=hide(&app,&window,work.clone(),transition.clone()).await?;
    let check_app=app.clone(); let check_work=work.clone(); let check_owned=owned.clone();
    let allowed: capture::CaptureCheck=Arc::new(move || {base_check(&check_app,&check_work)?; owned_set_current(&check_app,&check_owned)?; current(&check_owned,true)});
    let result=async {
        hidden?;
        work.wait_delay(preferences.capture_delay_seconds.seconds(),||allowed()).await?;
        let capture_root=app.state::<Host>().assets.clone(); let native_check=allowed.clone();
        let worker_work=work.clone(); let worker_transition=transition.clone();
        let mut pending=tauri::async_runtime::spawn_blocking(move || {
            let _work=worker_work; let _transition=worker_transition;
            capture_wayland_portal::capture(&capture_root,capture::CaptureOptions {include_cursor:preferences.include_cursor},native_check)
        }).await.map_err(|_|"截图任务中断")??;
        allowed()?;
        let configured=present(&window,owned.clone(),PresentationPermit::Screenshot(work.clone()),transition.clone()).await?;
        let asset=Asset {id:pending.asset_id.clone(),name:"截图.png".into(),kind:AssetKind::Image,path:pending.path.to_string_lossy().into_owned(),width:Some(pending.width),height:Some(pending.height),capture_coordinates:None,origin_x:None,origin_y:None,scale_factor:None};
        let host=app.state::<Host>(); let mut engine=host.lock()?;
        base_check(&app,&work)?; owned_set_current(&app,&owned)?; current(std::slice::from_ref(&configured),true)?;
        for w in owned.iter().filter(|w|w.label!="space") {current(std::slice::from_ref(w),true)?;}
        let snapshot=engine.store.adopt_capture_background(asset).map_err(|e|e.to_string())?;
        pending.retain();
        Ok(crate::publish(&app,&snapshot))
    }.await;
    // Restoration is deferred on the GTK queue and keeps the same operation
    // ownership. Exit/cancel or replacement cannot resurrect an old window.
    let restore_app=app.clone(); let restore_work=work.clone(); let restore_transition=transition.clone(); let success=result.is_ok();
    let (restore_tx,restore_rx)=oneshot::channel::<()>();
    let _=window.run_on_main_thread(move || {
        let _transition=restore_transition;
        let _done=restore_tx;
        if current(&owned,false).is_err(){return;}
        let allowed=base_check(&restore_app,&restore_work).is_ok();
        if let Some(space)=restore_app.get_webview_window("space") {
            if let Ok(widget)=space.gtk_window(){
                if !allowed {widget.hide();}
                widget.set_opacity(1.0);
                if !success {
                    if let Some(previous)=owned.iter().find(|w|w.label=="space") {
                        widget.set_default_size(previous.default_size.0,previous.default_size.1);
                        widget.set_resizable(previous.resizable);
                    }
                }
                if !success && !owned.iter().any(|w|w.label=="space"&&w.fullscreen) {widget.unfullscreen();}
            }
            if allowed && (success || owned.iter().any(|w|w.label=="space"&&w.visible)){
                // Successful capture already completed its final native show
                // and allocation checks before committing/publishing the image.
                if !success {crate::show(&restore_app);}
                #[cfg(debug_assertions)] {
                    let diagnostic_space=space.clone(); let diagnostic_owned=owned.clone();
                    gtk::glib::idle_add_local_once(move || {
                        if current(&diagnostic_owned,false).is_ok() {
                            if let Ok(widget)=diagnostic_space.gtk_window(){log_layout(&widget,"post-show-idle");}
                        }
                    });
                }
            }
            else {let _=space.hide();}
        }
        for w in owned.iter().filter(|w|allowed&&w.visible&&w.label!="space"&&(!success||w.label!="frozen-widget")) {
            if let Some(window)=restore_app.get_webview_window(&w.label){let _=window.show();}
        }
    });
    // The queued operation must finish before the invoke's CancelOnDrop fires.
    let _=restore_rx.await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn remapped_or_reused_surface_cannot_authorize_a_pending_portal_capture() {
        let mut r=Registry::default(); let identity=Identity{widget:1,drawable:2};
        let (old,_)=r.bind("space",identity.clone(),true).unwrap();
        r.mapped("space",old.generation); assert!(r.current(&old,true).is_err()); r.current(&old,false).unwrap();
        r.revoke("space",None); let (new,_)=r.bind("space",identity,true).unwrap();
        assert!(r.current(&old,false).is_err()); r.current(&new,true).unwrap();
        r.revoke("space",Some(old.generation)); r.current(&new,true).unwrap();
    }
}
