// SPDX-License-Identifier: MPL-2.0
//! Capture cancellation keeps the real worker and space-transition lifetime intact.
//! An abandoned invoke only signals cancel: actual blocking capture keeps Work.
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;

pub(crate) const CANCELED: &str = "截图已取消";
struct Active {
    id: u64,
    canceled: Arc<AtomicBool>,
    wake: Arc<Notify>,
}
#[derive(Default)]
pub(crate) struct CaptureJobs {
    next: AtomicU64,
    active: Mutex<Option<Active>>,
    idle: Notify,
}
impl CaptureJobs {
    /// Parent obtains an owned SpaceTransition before this method and keeps it
    /// through actual worker completion + source adoption + final display decision.
    /// None means another owner exists: repeated shortcut is silently ignored.
    pub(crate) fn begin(self: &Arc<Self>) -> Result<Option<Work>, String> {
        let mut active = self.active.lock().map_err(|_| "截图状态不可用")?;
        if active.is_some() {
            return Ok(None);
        }
        let id = self
            .next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .map_err(|_| "截图服务需要重新启动")?
            + 1;
        let canceled = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        *active = Some(Active {
            id,
            canceled: canceled.clone(),
            wake: wake.clone(),
        });
        Ok(Some(Work {
            id,
            registry: self.clone(),
            canceled,
            wake,
        }))
    }

    /// Invoke before hide/Esc and as soon as exit enters Preparing, even if exit
    /// is subsequently aborted. Restored app admits a new snapshot, never revives this one.
    pub(crate) fn cancel(&self) {
        if let Ok(active) = self.active.lock() {
            if let Some(active) = active.as_ref() {
                active.canceled.store(true, Ordering::Release);
                active.wake.notify_waiters();
            }
        }
    }

    pub(crate) async fn drain(&self, timeout: Duration) -> Result<(), String> {
        let until = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            // enable before checking state to avoid a completion/waiter race.
            notified.as_mut().enable();
            if self.active.lock().map_err(|_| "截图状态不可用")?.is_none() {
                return Ok(());
            }
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep_until(until) => return Err("截图任务尚未结束".into()),
            }
        }
    }
}
pub(crate) struct Work {
    id: u64,
    registry: Arc<CaptureJobs>,
    canceled: Arc<AtomicBool>,
    wake: Arc<Notify>,
}
impl Work {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.canceled.load(Ordering::Acquire) {
            Err(CANCELED.into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn cancel_on_drop(&self) -> CancelOnDrop {
        CancelOnDrop {
            canceled: self.canceled.clone(),
            wake: self.wake.clone(),
        }
    }
    /// After hiding own windows, wait without blocking a UI/native thread.
    /// Snapshot delay once before hiding. Settings changes affect the next capture.
    pub(crate) async fn wait_delay(
        &self,
        seconds: u8,
        allowed: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        if !matches!(seconds, 0 | 3 | 5) {
            return Err("截图延迟无效".into());
        }
        let until = tokio::time::Instant::now() + Duration::from_secs(u64::from(seconds));
        loop {
            let canceled = self.wake.notified();
            tokio::pin!(canceled);
            canceled.as_mut().enable();
            self.check()?;
            allowed()?;
            let now = tokio::time::Instant::now();
            if now >= until {
                return Ok(());
            }
            // The notification responds to explicit cancellation. This small tick
            // also checks shutdown/admission even without a lifecycle notification.
            let next = (now + Duration::from_millis(25)).min(until);
            tokio::select! {
                _ = &mut canceled => {},
                _ = tokio::time::sleep_until(next) => {},
            }
        }
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.active.lock() {
            if active.as_ref().is_some_and(|active| active.id == self.id) {
                *active = None;
            }
        }
        self.registry.idle.notify_waiters();
    }
}
pub(crate) struct CancelOnDrop {
    canceled: Arc<AtomicBool>,
    wake: Arc<Notify>,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invoking_cancel_never_frees_the_real_worker_slot() {
        let jobs = Arc::new(CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let invoke = work.cancel_on_drop();
        drop(invoke);
        assert_eq!(work.check().unwrap_err(), CANCELED);
        assert!(jobs.begin().unwrap().is_none());
        drop(work);
        let next = jobs.begin().unwrap().unwrap();
        next.check().unwrap();
    }
    #[test]
    fn cancel_on_exit_does_not_revive_after_exit_is_aborted() {
        let jobs = Arc::new(CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        jobs.cancel();
        assert!(work.check().is_err());
        drop(work);
        jobs.begin().unwrap().unwrap().check().unwrap();
    }
    #[tokio::test]
    async fn explicit_cancel_prevents_the_snapshot_dispatch_after_delay() {
        let jobs = Arc::new(CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        jobs.cancel();
        let calls = AtomicU64::new(0);
        assert!(work
            .wait_delay(3, || {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }
    #[tokio::test]
    async fn zero_delay_still_checks_shutdown_before_pixel_dispatch() {
        let jobs = Arc::new(CaptureJobs::default());
        let work = jobs.begin().unwrap().unwrap();
        let error = work
            .wait_delay(0, || Err("退出中".into()))
            .await
            .unwrap_err();
        assert_eq!(error, "退出中");
    }
}
