// SPDX-License-Identifier: MPL-2.0
//! Plugin-scoped credentials. Runtime tools receive status, not stored bytes.
//! A separately permitted live settings editor can refill its own password
//! field. Only a live settings surface can replace a credential; each immutable blob
//! is committed with CAS. Network use is restricted to its configured origin.
use super::*;
use zeroize::Zeroizing;

const SCOPE: &str = "plugin-secrets";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reference {
    revision: u64,
    origin: String,
    credential: Option<String>,
}

fn name(params: &Value) -> Result<&str, String> {
    let name = string(params, "key")?;
    if name.is_empty() || name.len() > 80
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
        return Err("凭据名称无效".into());
    }
    Ok(name)
}

fn owner(plugin: &str) -> String {
    let digest = Sha256::digest(format!("mewu.extension.credentials/{plugin}"));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}

fn origin(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|_| "凭据服务地址无效")?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && crate::network_policy::is_local(&url)))
        || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some() || url.query().is_some() {
        return Err("凭据服务需要 HTTPS 或本机 HTTP 地址".into());
    }
    Ok(url.origin().ascii_serialization())
}

fn read(app: &AppHandle, context: &Context, key: &str) -> Result<(Value, Option<Reference>), String> {
    let value = live_store(app, context, |store| store.value(&context.record.manifest.id, SCOPE, key))?;
    let reference = if value.is_null() { None } else {
        Some(serde_json::from_value(value.clone()).map_err(|_| "插件凭据记录无效")?)
    };
    Ok((value, reference))
}

pub(super) fn status(app: &AppHandle, context: &Context, params: &Value) -> Result<Value, String> {
    let (_, reference) = read(app, context, name(params)?)?;
    Ok(match reference {
        Some(r) => json!({"revision":r.revision,"configured":r.credential.is_some(),"origin":r.origin}),
        None => json!({"revision":0,"configured":false,"origin":null}),
    })
}

fn password_editor_owner(plugin: &str, revision: u64, window: bool, owner: Option<&crate::extension_surfaces::Owner>) -> Result<(), String> {
    if !window || !owner.is_some_and(|owner|
        owner.scene.is_empty() && owner.plugin == plugin && owner.revision == revision) {
        return Err("请在插件设置界面读取凭据".into());
    }
    Ok(())
}

pub(super) fn reveal(app: &AppHandle, context: &Context, params: &Value) -> Result<Value, String> {
    password_editor_owner(&context.record.manifest.id, context.record.revision,
        context.surface_window.is_some(), context.surface_owner.as_ref().map(|(_, owner)| owner))?;
    context.check(app)?;
    let key = name(params)?;
    let expected = params.get("expectedRevision").and_then(Value::as_u64).ok_or("缺少凭据版本")?;
    let (before, reference) = read(app, context, key)?;
    if reference.as_ref().map_or(0, |r| r.revision) != expected { return Err("凭据已变化，请重新载入".into()); }
    let Some(id) = reference.and_then(|r| r.credential) else { return Ok(Value::String(String::new())); };
    let host = app.state::<Host>();
    let secret = crate::credentials::read_scoped(&host.root, "extensions", &owner(&context.record.manifest.id), &id)?;
    context.check(app)?;
    let (after, _) = read(app, context, key)?;
    if before != after { return Err("凭据已变化，请重新载入".into()); }
    Ok(Value::String(secret.to_string()))
}

pub(super) fn set(app: &AppHandle, context: &Context, params: Value) -> Result<Value, String> {
    if !context.surface_owner.as_ref().is_some_and(|(_, owner)| owner.scene.is_empty()) {
        return Err("请在插件设置界面修改凭据".into());
    }
    let key = name(&params)?;
    let expected = params.get("expectedRevision").and_then(Value::as_u64).ok_or("缺少凭据版本")?;
    let endpoint = origin(string(&params,"origin")?)?;
    let secret = Zeroizing::new(string(&params,"value")?.to_owned());
    commit(app, context, key, expected, endpoint, &secret)
}

/// Native archive imports use the same CAS/origin boundary without ever
/// serializing the old credential into plugin input or a renderer response.
pub(super) fn import_archive(app: &AppHandle, context: &Context, route: &mewu_core::MemoryRoute, expected: u64) -> Result<Value,String> {
    context.permission("secrets.write")?;
    let host=app.state::<Host>();
    let secret=crate::credentials::read_memory(&host.root,&route.binding_id,route.credential_id.as_deref())?;
    commit(app,context,&format!("archive.{}",route.binding_id),expected,origin(&route.endpoint)?,&secret)
}

