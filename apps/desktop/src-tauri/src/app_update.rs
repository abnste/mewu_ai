// SPDX-License-Identifier: MPL-2.0
//! Fixed signed release channel. Only the first-party settings window can install.
//! Downloads finish and verify before entering the normal save/drain exit protocol.
use crate::Host;
use serde::Serialize;
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

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
struct Inner { status: Status, update: Option<Update> }
pub struct UpdateState { inner: Mutex<Inner>, busy: AtomicBool }
impl Default for UpdateState {
    fn default() -> Self {
        Self { inner: Mutex::new(Inner { status: Status { phase: Phase::Idle,
            current_version: env!("CARGO_PKG_VERSION").into(), available_version: None,
            downloaded_bytes: 0, total_bytes: None, error: None }, update: None }), busy: AtomicBool::new(false) }
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
    let current = semver::Version::parse(current).map_err(|_| "update_invalid")?;
    let candidate = semver::Version::parse(version).map_err(|_| "update_invalid")?;
    if candidate <= current || !candidate.build.is_empty() || candidate.major < 1 || version.len() > 64 {
        return Err("update_invalid");
    }
    let expected = format!("/abnste/mewu_ai/releases/download/v{version}/MewuAI-Remake-Setup-{version}-win-x64.exe");
    if url.scheme() != "https" || url.host_str() != Some("github.com") || url.port().is_some()
        || !url.username().is_empty() || url.password().is_some() || url.query().is_some()
        || url.fragment().is_some() || url.path() != expected { return Err("update_invalid"); }
    Ok(())
}
async fn check(app: &AppHandle) -> Result<Status, String> {
    app.state::<Host>().exit.ensure_new_operation()?;
    let state = app.state::<UpdateState>();
    let _permit = state.begin()?;
    state.set(Phase::Checking, None);
    let settings = app.state::<Host>().system_preferences.view().map_err(|_| "update_proxy");
    let result = async {
        let prefs = settings?;
        let mut builder = app.updater_builder().timeout(Duration::from_secs(25));
        match prefs.network_proxy_mode {
            crate::system_preferences::ProxyMode::System => {},
            crate::system_preferences::ProxyMode::Direct => { builder = builder.no_proxy(); },
            crate::system_preferences::ProxyMode::Custom => {
                builder = builder.proxy(url::Url::parse(&prefs.network_proxy_url).map_err(|_| "update_proxy")?);
            }
        }
        let updater = builder.build().map_err(|_| "update_invalid")?;
        let update = updater.check().await.map_err(|_| "update_check_failed")?;
        if let Some(value) = &update {
            validate_release(&value.current_version, &value.version, &value.download_url)?;
        }
        Ok::<_, &str>(update)
    }.await;
    match result {
        Ok(update) => {
            let mut inner = state.inner.lock().map_err(|_| "update_unavailable")?;
            inner.status.available_version = update.as_ref().map(|v| v.version.clone());
            inner.status.phase = if update.is_some() { Phase::Available } else { Phase::Current };
            inner.status.downloaded_bytes = 0; inner.status.total_bytes = None;
            inner.update = update;
        }
        Err(error) => state.set(Phase::Failed, Some(match error {
            "update_proxy" => "update_proxy", "update_invalid" => "update_invalid", _ => "update_check_failed",
        })),
    }
    state.status()
}
/// Created only after the updater verified the bytes and the signed version.
pub struct PreparedInstall { update: Update, bytes: Vec<u8> }
impl PreparedInstall {
    pub fn install(self, exit: &crate::lifecycle::ExitState) -> Result<(), String> {
        install_when_ready(exit.is_ready(), || self.update.install(self.bytes).map_err(|_| "update_install_failed".into()))
    }
}
fn install_when_ready(ready: bool, install: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if !ready { return Err("update_save_failed".into()); }
    install()
}
#[tauri::command]
pub fn app_update_status(window: WebviewWindow, app: AppHandle) -> Result<Status, String> {
    owner(&window)?; app.state::<UpdateState>().status()
}
#[tauri::command]
pub async fn check_app_update(window: WebviewWindow, app: AppHandle) -> Result<Status, String> {
    owner(&window)?; check(&app).await
}
#[tauri::command]
pub async fn install_app_update(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    owner(&window)?;
    app.state::<Host>().exit.ensure_new_operation()?;
    let state = app.state::<UpdateState>();
    let _permit = state.begin()?;
    let mut update = state.inner.lock().map_err(|_| "update_unavailable")?.update.clone().ok_or("update_not_available")?;
    validate_release(&update.current_version, &update.version, &update.download_url)?;
    update.timeout = Some(Duration::from_secs(300));
    state.set(Phase::Downloading, None);
    let received = Arc::new(AtomicU64::new(0));
    let oversized = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(tokio::sync::Notify::new());
    let (count, too_large, notify) = (received.clone(), oversized.clone(), cancel.clone());
    let download = update.download(|length, total| {
        let bytes = count.fetch_add(length as u64, Ordering::AcqRel).saturating_add(length as u64);
        if bytes > MAX_DOWNLOAD || total.is_some_and(|n| n > MAX_DOWNLOAD) {
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
        app.state::<Host>().exit.ensure_new_operation()?;
        state.set(Phase::Installing, None);
        crate::lifecycle::request_update_restart(&app, PreparedInstall { update, bytes }).await
    }.await;
    if result.is_err() { state.set(Phase::Failed, Some("update_install_failed")); }
    result
}
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
pub fn verify_cli(mut context: tauri::Context<tauri::Wry>, report: &std::path::Path) -> Result<(), String> {
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default().plugin(tauri_plugin_updater::Builder::new().build())
        .build(context).map_err(|_| "update_verify_start_failed")?;
    let result = tauri::async_runtime::block_on(async {
        // Verification may inspect the running version's own package. The
        // install path above always retains the strict newer-version comparator.
        let updater = app.handle().updater_builder().timeout(Duration::from_secs(25))
            .version_comparator(|_, _| true).build().map_err(|_| "update_invalid")?;
        let mut update = updater.check().await.map_err(|_| "update_check_failed")?.ok_or("update_not_available")?;
        validate_release("0.0.0", &update.version, &update.download_url)?;
        update.timeout = Some(Duration::from_secs(300));
        let oversized = AtomicBool::new(false);
        let count = AtomicU64::new(0);
        let cancel = tokio::sync::Notify::new();
        let download = update.download(|length, total| {
            let received = count.fetch_add(length as u64, Ordering::AcqRel) + length as u64;
            if received > MAX_DOWNLOAD || total.is_some_and(|n| n > MAX_DOWNLOAD) {
                oversized.store(true, Ordering::Release); cancel.notify_one();
            }
        }, || {});
        let bytes = tokio::select! { biased;
            _ = cancel.notified() => return Err("update_too_large"),
            bytes = download => bytes.map_err(|_| "update_download_failed")?,
        };
        if oversized.load(Ordering::Acquire) { return Err("update_too_large"); }
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_independent_release_allows_preview_to_stable_but_never_downgrades_or_another_repo() {
        for version in ["1.0.0-preview.2", "1.0.0", "1.1.0"] {
            let url = url::Url::parse(&format!("https://github.com/abnste/mewu_ai/releases/download/v{version}/MewuAI-Remake-Setup-{version}-win-x64.exe")).unwrap();
            assert!(validate_release("1.0.0-preview.1", version, &url).is_ok());
        }
        let valid = "https://github.com/abnste/mewu_ai/releases/download/v1.0.0/MewuAI-Remake-Setup-1.0.0-win-x64.exe";
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
    #[test]
    fn installer_is_not_called_if_saving_or_worker_drain_has_not_completed() {
        let called = AtomicBool::new(false);
        assert!(install_when_ready(false, || { called.store(true, Ordering::Release); Ok(()) }).is_err());
        assert!(!called.load(Ordering::Acquire));
        assert!(install_when_ready(true, || { called.store(true, Ordering::Release); Err("launch failed".into()) }).is_err());
        assert!(called.load(Ordering::Acquire));
    }
    #[test]
    fn update_permit_releases_on_failure() {
        let state = UpdateState::default();
        let permit = state.begin().unwrap(); assert!(state.begin().is_err());
        drop(permit); assert!(state.begin().is_ok());
    }
}
