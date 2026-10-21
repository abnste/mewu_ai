// SPDX-License-Identifier: MPL-2.0
//! Native Quartz/Accessibility backend. No shell, clipboard or AppleScript.
//! Pixel coordinates from the model are mapped through capture-time Quartz
//! geometry. Every action rechecks the same process lifetime/window and focus.
use super::*;
use core_foundation::{
    base::{CFType, CFTypeRef, CFGetTypeID, CFEqual, TCFType},
    string::{CFString, CFStringRef},
    array::{CFArrayRef, CFArrayGetCount, CFArrayGetValueAtIndex},
    dictionary::{CFDictionaryRef, CFDictionaryGetTypeID, CFDictionaryGetValue},
    number::{CFNumberGetTypeID, CFNumberGetValue, kCFNumberSInt64Type},
};
use std::ffi::c_void;

#[repr(C)] #[derive(Clone,Copy,Default)] struct Point { x:f64,y:f64 }
#[repr(C)] #[derive(Clone,Copy,Default)] struct Size { width:f64,height:f64 }
#[repr(C)] #[derive(Clone,Copy,Default)] struct Rect { origin:Point,size:Size }
#[link(name="ApplicationServices",kind="framework")]
extern "C" {
    fn AXIsProcessTrusted()->bool;
    fn AXUIElementCreateSystemWide()->CFTypeRef;
    fn AXUIElementCreateApplication(pid:i32)->CFTypeRef;
    fn AXUIElementGetPid(element:CFTypeRef,pid:*mut i32)->i32;
    fn AXUIElementSetMessagingTimeout(element:CFTypeRef,seconds:f32)->i32;
    fn AXUIElementCopyAttributeValue(element:CFTypeRef,name:CFStringRef,value:*mut CFTypeRef)->i32;
    fn AXUIElementSetAttributeValue(element:CFTypeRef,name:CFStringRef,value:CFTypeRef)->i32;
    fn AXUIElementCopyElementAtPosition(element:CFTypeRef,x:f32,y:f32,value:*mut CFTypeRef)->i32;
    fn AXValueGetType(value:CFTypeRef)->u32;
    fn AXValueGetValue(value:CFTypeRef,kind:u32,out:*mut c_void)->bool;
    fn AXValueGetTypeID()->usize;
    fn AXUIElementGetTypeID()->usize;
}
#[link(name="CoreGraphics",kind="framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(options:u32,relative:u32)->CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict:CFDictionaryRef,rect:*mut Rect)->bool;
    static kCGWindowNumber:CFStringRef;
    static kCGWindowOwnerPID:CFStringRef;
    static kCGWindowLayer:CFStringRef;
    static kCGWindowBounds:CFStringRef;
    fn CGEventCreate(source:CFTypeRef)->CFTypeRef;
    fn CGEventGetLocation(event:CFTypeRef)->Point;
    fn CGEventCreateMouseEvent(source:CFTypeRef,kind:u32,point:Point,button:u32)->CFTypeRef;
    fn CGEventCreateKeyboardEvent(source:CFTypeRef,key:u16,down:bool)->CFTypeRef;
    fn CGEventCreateScrollWheelEvent(source:CFTypeRef,units:u32,count:u32,wheel:i32,...)->CFTypeRef;
    fn CGEventKeyboardSetUnicodeString(event:CFTypeRef,len:usize,units:*const u16);
    fn CGEventSetIntegerValueField(event:CFTypeRef,field:u32,value:i64);
    fn CGEventSetFlags(event:CFTypeRef,flags:u64);
    fn CGEventPost(tap:u32,event:CFTypeRef);
    fn CGEventSourceKeyState(state:i32,key:u16)->bool;
    fn CGEventSourceButtonState(state:i32,button:u32)->bool;
}
static INPUT_LOCK:Mutex<()>=Mutex::new(());
fn invalid()->RegionError {RegionError::new("region_target_changed","目标窗口已改变或失去焦点，区域操作已停止")}
fn owned(raw:CFTypeRef)->Result<CFType> {
    if raw.is_null(){Err(invalid())}else{Ok(unsafe{CFType::wrap_under_create_rule(raw)})}
}
fn ax(raw:CFTypeRef)->Result<CFType> {
    let value=owned(raw)?;
    if unsafe{CFGetTypeID(value.as_CFTypeRef())}!=unsafe{AXUIElementGetTypeID()}{return Err(invalid())}
    unsafe{AXUIElementSetMessagingTimeout(value.as_CFTypeRef(),0.3);}
    Ok(value)
}
fn attr(element:&CFType,name:&str)->Result<CFType> {
    let key=CFString::new(name);let mut raw=std::ptr::null();
    if unsafe{AXUIElementCopyAttributeValue(element.as_CFTypeRef(),key.as_concrete_TypeRef(),&mut raw)}!=0{return Err(invalid())}
    owned(raw)
}
fn ax_attr(element:&CFType,name:&str)->Result<CFType> {
    let value=attr(element,name)?;
    if unsafe{CFGetTypeID(value.as_CFTypeRef())}!=unsafe{AXUIElementGetTypeID()}{return Err(invalid())}
    unsafe{AXUIElementSetMessagingTimeout(value.as_CFTypeRef(),0.3);}
    Ok(value)
}
fn pid(element:&CFType)->Result<i32> {
    let mut pid=0;
    if unsafe{AXUIElementGetPid(element.as_CFTypeRef(),&mut pid)}!=0||pid<=0||pid as u32==std::process::id(){return Err(invalid())}
    Ok(pid)
}
fn lifetime(pid:i32)->Result<(u64,u64)> {
    let mut info:libc::proc_bsdinfo=unsafe{std::mem::zeroed()};
    let size=std::mem::size_of_val(&info) as i32;
    if unsafe{libc::proc_pidinfo(pid,libc::PROC_PIDTBSDINFO,0,(&mut info as *mut libc::proc_bsdinfo).cast(),size)}!=size{return Err(invalid())}
    Ok((info.pbi_start_tvsec,info.pbi_start_tvusec))
}
fn bounds(element:&CFType)->Result<[f64;4]> {
    let position=attr(element,"AXPosition")?;let size=attr(element,"AXSize")?;
    let mut p=Point::default();let mut s=Size::default();
    unsafe {
        if CFGetTypeID(position.as_CFTypeRef())!=AXValueGetTypeID()||CFGetTypeID(size.as_CFTypeRef())!=AXValueGetTypeID()
            ||AXValueGetType(position.as_CFTypeRef())!=1||AXValueGetType(size.as_CFTypeRef())!=2
            ||!AXValueGetValue(position.as_CFTypeRef(),1,(&mut p as *mut Point).cast())
            ||!AXValueGetValue(size.as_CFTypeRef(),2,(&mut s as *mut Size).cast()){return Err(invalid())}
    }
    let result=[p.x,p.y,p.x+s.width,p.y+s.height];
    if !result.iter().all(|v|v.is_finite())||s.width<=0.||s.height<=0.{return Err(invalid())}
    Ok(result)
}
fn contains(outer:[f64;4],inner:[f64;4])->bool {
    inner[0]>=outer[0]&&inner[1]>=outer[1]&&inner[2]<=outer[2]&&inner[3]<=outer[3]
}
fn same(a:&CFType,b:&CFType)->bool{unsafe{CFEqual(a.as_CFTypeRef(),b.as_CFTypeRef())!=0}}
fn element_window(element:&CFType)->Result<CFType> {
    ax_attr(element,"AXWindow").or_else(|_| {
        let role=attr(element,"AXRole")?;
        if same(&role,&CFString::new("AXWindow").as_CFType()){Ok(element.clone())}else{Err(invalid())}
    })
}
fn hit(point:Point)->Result<CFType> {
    let system=ax(unsafe{AXUIElementCreateSystemWide()})?;let mut raw=std::ptr::null();
    if unsafe{AXUIElementCopyElementAtPosition(system.as_CFTypeRef(),point.x as f32,point.y as f32,&mut raw)}!=0{return Err(invalid())}
    ax(raw)
}
fn number(dict:CFDictionaryRef,key:CFStringRef)->Result<i64> {
    let raw=unsafe{CFDictionaryGetValue(dict,key.cast())};let mut n=0i64;
    if raw.is_null()||unsafe{CFGetTypeID(raw)}!=unsafe{CFNumberGetTypeID()}
        ||!unsafe{CFNumberGetValue(raw.cast(),kCFNumberSInt64Type,(&mut n as *mut i64).cast())}{return Err(invalid())}
    Ok(n)
}
fn window_id(pid:i32,bounds:[f64;4])->Result<u32> {
    let windows=owned(unsafe{CGWindowListCopyWindowInfo(1|16,0)}.cast())?;
    let array=windows.as_CFTypeRef().cast();let count=unsafe{CFArrayGetCount(array)};
    if !(0..=4096).contains(&count){return Err(invalid())}
    for i in 0..count {unsafe{
        let row=CFArrayGetValueAtIndex(array,i);
        if row.is_null()||CFGetTypeID(row)!=CFDictionaryGetTypeID(){return Err(invalid())}
        if number(row.cast(),kCGWindowLayer)?!=0||number(row.cast(),kCGWindowOwnerPID)?!=i64::from(pid){continue}
        let r=CFDictionaryGetValue(row.cast(),kCGWindowBounds.cast());let mut rect=Rect::default();
        if r.is_null()||CFGetTypeID(r)!=CFDictionaryGetTypeID()||!CGRectMakeWithDictionaryRepresentation(r.cast(),&mut rect){return Err(invalid())}
        if [rect.origin.x,rect.origin.y,rect.origin.x+rect.size.width,rect.origin.y+rect.size.height]==bounds {
            return u32::try_from(number(row.cast(),kCGWindowNumber)?).map_err(|_|invalid());
        }
    }}
    Err(invalid())
}
pub(super) struct Target {pid:i32,born:(u64,u64),window:u32,bounds:[f64;4],region:[f64;4]}
pub(super) fn dpi_scope()->Result<()> {
    if !unsafe{AXIsProcessTrusted()}{return Err(RegionError::new("region_accessibility_permission","请在系统设置中允许 MewuAI 使用辅助功能"))}
    Ok(())
}
pub(super) fn bind(v:Viewport,b:&RegionBinding)->Result<Target> {
    if b.coordinates.space!=CaptureCoordinateSpace::QuartzPoints{return Err(invalid())}
    let region=v.native_rect(b)?;
    let element=hit(Point{x:(region[0]+region[2])/2.,y:(region[1]+region[3])/2.})?;
    let pid=pid(&element)?;let window=element_window(&element)?;let bounds=bounds(&window)?;
    if !contains(bounds,region){return Err(RegionError::new("region_window_boundary","选区必须完整位于同一个应用窗口内"))}
    let target=Target{pid,born:lifetime(pid)?,window:window_id(pid,bounds)?,bounds,region};
    let application=ax(unsafe{AXUIElementCreateApplication(pid)})?;
    let key=CFString::new("AXFrontmost");
    unsafe{AXUIElementSetAttributeValue(application.as_CFTypeRef(),key.as_concrete_TypeRef(),core_foundation::boolean::CFBoolean::true_value().as_CFTypeRef());}
    validate(&target,v)?;Ok(target)
}
pub(super) fn description(_: &Target)->String {"macOS 应用窗口".into()}
fn focused_window(t:&Target)->Result<CFType> {
    if lifetime(t.pid)?!=t.born{return Err(invalid())}
    let system=ax(unsafe{AXUIElementCreateSystemWide()})?;
    let app=ax_attr(&system,"AXFocusedApplication")?;
    if pid(&app)?!=t.pid{return Err(invalid())}
    let window=ax_attr(&app,"AXFocusedWindow")?;
    if bounds(&window)?!=t.bounds||window_id(t.pid,t.bounds)?!=t.window{return Err(invalid())}
    Ok(window)
}
fn hit_target(t:&Target,point:Point)->Result<()> {
    if point.x<t.region[0]||point.x>=t.region[2]||point.y<t.region[1]||point.y>=t.region[3]{return Err(invalid())}
    let expected=focused_window(t)?;let actual=hit(point)?;
    if pid(&actual)?!=t.pid||!same(&element_window(&actual)?,&expected){return Err(invalid())}
    Ok(())
}
pub(super) fn validate(t:&Target,_:Viewport)->Result<()> {
    dpi_scope()?;hit_target(t,Point{x:(t.region[0]+t.region[2])/2.,y:(t.region[1]+t.region[3])/2.})
}
pub(super) fn neutral_input()->Result<()> {
    for key in [53,54,55,56,58,59,60,61,62] {if unsafe{CGEventSourceKeyState(1,key)}{return Err(RegionError::new("region_input_busy","请松开修饰键或停止键后重试"))}}
    if (0..3).any(|button|unsafe{CGEventSourceButtonState(1,button)}){return Err(RegionError::new("region_input_busy","请松开鼠标后重试"))}
    Ok(())
}
pub(super) fn stop_requested()->bool {unsafe{CGEventSourceKeyState(1,53)}}
fn live(s:&RegionSession)->Result<()> {
    s.ensure_live()?;if stop_requested(){return Err(RegionError::new("region_authority_revoked","区域操作已停止"))}
    s.hooks.validate_source(&s.info.binding).map_err(|e|RegionError::new("region_authority_revoked",e))?;
    validate(&s.target,s.viewport)
}
fn post(raw:CFTypeRef)->Result<()> {
    let event=owned(raw)?;unsafe{CGEventSetFlags(event.as_CFTypeRef(),0);CGEventPost(0,event.as_CFTypeRef());}Ok(())
}
fn mouse(s:&RegionSession,p:RegionPoint,kind:u32,count:i64)->Result<()> {
    let _guard=INPUT_LOCK.lock().map_err(|_|invalid())?;live(s)?;
    let (x,y)=s.viewport.native_point(&s.info.binding,p)?;let point=Point{x,y};hit_target(&s.target,point)?;
    let event=owned(unsafe{CGEventCreateMouseEvent(std::ptr::null(),kind,point,0)})?;
    unsafe{CGEventSetIntegerValueField(event.as_CFTypeRef(),1,count);CGEventSetFlags(event.as_CFTypeRef(),0);}
    if kind==1{s.button_down.store(true,Ordering::SeqCst);}
    unsafe{CGEventPost(0,event.as_CFTypeRef());}
    if kind==2{s.button_down.store(false,Ordering::SeqCst);}Ok(())
}
pub(super) fn release_button(down:&AtomicBool) {
    let _guard=INPUT_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    if down.swap(false,Ordering::SeqCst) {
        if let Ok(event)=owned(unsafe{CGEventCreate(std::ptr::null())}) {
            let point=unsafe{CGEventGetLocation(event.as_CFTypeRef())};
            let _=post(unsafe{CGEventCreateMouseEvent(std::ptr::null(),2,point,0)});
        }
    }
}
fn keyboard(s:&RegionSession,key:u16,text:Option<&str>)->Result<()> {
    let _guard=INPUT_LOCK.lock().map_err(|_|invalid())?;live(s)?;
    let app=ax(unsafe{AXUIElementCreateApplication(s.target.pid)})?;
    let focus=ax_attr(&app,"AXFocusedUIElement")?;
    if !contains(s.target.region,bounds(&focus)?)||!same(&element_window(&focus)?,&focused_window(&s.target)?){return Err(RegionError::new("region_focus_outside","输入焦点不在授权区域内"))}
    if let Ok(subrole)=attr(&focus,"AXSubrole") {if same(&subrole,&CFString::new("AXSecureTextField").as_CFType()){return Err(RegionError::new("region_secure_input","不能向密码框输入"))}}
    let down=owned(unsafe{CGEventCreateKeyboardEvent(std::ptr::null(),key,true)})?;
    let up=owned(unsafe{CGEventCreateKeyboardEvent(std::ptr::null(),key,false)})?;
    if let Some(text)=text {let units:Vec<u16>=text.encode_utf16().collect();unsafe{CGEventKeyboardSetUnicodeString(down.as_CFTypeRef(),units.len(),units.as_ptr());CGEventKeyboardSetUnicodeString(up.as_CFTypeRef(),units.len(),units.as_ptr());}}
    unsafe {CGEventSetFlags(down.as_CFTypeRef(),0);CGEventSetFlags(up.as_CFTypeRef(),0);CGEventPost(0,down.as_CFTypeRef());CGEventPost(0,up.as_CFTypeRef());}Ok(())
}
pub(super) fn execute(s:&RegionSession,action:RegionAction)->Result<()> {
    match action {
        RegionAction::Click{x,y}|RegionAction::DoubleClick{x,y}=>{
            let times=if matches!(action,RegionAction::DoubleClick{..}){2}else{1};
            for count in 1..=times {let p=RegionPoint{x,y};mouse(s,p,5,0)?;mouse(s,p,1,count)?;mouse(s,p,2,count)?;}
        }
        RegionAction::Drag{path}=>{
            mouse(s,path[0],5,0)?;mouse(s,path[0],1,1)?;
            for pair in path.windows(2) {
                let dx=i64::from(pair[1].x)-i64::from(pair[0].x);let dy=i64::from(pair[1].y)-i64::from(pair[0].y);
                let steps=((dx.abs().max(dy.abs())+7)/8).max(1);
                for n in 1..=steps {mouse(s,RegionPoint{x:(i64::from(pair[0].x)+dx*n/steps) as u32,y:(i64::from(pair[0].y)+dy*n/steps) as u32},6,1)?;std::thread::sleep(Duration::from_millis(4));}
            }
            mouse(s,*path.last().ok_or_else(invalid)?,2,1)?;
        }
        RegionAction::Scroll{x,y,delta_y}=>{
            mouse(s,RegionPoint{x,y},5,0)?;let _guard=INPUT_LOCK.lock().map_err(|_|invalid())?;live(s)?;
            let (x,y)=s.viewport.native_point(&s.info.binding,RegionPoint{x,y})?;hit_target(&s.target,Point{x,y})?;
            post(unsafe{CGEventCreateScrollWheelEvent(std::ptr::null(),0,1,delta_y)})?;
        }
        RegionAction::Type{text}=>{for ch in text.chars(){keyboard(s,0,Some(ch.encode_utf8(&mut [0;4])))?;}}
        RegionAction::Key{key}=>keyboard(s,match key{RegionKey::Enter=>36,RegionKey::Backspace=>51,RegionKey::Delete=>117,RegionKey::ArrowLeft=>123,RegionKey::ArrowRight=>124,RegionKey::ArrowUp=>126,RegionKey::ArrowDown=>125,RegionKey::Home=>115,RegionKey::End=>119},None)?,
    }Ok(())
}
pub(super) fn capture(s:&RegionSession)->Result<image::RgbaImage> {
    let b=&s.info.binding;
    let monitor=s.topology.iter().find(|m|f64::from(m.origin_x)==b.coordinates.origin_x&&f64::from(m.origin_y)==b.coordinates.origin_y).ok_or_else(invalid)?;
    let hooks=s.hooks.clone();let binding=b.clone();let created=s.created;
    let pixels=crate::capture_macos::capture(monitor.id,b.source_width,b.source_height,b.coordinates,&[],Arc::new(move||{
        if created.elapsed()>MAX_SESSION||stop_requested(){return Err("区域操作已停止".into())}hooks.validate_source(&binding)
    })).map_err(|e|RegionError::new("region_capture_failed",e))?;
    Ok(image::imageops::crop_imm(&pixels,b.selection.x.ceil() as u32,b.selection.y.ceil() as u32,s.viewport.width,s.viewport.height).to_image())
}
#[cfg(test)] pub(super) fn fixture_target()->Target {Target{pid:0,born:(0,0),window:0,bounds:[0.;4],region:[0.;4]}}