fn commit(app:&AppHandle,context:&Context,key:&str,expected:u64,endpoint:String,secret:&str)->Result<Value,String> {
    let (old, reference) = read(app, context, key)?;
    if reference.as_ref().map_or(0, |r| r.revision) != expected { return Err("凭据已变化，请重新载入".into()); }
    let revision = expected.checked_add(1).ok_or("凭据版本已耗尽")?;
    crate::credentials::validate_key(secret)?;
    let host = app.state::<Host>();
    let owner = owner(&context.record.manifest.id);
    let credential = if secret.is_empty() { None } else {
        Some(crate::credentials::create_scoped(&host.root,"extensions",&owner,secret)?)
    };
    let next = Reference { revision, origin: endpoint, credential: credential.clone() };
    let commit = context.check(app).and_then(|()| live_store(app,context,|store|
        store.compare_and_set(&context.record.manifest.id,SCOPE,key,&old,&serde_json::to_value(&next).map_err(|_|"凭据记录无效")?)
    ));
    if !matches!(commit,Ok(true)) {
        if let Some(id) = &credential { crate::credentials::discard_scoped(&host.root,"extensions",&owner,id); }
        return Err(commit.err().unwrap_or_else(||"凭据已变化，请重新载入".into()));
    }
    // Prior immutable blobs can still belong to an in-flight request; do not
    // delete them during replacement. A cleared reference cannot read them.
    Ok(json!({"revision":next.revision,"configured":next.credential.is_some(),"origin":next.origin}))
}

pub(super) fn authorization(app: &AppHandle, context: &Context, params: &Value, url: &url::Url)
    -> Result<reqwest::header::HeaderValue,String> {
    context.permission("secrets.use")?;
    let (_, reference) = read(app,context,name(params)?)?;
    let reference = reference.ok_or("插件凭据未配置")?;
    if reference.origin != url.origin().ascii_serialization() { return Err("凭据不能发送到其他服务".into()); }
    let id = reference.credential.ok_or("插件凭据未配置")?;
    let host=app.state::<Host>();
    let secret=crate::credentials::read_scoped(&host.root,"extensions",&owner(&context.record.manifest.id),&id)?;
    let prefix=params.get("prefix").and_then(Value::as_str).unwrap_or("Bearer ");
    if !["Bearer ","Token ",""].contains(&prefix) { return Err("凭据认证方式无效".into()); }
    let value=Zeroizing::new(format!("{prefix}{}",secret.as_str()));
    let mut header=reqwest::header::HeaderValue::from_str(&value).map_err(|_|"凭据格式无效")?;
    header.set_sensitive(true);
    context.check(app)?;
    Ok(header)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn only_the_exact_plugins_settings_editor_can_read_a_password() {
        let owner = crate::extension_surfaces::Owner { plugin: "own.plugin".into(), revision: 3, service: "settings".into(), scene: String::new() };
        assert!(password_editor_owner("own.plugin", 3, true, Some(&owner)).is_ok());
        assert!(password_editor_owner("own.plugin", 3, true, None).is_err());
        assert!(password_editor_owner("own.plugin", 3, false, Some(&owner)).is_err());
        assert!(password_editor_owner("other.plugin", 3, true, Some(&owner)).is_err());
        assert!(password_editor_owner("own.plugin", 4, true, Some(&owner)).is_err());
        let object = crate::extension_surfaces::Owner { scene: "scene".into(), ..owner };
        assert!(password_editor_owner("own.plugin", 3, true, Some(&object)).is_err());
    }
    #[test] fn owners_are_stable_and_distinct_without_paths() {
        assert_eq!(owner("p"),owner("p")); assert_ne!(owner("p"),owner("p2"));
        assert!(uuid::Uuid::parse_str(&owner("p")).is_ok());
        assert!(name(&json!({"key":"../../memory"})).is_err());
    }
    #[test] fn credential_destinations_are_exact_origins() {
        assert_eq!(origin("https://EXAMPLE.com/base/").unwrap(),"https://example.com");
        assert_ne!(origin("http://127.0.0.1:8888/").unwrap(),origin("http://127.0.0.1:8889/").unwrap());
        for value in ["http://example.com","https://user:pass@example.com","https://example.com/?key=x","file:///etc/passwd"] { assert!(origin(value).is_err()); }
    }
}
