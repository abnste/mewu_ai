// SPDX-License-Identifier: MPL-2.0
//! Bounded, display-only downloads. No installation or provider authority is
//! derived from this optional feed. Image jobs outlive a canceled IPC until the
//! actual HTTP/decode work has finished, so active entries are never evicted.
use crate::{extension_market as market, extension_project as project};
use base64::Engine as _;
use futures_util::StreamExt;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

const MAX_CACHED_IMAGES: usize = 4;
type ImageResult = Result<Arc<Vec<u8>>, String>;
type ImageReceiver = watch::Receiver<Option<ImageResult>>;

#[derive(Clone, PartialEq, Eq)]
struct ImageKey {
    url: String,
    sha256: String,
    size: u64,
    width: u32,
    height: u32,
}
struct CachedImage {
    generation: u64,
    key: ImageKey,
    result: ImageReceiver,
}
#[derive(Default)]
struct ImageCache {
    next: u64,
    entries: VecDeque<CachedImage>,
}
fn cache() -> &'static Mutex<ImageCache> {
    static CACHE: OnceLock<Mutex<ImageCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ImageCache::default()))
}
fn slots() -> &'static Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(2)))
}
fn acquire() -> Result<OwnedSemaphorePermit, String> {
    slots()
        .clone()
        .try_acquire_owned()
        .map_err(|_| "插件图库正在加载，请稍后重试".into())
}
fn client() -> Result<reqwest::Client, String> {
    crate::network_policy::apply(reqwest::Client::builder(), None)?
        .user_agent("Mewu/1.0 extension-projects")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        // The fixed raw URLs need no redirects. A renamed/deleted project must
        // be corrected in the trusted feed rather than silently changing origin.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建插件图库连接".into())
}
fn raw_url(url: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(url).map_err(|_| "插件图库来源无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("raw.githubusercontent.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("插件图库来源无效".into());
    }
    Ok(url)
}
async fn download(url: &str, limit: usize, optional: bool) -> Result<Option<Vec<u8>>, String> {
    let response = client()?
        .get(raw_url(url)?)
        .send()
        .await
        .map_err(|_| "无法连接 GitHub 插件图库")?;
    if optional && response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub 插件图库返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("插件图库下载内容过大".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(part) = stream.next().await {
        let part = part.map_err(|_| "插件图库下载中断")?;
        if bytes.len().saturating_add(part.len()) > limit {
            return Err("插件图库下载内容过大".into());
        }
        bytes.extend_from_slice(&part);
    }
    Ok(Some(bytes))
}

pub async fn get_project(entry: &market::Entry) -> Result<Option<project::Project>, String> {
    market::validate_entry(entry)?;
    let _slot = acquire()?;
    let Some(bytes) =
        download(project::DEFAULT_PROJECTS, project::MAX_PROJECTS_BYTES, true).await?
    else {
        return Ok(None);
    };
    let catalog = project::parse_projects(&bytes)?;
    Ok(project::find_project(&catalog, &entry.id, &entry.version, &entry.sha256).cloned())
}

fn cached_image(cache: &mut ImageCache, key: &ImageKey) -> Option<ImageReceiver> {
    let index = cache.entries.iter().position(|item| &item.key == key)?;
    let item = cache.entries.remove(index)?;
    let receiver = item.result.clone();
    cache.entries.push_back(item);
    Some(receiver)
}
fn reserve_image(
    cache: &mut ImageCache,
    key: ImageKey,
) -> Result<(u64, watch::Sender<Option<ImageResult>>, ImageReceiver), String> {
    if cache.entries.len() == MAX_CACHED_IMAGES {
        let ready = cache
            .entries
            .iter()
            .position(|item| item.result.borrow().is_some())
            .ok_or("插件图库正在加载，请稍后重试")?;
        cache.entries.remove(ready);
    }
    cache.next = cache.next.checked_add(1).ok_or("插件图库请求已过期")?;
    let generation = cache.next;
    let (sender, receiver) = watch::channel(None);
    cache.entries.push_back(CachedImage {
        generation,
        key,
        result: receiver.clone(),
    });
    Ok((generation, sender, receiver))
}

async fn image_bytes(project: &project::Project, index: usize) -> Result<Arc<Vec<u8>>, String> {
    let shot = project
        .screenshots
        .get(index)
        .ok_or("插件截图不存在")?
        .clone();
    let key = ImageKey {
        url: project::image_url(project, index)?,
        sha256: shot.sha256.clone(),
        size: shot.size,
        width: shot.width,
        height: shot.height,
    };
    let existing = {
        let mut state = cache().lock().map_err(|_| "插件图库缓存不可用")?;
        cached_image(&mut state, &key)
    };
    let mut receiver = if let Some(receiver) = existing {
        receiver
    } else {
        let slot = acquire()?;
        let mut state = cache().lock().map_err(|_| "插件图库缓存不可用")?;
        if let Some(receiver) = cached_image(&mut state, &key) {
            drop(slot);
            receiver
        } else {
            let (generation, sender, receiver) = reserve_image(&mut state, key.clone())?;
            drop(state);
            tauri::async_runtime::spawn(async move {
                let result: ImageResult = async {
                    let bytes = download(&key.url, project::MAX_SCREENSHOT_BYTES, false)
                        .await?
                        .ok_or("插件截图不存在")?;
                    // Keep the global slot through the actual blocking decoder,
                    // including after a caller has left the settings page.
                    tauri::async_runtime::spawn_blocking(move || {
                        let _slot = slot;
                        project::validate_image(&shot, &bytes)?;
                        Ok(Arc::new(bytes))
                    })
                    .await
                    .map_err(|_| "插件截图解码中断".to_string())?
                }
                .await;
                let failed = result.is_err();
                let _ = sender.send(Some(result));
                if failed {
                    if let Ok(mut state) = cache().lock() {
                        state.entries.retain(|entry| entry.generation != generation);
                    }
                }
            });
            receiver
        }
    };
    loop {
        if let Some(result) = receiver.borrow().as_ref() {
            return result.clone();
        }
        receiver.changed().await.map_err(|_| "插件图库请求中断")?;
    }
}

pub async fn get_image(
    entry: &market::Entry,
    index: usize,
    expected_source_commit: &str,
    expected_image_sha256: &str,
) -> Result<String, String> {
    let project = get_project(entry).await?.ok_or("插件项目资料暂不可用")?;
    let shot = project::image_for_snapshot(
        &project,
        index,
        expected_source_commit,
        expected_image_sha256,
    )?;
    let mime = project::image_mime(shot)?;
    let bytes = image_bytes(&project, index).await?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(id: &str) -> ImageKey {
        ImageKey {
            url: format!("https://raw.githubusercontent.com/a/b/c/{id}.png"),
            sha256: "a".repeat(64),
            size: 32,
            width: 16,
            height: 16,
        }
    }
    #[test]
    fn only_credential_free_raw_https_urls_are_downloaded() {
        assert!(raw_url(project::DEFAULT_PROJECTS).is_ok());
        for url in [
            "http://raw.githubusercontent.com/a",
            "https://raw.githubusercontent.com.evil.invalid/a",
            "https://user@raw.githubusercontent.com/a",
            "https://raw.githubusercontent.com/a?secret=x",
            "https://127.0.0.1/a",
        ] {
            assert!(raw_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn pending_images_are_shared_and_never_evicted_to_start_duplicate_work() {
        let mut state = ImageCache::default();
        let mut senders = Vec::new();
        for index in 0..MAX_CACHED_IMAGES {
            let (_, sender, _) = reserve_image(&mut state, key(&index.to_string())).unwrap();
            senders.push(sender);
        }
        assert!(cached_image(&mut state, &key("0")).is_some());
        assert_eq!(state.entries.len(), MAX_CACHED_IMAGES);
        assert!(reserve_image(&mut state, key("overflow")).is_err());
        senders[1].send(Some(Ok(Arc::new(vec![1, 2, 3])))).unwrap();
        reserve_image(&mut state, key("next")).unwrap();
        assert!(cached_image(&mut state, &key("1")).is_none());
        assert!(cached_image(&mut state, &key("0")).is_some());
        assert_eq!(state.entries.len(), MAX_CACHED_IMAGES);
    }
    #[test]
    fn backend_slots_bound_work_across_all_windows() {
        let a = acquire().unwrap();
        let b = acquire().unwrap();
        assert!(acquire().is_err());
        drop(a);
        assert!(acquire().is_ok());
        drop(b);
    }
    #[test]
    fn generated_gallery_acl_grants_only_the_local_settings_window() {
        use serde_json::{json, Value};
        // These artifacts are emitted by the real Tauri build script. A missing
        // AppManifest command cannot pass by merely editing capability JSON.
        let manifests: Value =
            serde_json::from_str(include_str!("../gen/schemas/acl-manifests.json")).unwrap();
        let capabilities: Value =
            serde_json::from_str(include_str!("../gen/schemas/capabilities.json")).unwrap();
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let active = config["app"]["security"]["capabilities"]
            .as_array()
            .unwrap();
        assert!(active.iter().any(|item| item == "settings"));
        assert!(manifests["__app-acl__"]["default_permission"].is_null());
        for command in ["get_extension_project", "get_extension_project_image"] {
            let allow = format!("allow-{}", command.replace('_', "-"));
            let deny = format!("deny-{}", command.replace('_', "-"));
            assert_eq!(
                manifests["__app-acl__"]["permissions"][&allow]["commands"]["allow"],
                json!([command])
            );
            assert_eq!(
                manifests["__app-acl__"]["permissions"][&deny]["commands"]["deny"],
                json!([command])
            );
            let mut holders = Vec::new();
            for (name, capability) in capabilities.as_object().unwrap() {
                if capability["permissions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|permission| {
                        permission
                            .as_str()
                            .or_else(|| permission["identifier"].as_str())
                            == Some(allow.as_str())
                    })
                {
                    holders.push(name.as_str());
                    assert_eq!(capability["windows"], json!(["settings"]));
                    assert_eq!(capability["local"], json!(true));
                    assert!(capability["remote"].is_null());
                }
            }
            assert_eq!(holders, ["settings"]);
        }
    }
}
