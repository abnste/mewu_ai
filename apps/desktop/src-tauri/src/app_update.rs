// SPDX-License-Identifier: MPL-2.0
//! Fixed signed release channel. Only the first-party settings window can install.
//! Downloads finish and verify before entering the normal save/drain exit protocol.
#[path = "app_update_release.rs"]
mod release;
#[path = "app_update_github.rs"]
mod github;
pub use release::{validate_release_for, ReleaseTarget};
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use crate::Host;
#[cfg(target_os = "macos")]
#[path = "app_update_macos.rs"]
pub(crate) mod macos;
#[cfg(any(target_os = "linux", test))]
#[path = "app_update_appimage.rs"]
mod appimage;
#[cfg(target_os = "linux")]
#[path = "app_update_linux.rs"]
pub(crate) mod linux;
use serde::Serialize;
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use std::sync::{atomic::AtomicU64, Arc};
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use std::time::Duration;
use tauri::{AppHandle, WebviewWindow};
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use tauri::Manager;
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use tauri_plugin_updater::{Update, Updater, UpdaterExt};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Idle, Checking, Current, Available, Downloading, Installing, Failed }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    pub current_version: String,
    pub available_version: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<&'static str>,
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
#[derive(Clone)]
struct SelectedUpdate { native: Update, offer: github::Offer }
struct Inner { status: Status, #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))] update: Option<SelectedUpdate> }
pub struct UpdateState { inner: Mutex<Inner>, busy: AtomicBool }
impl Default for UpdateState {
    fn default() -> Self {
        Self { inner: Mutex::new(Inner { status: Status { phase: Phase::Idle,
            current_version: env!("CARGO_PKG_VERSION").into(), available_version: None,
            downloaded_bytes: 0, total_bytes: None, error: None }, #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))] update: None }), busy: AtomicBool::new(false) }
    }
}
struct Permit<'a>(&'a AtomicBool);
impl Drop for Permit<'_> { fn drop(&mut self) { self.0.store(false, Ordering::Release); } }
impl UpdateState {
    fn begin(&self) -> Result<Permit<'_>, String> {
        self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "update_busy".to_string())?;
        Ok(Permit(&self.busy))
    }
    fn status(&self) -> Result<Status, String> { self.inner.lock().map(|v| v.status.clone()).map_err(|_| "update_unavailable".into()) }
    fn set(&self, phase: Phase, error: Option<&'static str>) {
        if let Ok(mut inner) = self.inner.lock() { inner.status.phase = phase; inner.status.error = error; }
    }
}
fn owner(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "settings" { Ok(()) } else { Err("update_forbidden".into()) }
}
pub fn validate_release(current: &str, version: &str, url: &url::Url) -> Result<(), &'static str> {
    validate_release_for(ReleaseTarget::WindowsX64, current, version, url)
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn validate_native_release(current: &str, version: &str, url: &url::Url) -> Result<(), &'static str> {
    #[cfg(target_os = "windows")]
    let target = ReleaseTarget::WindowsX64;
    #[cfg(target_os = "macos")]
    let target = ReleaseTarget::MacArm64;
    #[cfg(target_os = "linux")]
    let target = ReleaseTarget::LinuxX64;
    validate_release_for(target, current, version, url)
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn check_error(error: &tauri_plugin_updater::Error) -> &'static str {
    use tauri_plugin_updater::Error;
    match error {
        // The SDK uses this for non-success HTTP responses without preserving
        // their status. Do not misreport it as "current" or assume it is 404.
        Error::ReleaseNotFound => "update_feed_unavailable",
        Error::Serialization(_) | Error::Semver(_) | Error::TargetNotFound(_)
        | Error::TargetsNotFound(_) => "update_invalid",
        Error::Reqwest(error) if error.is_timeout() => "update_timeout",
        Error::Reqwest(error) if error.is_decode() || error.is_builder() => "update_invalid",
        Error::Reqwest(_) => "update_network",
        _ => "update_check_failed",
    }
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn native_target() -> ReleaseTarget {
    #[cfg(target_os = "windows")]
    { ReleaseTarget::WindowsX64 }
    #[cfg(target_os = "macos")]
    { ReleaseTarget::MacArm64 }
    #[cfg(target_os = "linux")]
    { ReleaseTarget::LinuxX64 }
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn download_redirects(attempt: reqwest::redirect::Attempt<'_>) -> reqwest::redirect::Action {
    let url = attempt.url();
    if attempt.previous().len() >= 5 || url.scheme() != "https"
        || !url.username().is_empty() || url.password().is_some() || url.port().is_some()
        || !matches!(url.host_str(), Some("github.com" | "release-assets.githubusercontent.com" | "objects.githubusercontent.com")) {
        attempt.error("update_download_redirect")
    } else { attempt.follow() }
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
async fn check_release(updater: &Updater, client: &reqwest::Client, current: &str) -> Result<Option<SelectedUpdate>, &'static str> {
    let bytes = github::fetch(client).await?;
    let Some(offer) = github::select(&bytes, current, native_target())? else { return Ok(None) };
    let signature = github::signature(&offer)?;
    let data = serde_json::json!({"version":offer.version,"notes":offer.release_body,
        "platforms":{(native_target().manifest_key()):{"url":offer.url,"signature":signature}}});
    let update = updater.from_release_data(data).map_err(|error| check_error(&error))?.ok_or("update_invalid")?;
    validate_native_release(current, &update.version, &update.download_url)?;
    Ok(Some(SelectedUpdate { native: update, offer }))
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
async fn check(app: &AppHandle) -> Result<Status, String> {
    #[cfg(target_os = "linux")]
    let installation_path = linux::appimage_path(app).map_err(str::to_string)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let state = app.state::<UpdateState>();
    let _permit = state.begin()?;
    state.set(Phase::Checking, None);
    let settings = app.state::<Host>().system_preferences.view().map_err(|_| "update_proxy");
    let result = async {
        let prefs = settings?;
        #[cfg(target_os = "macos")]
        if !macos::supported() { return Err("update_install_location"); }
        let mut builder = app.updater_builder().timeout(Duration::from_secs(25))
            .configure_client(|client| client.redirect(reqwest::redirect::Policy::custom(download_redirects)));
        let mut client = reqwest::Client::builder().timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none());
        #[cfg(target_os = "linux")]
        { builder = builder.executable_path(installation_path); }
        match prefs.network_proxy_mode {
            crate::system_preferences::ProxyMode::System => {},
            crate::system_preferences::ProxyMode::Direct => { builder = builder.no_proxy(); client = client.no_proxy(); },
            crate::system_preferences::ProxyMode::Custom => {
                let proxy = url::Url::parse(&prefs.network_proxy_url).map_err(|_| "update_proxy")?;
                client = client.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(|_| "update_proxy")?);
                builder = builder.proxy(proxy);
            }
        }
        let updater = builder.build().map_err(|_| "update_invalid")?;
        let client = client.build().map_err(|_| "update_proxy")?;
        let update = check_release(&updater, &client, env!("CARGO_PKG_VERSION")).await?;
        Ok::<_, &str>(update)
    }.await;
    match result {
        Ok(update) => {
            let mut inner = state.inner.lock().map_err(|_| "update_unavailable")?;
            inner.status.available_version = update.as_ref().map(|v| v.native.version.clone());
            inner.status.phase = if update.is_some() { Phase::Available } else { Phase::Current };
            inner.status.downloaded_bytes = 0; inner.status.total_bytes = None;
            inner.update = update;
        }
        Err(error) => state.set(Phase::Failed, Some(match error {
            "update_proxy" => "update_proxy", "update_invalid" => "update_invalid",
            "update_feed_unavailable" => "update_feed_unavailable",
            "update_timeout" => "update_timeout", "update_network" => "update_network",
            "update_rate_limited" => "update_rate_limited",
            "update_install_location" => "update_install_location", _ => "update_check_failed",
        })),
    }
    state.status()
}
// Unconfigured platforms refuse before obtaining a host or starting IO.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
async fn check(app: impl FnOnce() -> AppHandle) -> Result<Status, String> {
    drop(app);
    Err("update_unsupported".into())
}
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
async fn install(app: impl FnOnce() -> AppHandle) -> Result<(), String> {
    drop(app);
    Err("update_unsupported".into())
}
/// Created only after the updater verified the bytes and the signed version.
pub struct PreparedInstall {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    update: Update,
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    bytes: Vec<u8>,
    #[cfg(target_os = "macos")]
    installation: macos::Installation,
    #[cfg(target_os = "linux")]
    installation: linux::Installation,
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    _private: (),
}
impl PreparedInstall {
    pub fn preflight(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        self.installation.check().map_err(str::to_string)?;
        #[cfg(target_os = "linux")]
        self.installation.check().map_err(str::to_string)?;
        Ok(())
    }
    pub fn install(self, exit: &crate::lifecycle::ExitState) -> Result<(), String> {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
        install_when_ready(exit.is_ready(), || {
            #[cfg(target_os = "macos")]
            self.installation.check().map_err(str::to_string)?;
            #[cfg(target_os = "linux")]
            self.installation.check().map_err(str::to_string)?;
            self.update.install(self.bytes).map_err(|_| "update_install_failed".into())
        })
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            let _ = exit;
            Err("update_unsupported".into())
        }
    }
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn install_when_ready(ready: bool, install: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if !ready { return Err("update_save_failed".into()); }
    install()
}
#[tauri::command]
pub fn app_update_status(window: WebviewWindow, app: AppHandle) -> Result<Status, String> {
    owner(&window)?;
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    { app.state::<UpdateState>().status() }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    { let _ = app; Err("update_unsupported".into()) }
}
#[tauri::command]
pub async fn check_app_update(window: WebviewWindow, app: AppHandle) -> Result<Status, String> {
    owner(&window)?;
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    { check(&app).await }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    { check(|| app).await }
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
#[tauri::command]
pub async fn install_app_update(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    owner(&window)?;
    #[cfg(target_os = "linux")]
    let installation_path = linux::appimage_path(&app).map_err(str::to_string)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let state = app.state::<UpdateState>();
    let _permit = state.begin()?;
    let selected = state.inner.lock().map_err(|_| "update_unavailable")?.update.clone().ok_or("update_not_available")?;
    let mut update = selected.native;
    validate_native_release(&update.current_version, &update.version, &update.download_url)?;
    #[cfg(target_os = "macos")]
    let installation = tauri::async_runtime::spawn_blocking(macos::Installation::capture).await
        .map_err(|_| "update_install_location")?.map_err(str::to_string)?;
    #[cfg(target_os = "linux")]
    let installation = tauri::async_runtime::spawn_blocking(move || linux::Installation::capture(installation_path)).await
        .map_err(|_| "update_appimage_location")?.map_err(str::to_string)?;
    update.timeout = Some(Duration::from_secs(300));
    state.set(Phase::Downloading, None);
    let received = Arc::new(AtomicU64::new(0));
    let oversized = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(tokio::sync::Notify::new());
    let (count, too_large, notify) = (received.clone(), oversized.clone(), cancel.clone());
    let expected_bytes = selected.offer.bytes;
    let download = update.download(|length, total| {
        let bytes = count.fetch_add(length as u64, Ordering::AcqRel).saturating_add(length as u64);
        if bytes > expected_bytes || bytes > MAX_DOWNLOAD || total.is_some_and(|n| n != expected_bytes || n > MAX_DOWNLOAD) {
            too_large.store(true, Ordering::Release); notify.notify_one();
        }
        if let Ok(mut inner) = state.inner.lock() {
            inner.status.downloaded_bytes = bytes; inner.status.total_bytes = total;
        }
    }, || {});
    let bytes = tokio::select! {
        biased;
        _ = cancel.notified() => Err("update_too_large"),
        value = download => value.map_err(|_| "update_download_failed"),
    };
    let result = async {
        let bytes = bytes.map_err(str::to_string)?;
        if oversized.load(Ordering::Acquire) || bytes.len() as u64 > MAX_DOWNLOAD { return Err("update_too_large".into()); }
        github::validate_download(&selected.offer, &bytes).map_err(str::to_string)?;
        #[cfg(target_os = "macos")]
        let (bytes, installation) = {
            let version = update.version.clone();
            tauri::async_runtime::spawn_blocking(move || {
                macos::validate_archive(&bytes, &version).map_err(str::to_string)?;
                installation.check().map_err(str::to_string)?;
                Ok::<_, String>((bytes, installation))
            }).await.map_err(|_| "update_invalid")??
        };
        app.state::<Host>().exit.ensure_new_operation()?;
        #[cfg(target_os = "linux")]
        let (bytes, installation) = tauri::async_runtime::spawn_blocking(move || {
            appimage::validate_bytes(&bytes).map_err(str::to_string)?;
            installation.check().map_err(str::to_string)?;
            Ok::<_, String>((bytes, installation))
        }).await.map_err(|_| "update_invalid")??;
        state.set(Phase::Installing, None);
        crate::lifecycle::request_update_restart(&app, PreparedInstall { update, bytes,
            #[cfg(any(target_os = "macos", target_os = "linux"))] installation }).await
    }.await;
    if result.is_err() { state.set(Phase::Failed, Some("update_install_failed")); }
    result
}
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
#[tauri::command]
pub async fn install_app_update(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    owner(&window)?;
    install(|| app).await
}
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn start_background_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let _ = check(&app).await;
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}
/// Release verification CLI. No windows, business database, credentials or
/// single-instance commands are initialized. This never installs a package.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn verify_cli(mut context: tauri::Context<tauri::Wry>, report: &std::path::Path) -> Result<(), String> {
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default().plugin(tauri_plugin_updater::Builder::new().build())
        .build(context).map_err(|_| "update_verify_start_failed")?;
    let result = tauri::async_runtime::block_on(async {
        // Verification may inspect the running version's own package. The
        // install path above always retains the strict newer-version comparator.
        let updater = app.handle().updater_builder().timeout(Duration::from_secs(25))
            .configure_client(|client| client.redirect(reqwest::redirect::Policy::custom(download_redirects)))
            .version_comparator(|_, _| true).build().map_err(|_| "update_invalid")?;
        let client = reqwest::Client::builder().timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "update_invalid")?;
        let selected = check_release(&updater, &client, "0.0.0").await?.ok_or("update_not_available")?;
        let mut update = selected.native;
        validate_native_release("0.0.0", &update.version, &update.download_url)?;
        update.timeout = Some(Duration::from_secs(300));
        let oversized = AtomicBool::new(false);
        let count = AtomicU64::new(0);
        let cancel = tokio::sync::Notify::new();
        let expected_bytes = selected.offer.bytes;
        let download = update.download(|length, total| {
            let received = count.fetch_add(length as u64, Ordering::AcqRel) + length as u64;
            if received > expected_bytes || received > MAX_DOWNLOAD || total.is_some_and(|n| n != expected_bytes || n > MAX_DOWNLOAD) {
                oversized.store(true, Ordering::Release); cancel.notify_one();
            }
        }, || {});
        let bytes = tokio::select! { biased;
            _ = cancel.notified() => return Err("update_too_large"),
            bytes = download => bytes.map_err(|_| "update_download_failed")?,
        };
        if oversized.load(Ordering::Acquire) { return Err("update_too_large"); }
        github::validate_download(&selected.offer, &bytes)?;
        #[cfg(target_os = "macos")]
        macos::validate_archive(&bytes, &update.version)?;
        #[cfg(target_os = "linux")]
        appimage::validate_bytes(&bytes)?;
        use sha2::Digest;
        Ok(serde_json::json!({ "currentVersion": env!("CARGO_PKG_VERSION"),
            "offeredVersion": update.version, "signatureVerified": true,
            "versionBound": true, "packageBytes": bytes.len(),
            "sha256": format!("{:x}", sha2::Sha256::digest(&bytes)), "installed": false }))
    });
    let value = result.map_err(str::to_string)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(report).map_err(|_| "update_report_exists")?;
    file.write_all(serde_json::to_string_pretty(&value).map_err(|_| "update_report_failed")?.as_bytes()).map_err(|_| "update_report_failed")?;
    file.sync_all().map_err(|_| "update_report_failed")?;
    Ok(())
}
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn verify_cli(_context: tauri::Context<tauri::Wry>, _report: &std::path::Path) -> Result<(), String> {
    Err("update_unsupported".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    #[test]
    fn check_errors_keep_unavailable_feed_distinct_from_invalid_manifest() {
        use tauri_plugin_updater::Error;
        assert_eq!(check_error(&Error::ReleaseNotFound), "update_feed_unavailable");
        let invalid = serde_json::from_str::<serde_json::Value>("not-json-private-value").unwrap_err();
        assert_eq!(check_error(&Error::Serialization(invalid)), "update_invalid");
        assert_eq!(check_error(&Error::TargetNotFound("private-target".into())), "update_invalid");
        assert_eq!(check_error(&Error::TargetsNotFound(vec!["private-target".into()])), "update_invalid");
        assert_eq!(check_error(&Error::EmptyEndpoints), "update_check_failed");
    }
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    #[tokio::test]
    async fn check_errors_classify_real_http_decode_timeout_and_connect_without_raw_details() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use tauri_plugin_updater::Error;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json").unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
        let error = client.get(format!("http://{address}/manifest")).send().await.unwrap()
            .json::<serde_json::Value>().await.unwrap_err();
        assert_eq!(check_error(&Error::Reqwest(error)), "update_invalid");
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let error = client.get(format!("http://{address}/private-query?token=private-value"))
            .timeout(Duration::from_millis(100)).send().await.unwrap_err();
        drop(listener);
        assert_eq!(check_error(&Error::Reqwest(error)), "update_timeout");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = client.get(format!("http://{address}/private-path")).send().await.unwrap_err();
        assert_eq!(check_error(&Error::Reqwest(error)), "update_network");
    }
    #[test]
    fn branch_independent_release_allows_preview_to_stable_but_never_downgrades_or_another_repo() {
        for version in ["1.0.0-preview.2", "1.0.0", "1.1.0"] {
            let url = url::Url::parse(&format!("https://github.com/abnste/mewu_ai/releases/download/v{version}/MewuAI-Setup-{version}-win-x64.exe")).unwrap();
            assert!(validate_release("1.0.0-preview.1", version, &url).is_ok());
        }
        let valid = "https://github.com/abnste/mewu_ai/releases/download/v1.0.0/MewuAI-Setup-1.0.0-win-x64.exe";
        for bad in [valid.replace("https:", "http:"), valid.replace("abnste/", "someone/"),
            valid.replace("github.com/", "github.com.evil/"), format!("{valid}?token=x"),
            valid.replace("v1.0.0/", "master/"), valid.replace("Setup-1.0.0", "Setup-0.7.4")] {
            assert!(validate_release("1.0.0-preview.1", "1.0.0", &url::Url::parse(&bad).unwrap()).is_err());
        }
        for old in ["0.7.4", "1.0.0-preview.1", "1.0.0-alpha.1"] {
            assert!(validate_release("1.0.0-preview.1", old, &url::Url::parse(valid).unwrap()).is_err());
        }
        assert!(validate_release("1.0.0", "1.0.0-preview.2", &url::Url::parse(valid).unwrap()).is_err());
    }
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    #[test]
    fn installer_is_not_called_if_saving_or_worker_drain_has_not_completed() {
        let called = AtomicBool::new(false);
        assert!(install_when_ready(false, || { called.store(true, Ordering::Release); Ok(()) }).is_err());
        assert!(!called.load(Ordering::Acquire));
        assert!(install_when_ready(true, || { called.store(true, Ordering::Release); Err("launch failed".into()) }).is_err());
        assert!(called.load(Ordering::Acquire));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    #[tokio::test]
    async fn unsupported_platform_check_and_install_never_obtain_host_or_start_io() {
        assert_eq!(check(|| panic!("unsupported check obtained a host")).await.err().as_deref(), Some("update_unsupported"));
        assert_eq!(install(|| panic!("unsupported install obtained a host")).await.err().as_deref(), Some("update_unsupported"));
    }
    #[test]
    fn update_permit_releases_on_failure() {
        let state = UpdateState::default();
        let permit = state.begin().unwrap(); assert!(state.begin().is_err());
        drop(permit); assert!(state.begin().is_ok());
    }
}
