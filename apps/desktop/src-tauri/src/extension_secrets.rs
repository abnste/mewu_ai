// SPDX-License-Identifier: MPL-2.0
//! Plugin-scoped credentials. Workers receive status, never stored secret bytes.
//! Only a live settings surface can replace a credential; each immutable blob
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

pub(super) fn set(app: &AppHandle, context: &Context, params: Value) -> Result<Value, String> {
    if !context.surface_owner.as_ref().is_some_and(|(_, owner)| owner.scene.is_empty()) {
        return Err("请在插件设置界面修改凭据".into());
    }
    let key = name(&params)?;
    let expected = params.get("expectedRevision").and_then(Value::as_u64).ok_or("缺少凭据版本")?;
    let (old, reference) = read(app, context, key)?;
    if reference.as_ref().map_or(0, |r| r.revision) != expected { return Err("凭据已变化，请重新载入".into()); }
    let revision = expected.checked_add(1).ok_or("凭据版本已耗尽")?;
    let endpoint = origin(string(&params,"origin")?)?;
    let secret = Zeroizing::new(string(&params,"value")?.to_owned());
    crate::credentials::validate_key(&secret)?;
    let host = app.state::<Host>();
    let owner = owner(&context.record.manifest.id);
    let credential = if secret.is_empty() { None } else {
        Some(crate::credentials::create_scoped(&host.root,"extensions",&owner,&secret)?)
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
