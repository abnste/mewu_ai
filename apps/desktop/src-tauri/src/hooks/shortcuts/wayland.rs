// SPDX-License-Identifier: MPL-2.0
//! GlobalShortcuts sessions use stable rule IDs. The compositor chooses actual
//! bindings; preferred_trigger is a hint and never reported as an active key.
use super::*;
use futures_util::StreamExt;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value as Variant},
    Connection, Proxy,
};
type Options<'a> = HashMap<&'a str, Variant<'a>>;
type Results = HashMap<String, OwnedValue>;
type Bindings = Vec<(String, Results)>;
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const SESSION: &str = "org.freedesktop.portal.Session";
const REQUEST: &str = "org.freedesktop.portal.Request";
const ERROR: &str = "系统全局快捷键不可用";
fn token() -> String {
    format!("mewu_{}", uuid::Uuid::new_v4().simple())
}
fn allowed(r: &Shared, epoch: u64) -> bool {
    require(r).is_ok()
        && !r.shortcuts.portal_fault.load(Ordering::Acquire)
        && r.shortcuts.portal_epoch.load(Ordering::Acquire) == epoch
}
pub(super) fn restart(r: Arc<Shared>) {
    if r.shortcuts.portal_fault.load(Ordering::Acquire) {
        if let Ok(rules) = lock(&r.store).and_then(|store| store.rules()) {
            if let Ok(mut statuses) = r.shortcuts.status.lock() {
                for rule in rules
                    .into_iter()
                    .filter(|r| r.enabled && matches!(r.trigger, Trigger::Shortcut { .. }))
                {
                    statuses.insert(
                        rule.id,
                        Status {
                            status: "cleanupPending",
                            message: Some("系统快捷键清理尚未确认，请重启软件".into()),
                            label: None,
                        },
                    );
                }
            }
        }
        changed(&r);
        return;
    }
    let epoch = r.shortcuts.portal_epoch.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn(async move {
        let _serial = r.shortcuts.portal_serial.lock().await;
        if !allowed(&r, epoch) {
            return;
        }
        let rules = lock(&r.store)
            .and_then(|store| store.rules())
            .unwrap_or_default()
            .into_iter()
            .filter(|rule| rule.enabled && matches!(rule.trigger, Trigger::Shortcut { .. }))
            .collect::<Vec<_>>();
        if rules.is_empty() {
            return;
        }
        publish(&r, epoch, &rules, &[], Some("等待系统快捷键授权"));
        if let Err(error) = serve(r.clone(), epoch, &rules).await {
            publish(&r, epoch, &rules, &[], Some(&error));
        }
    });
}
async fn checked<T, E>(
    future: impl std::future::Future<Output = std::result::Result<T, E>>,
    until: Instant,
    gate: &impl Fn() -> bool,
) -> Result<T> {
    tokio::pin!(future);
    loop {
        if !gate() {
            return Err("快捷键设置已取消".into());
        }
        if Instant::now() >= until {
            return Err("系统快捷键授权超时".into());
        }
        tokio::select! {result=&mut future=>{if !gate(){return Err("快捷键设置已取消".into())}return result.map_err(|_|ERROR.into())},_=tokio::time::sleep(Duration::from_millis(20))=>{}}
    }
}
struct Portal {
    connection: Connection,
    owner: String,
    path: OwnedObjectPath,
    pending: Option<String>,
}
impl Portal {
    async fn request<B: serde::Serialize + zbus::zvariant::DynamicType>(
        &mut self,
        method: &str,
        handle: &str,
        body: &B,
        until: Instant,
        gate: &impl Fn() -> bool,
    ) -> Result<Results> {
        let sender = self
            .connection
            .unique_name()
            .ok_or(ERROR)?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{handle}");
        let proxy = checked(
            Proxy::new(
                &self.connection,
                self.owner.as_str(),
                path.as_str(),
                REQUEST,
            ),
            until.min(Instant::now() + Duration::from_secs(5)),
            gate,
        )
        .await?;
        let mut responses = checked(
            proxy.receive_signal("Response"),
            until.min(Instant::now() + Duration::from_secs(5)),
            gate,
        )
        .await?;
        self.pending = Some(path.clone());
        let desktop = checked(
            Proxy::new(&self.connection, self.owner.as_str(), PATH, IFACE),
            until.min(Instant::now() + Duration::from_secs(5)),
            gate,
        )
        .await?;
        let returned: OwnedObjectPath = checked(
            desktop.call(method, body),
            until.min(Instant::now() + Duration::from_secs(5)),
            gate,
        )
        .await?;
        if returned.as_str() != path {
            return Err("系统快捷键请求身份不匹配".into());
        }
        let signal = checked(async { responses.next().await.ok_or(ERROR) }, until, gate).await?;
        if signal.data().bytes().len() > 256 * 1024 {
            return Err("系统快捷键响应过大".into());
        }
        let (code, results): (u32, Results) = signal.body().deserialize().map_err(|_| ERROR)?;
        self.pending = None;
        match code {
            0 => Ok(results),
            1 => Err("已取消系统快捷键设置".into()),
            _ => Err("系统未绑定快捷键".into()),
        }
    }
    async fn close(&mut self) -> Result<()> {
        // GlobalShortcuts requires Request.Close before Session.Close, unlike
        // ScreenCast. A failed close blocks replacement to avoid duplicate grabs.
        if let Some(path) = &self.pending {
            close(&self.connection, &self.owner, path, REQUEST).await?;
            self.pending = None;
        }
        close(&self.connection, &self.owner, self.path.as_str(), SESSION).await
    }
}
async fn close(connection: &Connection, owner: &str, path: &str, interface: &str) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let proxy = Proxy::new(connection, owner, path, interface).await?;
        proxy.call::<_, _, ()>("Close", &()).await
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(zbus::Error::MethodError(name, _, _)))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownObject" =>
        {
            Ok(())
        }
        _ => Err("系统快捷键清理尚未确认".into()),
    }
}
async fn serve(r: Arc<Shared>, epoch: u64, rules: &[Rule]) -> Result<()> {
    let settings = r
        .app
        .get_webview_window("settings")
        .filter(|w| w.is_visible().unwrap_or(false));
    let parent = if let Some(window) = settings.as_ref() {
        Some(crate::capture_shortcut_wayland_host::settings_parent(window).await?)
    } else {
        None
    };
    let gate = || allowed(&r, epoch);
    let configuring = || gate() && parent.as_ref().is_none_or(|p| p.check_mapped().is_ok());
    crate::capture_shortcut_desktop_linux::ensure(
        &r.app.state::<crate::Host>().root,
        parent.is_some(),
        gate,
    )?;
    let until = Instant::now() + Duration::from_secs(120);
    let (connection, owner) = crate::capture_shortcut_portal_linux::registered_connection(
        zbus::Address::session().map_err(|_| ERROR)?,
        tokio::time::Instant::now() + Duration::from_secs(5),
        &gate,
    )
    .await?;
    let session_token = token();
    let sender = connection
        .unique_name()
        .ok_or(ERROR)?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = OwnedObjectPath::try_from(format!(
        "/org/freedesktop/portal/desktop/session/{sender}/{session_token}"
    ))
    .map_err(|_| ERROR)?;
    let mut portal = Portal {
        connection,
        owner,
        path,
        pending: None,
    };
    let result=async{
        // Pin service/session before Create/Bind, including early activation.
        let bus=checked(Proxy::new_owned(portal.connection.clone(),"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus"),until,&gate).await?;
        let mut owners=checked(bus.receive_signal_with_args("NameOwnerChanged",&[(0,"org.freedesktop.portal.Desktop")]),until,&gate).await?;
        let globals=checked(Proxy::new_owned(portal.connection.clone(),portal.owner.clone(),PATH,IFACE),until,&gate).await?;
        let mut activated=checked(globals.receive_signal("Activated"),until,&gate).await?;
        let mut deactivated=checked(globals.receive_signal("Deactivated"),until,&gate).await?;
        let mut changes=checked(globals.receive_signal("ShortcutsChanged"),until,&gate).await?;
        let session=checked(Proxy::new_owned(portal.connection.clone(),portal.owner.clone(),portal.path.clone(),SESSION),until,&gate).await?;
        let mut closed=checked(session.receive_signal("Closed"),until,&gate).await?;
        let handle=token();let mut options=Options::new();options.insert("handle_token",Variant::from(handle.as_str()));options.insert("session_handle_token",Variant::from(session_token.as_str()));
        let response=portal.request("CreateSession",&handle,&(options,),until,&gate).await?;
        if response.get("session_handle").and_then(|v|<&str>::try_from(v).ok())!=Some(portal.path.as_str()){return Err("系统快捷键会话身份不匹配".into())}
        let handle=token();let mut options=Options::new();options.insert("handle_token",Variant::from(handle.as_str()));
        let mut bindings=vec![];for rule in rules{let mut properties=Options::new();properties.insert("description",Variant::from(rule.name.as_str()));let Trigger::Shortcut{accelerator}=&rule.trigger else{unreachable!()};properties.insert("preferred_trigger",Variant::from(preferred(accelerator)?));bindings.push((rule.id.as_str(),properties));}
        let path=portal.path.clone();let mut response=portal.request("BindShortcuts",&handle,&(path,bindings,parent.as_ref().map(|p|p.identifier()).unwrap_or(""),options),until,&configuring).await?;
        let actual:Bindings=response.remove("shortcuts").ok_or(ERROR)?.try_into().map_err(|_|ERROR)?;
        publish(&r,epoch,rules,&actual,None);let mut bound=actual_ids(&actual);let mut pressed=HashSet::new();
        while gate(){tokio::select!{biased;
            message=owners.next()=>{if message.is_none_or(|m|m.body().deserialize::<(String,String,String)>().is_ok_and(|(_,old,new)|old==portal.owner&&new!=old)){return Err(ERROR.into())}},
            _=closed.next()=>return Err("系统快捷键会话已结束".into()),
            message=deactivated.next()=>{let Some(message)=message else{return Err(ERROR.into())};if let Ok((path,id,_,_))=message.body().deserialize::<(OwnedObjectPath,String,u64,Results)>(){if path==portal.path{pressed.remove(&id);}}},
            message=activated.next()=>{let Some(message)=message else{return Err(ERROR.into())};if let Ok((path,id,_,_))=message.body().deserialize::<(OwnedObjectPath,String,u64,Results)>(){if path==portal.path&&bound.contains(&id)&&pressed.insert(id.clone())&&gate()&&!r.input_active.load(Ordering::Acquire)&&!super::recording(&r,None){if let Some(rule)=rules.iter().find(|rule|rule.id==id){let mut event=Event::new("shortcut.pressed");event.payload=json!({"ruleId":rule.id,"ruleRevision":rule.revision});emit(&r.app,event);}}}},
            message=changes.next()=>{let Some(message)=message else{return Err(ERROR.into())};if let Ok((path,actual))=message.body().deserialize::<(OwnedObjectPath,Bindings)>(){if path==portal.path{bound=actual_ids(&actual);publish(&r,epoch,rules,&actual,None);}}},
            _=tokio::time::sleep(Duration::from_millis(25))=>{}
        }}Ok(())
    }.await;
    let cleanup = portal.close().await;
    if cleanup.is_err() {
        // Fail closed; no new session until process restart.
        r.shortcuts.portal_fault.store(true, Ordering::Release);
        for rule in rules {
            if let Ok(mut statuses) = r.shortcuts.status.lock() {
                statuses.insert(
                    rule.id.clone(),
                    Status {
                        status: "cleanupPending",
                        message: Some("系统快捷键清理尚未确认，请重启软件".into()),
                        label: None,
                    },
                );
            }
        }
        changed(&r);
    }
    cleanup?;
    result
}
fn actual_ids(bindings: &Bindings) -> HashSet<String> {
    bindings
        .iter()
        .filter(|(_, properties)| label(properties).is_some())
        .map(|(id, _)| id.clone())
        .collect()
}
fn label(properties: &Results) -> Option<String> {
    properties
        .get("trigger_description")
        .and_then(|v| <&str>::try_from(v).ok())
        .filter(|s| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
fn publish(r: &Shared, epoch: u64, rules: &[Rule], bindings: &Bindings, error: Option<&str>) {
    if !allowed(r, epoch) {
        return;
    }
    if let Ok(mut states) = r.shortcuts.status.lock() {
        for rule in rules {
            let label = bindings
                .iter()
                .find(|(id, _)| *id == rule.id)
                .and_then(|(_, p)| label(p));
            states.insert(
                rule.id.clone(),
                Status {
                    status: if label.is_some() {
                        "active"
                    } else {
                        "needsSetup"
                    },
                    message: label
                        .is_none()
                        .then(|| error.unwrap_or("系统未设置此快捷键").to_owned()),
                    label,
                },
            );
        }
    }
    changed(r);
}
fn preferred(accelerator: &str) -> Result<String> {
    let key = parse(accelerator)?;
    let mut parts = vec![];
    if key.mods.contains(Modifiers::CONTROL) {
        parts.push("CTRL");
    }
    if key.mods.contains(Modifiers::ALT) {
        parts.push("ALT");
    }
    if key.mods.contains(Modifiers::SHIFT) {
        parts.push("SHIFT");
    }
    if key.mods.contains(Modifiers::SUPER) {
        parts.push("LOGO");
    }
    let code = key.key.to_string();
    let symbol = match code.as_str() {
        "Space" => "space",
        "Enter" => "Return",
        "Backspace" => "BackSpace",
        "PrintScreen" => "Print",
        "Backquote" => "grave",
        "Minus" => "minus",
        "Equal" => "equal",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backslash" => "backslash",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        s => s
            .strip_prefix("Key")
            .or_else(|| s.strip_prefix("Digit"))
            .or_else(|| s.strip_prefix("Arrow"))
            .unwrap_or(s),
    };
    parts.push(symbol);
    Ok(parts.join("+"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferred_is_only_an_xdg_hint_and_actual_label_is_required() {
        assert_eq!(preferred("Control+Shift+KeyK").unwrap(), "CTRL+SHIFT+K");
        let mut p = Results::new();
        p.insert(
            "preferred_trigger".into(),
            OwnedValue::from(zbus::zvariant::Str::from("CTRL+K")),
        );
        assert!(label(&p).is_none());
        assert!(actual_ids(&vec![("a".into(), p.clone())]).is_empty());
        p.insert(
            "trigger_description".into(),
            OwnedValue::from(zbus::zvariant::Str::from("Ctrl + K")),
        );
        assert_eq!(label(&p).as_deref(), Some("Ctrl + K"));
    }
}
