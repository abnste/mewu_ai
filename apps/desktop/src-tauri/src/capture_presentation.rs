// SPDX-License-Identifier: MPL-2.0
//! A new screenshot stays hidden until its exact DOM image has decoded.
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub scene: String,
    pub background: String,
    pub width: u32,
    pub height: u32,
}
struct Pending {
    identity: Identity,
    outcome: Option<bool>,
    deadline: tokio::time::Instant,
    wake: Arc<Notify>,
}
#[derive(Default)]
pub(crate) struct Registry(Mutex<Option<Pending>>);
impl Registry {
    pub(crate) fn begin(self: &Arc<Self>, identity: Identity) -> Result<Presentation, String> {
        let mut pending = self.0.lock().map_err(|_| "截图呈现状态不可用")?;
        if pending.is_some() || identity.width == 0 || identity.height == 0 {
            return Err("截图呈现状态无效".into());
        }
        let wake = Arc::new(Notify::new());
        *pending = Some(Pending {
            identity: identity.clone(),
            outcome: None,
            wake: wake.clone(),
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(8),
        });
        Ok(Presentation {
            registry: self.clone(),
            identity,
            wake,
        })
    }
    /// Duplicate/late/wrong-background acknowledgements never authorize a show.
    pub(crate) fn acknowledge(&self, identity: &Identity, success: bool) -> Result<bool, String> {
        let mut pending = self.0.lock().map_err(|_| "截图呈现状态不可用")?;
        let Some(pending) = pending.as_mut() else {
            return Ok(false);
        };
        if pending.identity != *identity
            || pending.outcome.is_some()
            || tokio::time::Instant::now() >= pending.deadline
        {
            return Ok(false);
        }
        pending.outcome = Some(success);
        pending.wake.notify_waiters();
        Ok(true)
    }
}
pub(crate) struct Presentation {
    registry: Arc<Registry>,
    identity: Identity,
    wake: Arc<Notify>,
}
impl Presentation {
    pub(crate) async fn wait(
        &self,
        allowed: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            allowed()?;
            let (outcome, until) = {
                let pending = self.registry.0.lock().map_err(|_| "截图呈现状态不可用")?;
                let pending = pending
                    .as_ref()
                    .filter(|pending| pending.identity == self.identity)
                    .ok_or("截图呈现已结束")?;
                (pending.outcome, pending.deadline)
            };
            let now = tokio::time::Instant::now();
            if now >= until {
                return Err("截图显示超时，请重新截图".into());
            }
            match outcome {
                Some(true) => {
                    allowed()?;
                    return Ok(());
                }
                Some(false) => return Err("无法读取屏幕截图".into()),
                None => {}
            }
            // This is an admission/cancel check, not a guessed rendering delay.
            let next = (now + std::time::Duration::from_millis(25)).min(until);
            tokio::select! { _ = &mut notified => {}, _ = tokio::time::sleep_until(next) => {} }
        }
    }
}
impl Drop for Presentation {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.registry.0.lock() {
            if pending
                .as_ref()
                .is_some_and(|pending| pending.identity == self.identity)
            {
                *pending = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(background: &str) -> Identity {
        Identity {
            scene: "scene".into(),
            background: background.into(),
            width: 1920,
            height: 1080,
        }
    }
    #[tokio::test]
    async fn only_exact_decoded_original_is_admitted_once() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("new")).unwrap();
        assert!(!registry.acknowledge(&identity("old"), true).unwrap());
        let mut wrong = identity("new");
        wrong.width = 960;
        assert!(!registry.acknowledge(&wrong, true).unwrap());
        wrong = identity("new");
        wrong.scene = "other".into();
        assert!(!registry.acknowledge(&wrong, true).unwrap());
        assert!(registry.acknowledge(&identity("new"), true).unwrap());
        assert!(!registry.acknowledge(&identity("new"), true).unwrap());
        pending.wait(|| Ok(())).await.unwrap();
        drop(pending);
        assert!(!registry.acknowledge(&identity("new"), true).unwrap());
    }
    #[tokio::test]
    async fn cancellation_or_exit_wins_even_after_success() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("new")).unwrap();
        registry.acknowledge(&identity("new"), true).unwrap();
        assert_eq!(
            pending.wait(|| Err("截图已取消".into())).await.unwrap_err(),
            "截图已取消"
        );
    }
    #[tokio::test]
    async fn failed_decode_is_not_a_success_and_drop_releases_only_its_owner() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("one")).unwrap();
        assert!(registry.begin(identity("two")).is_err());
        registry.acknowledge(&identity("one"), false).unwrap();
        assert_eq!(
            pending.wait(|| Ok(())).await.unwrap_err(),
            "无法读取屏幕截图"
        );
        drop(pending);
        let next = registry.begin(identity("two")).unwrap();
        assert!(!registry.acknowledge(&identity("one"), true).unwrap());
        drop(next);
    }
    #[tokio::test]
    async fn expired_render_cannot_acknowledge_or_revive_a_window() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("new")).unwrap();
        registry.0.lock().unwrap().as_mut().unwrap().deadline = tokio::time::Instant::now();
        assert!(!registry.acknowledge(&identity("new"), true).unwrap());
        assert_eq!(
            pending.wait(|| Ok(())).await.unwrap_err(),
            "截图显示超时，请重新截图"
        );
        drop(pending);
        assert!(!registry.acknowledge(&identity("new"), true).unwrap());
    }
    #[tokio::test]
    async fn acknowledgement_after_wait_starts_wakes_the_exact_owner() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("new")).unwrap();
        let (waited, acknowledged) = tokio::join!(pending.wait(|| Ok(())), async {
            tokio::task::yield_now().await;
            registry.acknowledge(&identity("new"), true).unwrap()
        });
        waited.unwrap();
        assert!(acknowledged);
    }
    #[tokio::test]
    async fn cancel_during_wait_keeps_owner_until_the_wait_actually_ends() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("one")).unwrap();
        let canceled = AtomicBool::new(false);
        let (waited, ()) = tokio::join!(
            pending.wait(|| {
                if canceled.load(Ordering::Acquire) {
                    Err("截图已取消".into())
                } else {
                    Ok(())
                }
            }),
            async {
                tokio::task::yield_now().await;
                assert!(registry.begin(identity("two")).is_err());
                canceled.store(true, Ordering::Release);
            }
        );
        assert_eq!(waited.unwrap_err(), "截图已取消");
        assert!(registry.begin(identity("two")).is_err());
        drop(pending);
        drop(registry.begin(identity("two")).unwrap());
    }
    #[tokio::test]
    async fn missing_ack_has_a_real_deadline_instead_of_an_unbounded_hidden_frame_wait() {
        let registry = Arc::new(Registry::default());
        let pending = registry.begin(identity("new")).unwrap();
        registry.0.lock().unwrap().as_mut().unwrap().deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(5);
        assert_eq!(
            pending.wait(|| Ok(())).await.unwrap_err(),
            "截图显示超时，请重新截图"
        );
        assert!(!registry.acknowledge(&identity("new"), true).unwrap());
    }
}
