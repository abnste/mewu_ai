// SPDX-License-Identifier: MPL-2.0
//! Tracks actual HTTP task lifetimes. Durable work lives only in the core ledger.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, Notify};

#[derive(Clone, PartialEq, Eq)]
pub struct Origin {
    pub binding_id: String,
    pub plugin_id: String,
    pub plugin_revision: u64,
}

struct Task {
    origin: Origin,
    request: Option<(String, String)>,
    cancel: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct Registry {
    stopped: bool,
    tasks: HashMap<String, Task>,
}

#[derive(Default)]
pub struct Runtime {
    registry: Arc<Mutex<Registry>>,
    pub wake: Notify,
}

pub struct Permit {
    registry: Arc<Mutex<Registry>>,
    id: String,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.tasks.remove(&self.id);
        }
    }
}

impl Runtime {
    /// Register before checking the host exit gate. The permit remains with the
    /// task through its final ledger receipt; cancellation never releases it.
    pub fn register(&self, origin: Origin) -> Result<(Permit, oneshot::Receiver<()>), String> {
        self.register_owned(origin, None)
    }

    pub fn register_request(
        &self,
        origin: Origin,
        owner: &str,
        request_id: &str,
    ) -> Result<(Permit, oneshot::Receiver<()>), String> {
        if uuid::Uuid::parse_str(request_id)
            .map(|id| id.to_string() != request_id)
            .unwrap_or(true)
        {
            return Err("记忆请求标识无效".into());
        }
        self.register_owned(origin, Some((owner.into(), request_id.into())))
    }

    fn register_owned(
        &self,
        origin: Origin,
        request: Option<(String, String)>,
    ) -> Result<(Permit, oneshot::Receiver<()>), String> {
        let mut registry = self.registry.lock().map_err(|_| "记忆任务状态不可用")?;
        if registry.stopped {
            return Err("正在退出，请稍候".into());
        }
        if registry.tasks.len() >= 16 {
            return Err("记忆服务繁忙，请稍后重试".into());
        }
        if request.is_some() && registry.tasks.values().any(|task| task.request == request) {
            return Err("此记忆请求仍在处理".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (send, receive) = oneshot::channel();
        registry.tasks.insert(
            id.clone(),
            Task {
                origin,
                request,
                cancel: Some(send),
            },
        );
        Ok((
            Permit {
                registry: self.registry.clone(),
                id,
            },
            receive,
        ))
    }

    pub fn cancel_request(&self, owner: &str, request_id: &str) {
        if let Ok(mut registry) = self.registry.lock() {
            for task in registry.tasks.values_mut().filter(|task| {
                task.request
                    .as_ref()
                    .is_some_and(|(window, id)| window == owner && id == request_id)
            }) {
                if let Some(cancel) = task.cancel.take() {
                    let _ = cancel.send(());
                }
            }
        }
    }

    pub fn cancel_owner(&self, owner: &str) {
        if let Ok(mut registry) = self.registry.lock() {
            for task in registry.tasks.values_mut().filter(|task| {
                task.request
                    .as_ref()
                    .is_some_and(|(window, _)| window == owner)
            }) {
                if let Some(cancel) = task.cancel.take() {
                    let _ = cancel.send(());
                }
            }
        }
    }

    pub fn cancel_where(&self, revoked: impl Fn(&Origin) -> bool) {
        if let Ok(mut registry) = self.registry.lock() {
            for task in registry
                .tasks
                .values_mut()
                .filter(|task| revoked(&task.origin))
            {
                if let Some(cancel) = task.cancel.take() {
                    let _ = cancel.send(());
                }
            }
        }
    }

    pub fn resume(&self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.stopped = false;
        }
        self.wake.notify_one();
    }

    pub async fn drain(&self, timeout: Duration) -> Result<(), String> {
        {
            let mut registry = self.registry.lock().map_err(|_| "记忆任务状态不可用")?;
            registry.stopped = true;
            for task in registry.tasks.values_mut() {
                if let Some(cancel) = task.cancel.take() {
                    let _ = cancel.send(());
                }
            }
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self
                .registry
                .lock()
                .map_err(|_| "记忆任务状态不可用")?
                .tasks
                .is_empty()
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("记忆任务尚未结束，已取消退出".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn origin(binding: &str, revision: u64) -> Origin {
        Origin {
            binding_id: binding.into(),
            plugin_id: "mewu.memory-hindsight".into(),
            plugin_revision: revision,
        }
    }
    #[tokio::test]
    async fn cancellation_keeps_actual_task_registered_until_receipt_and_permit_drop() {
        let runtime = Runtime::default();
        let (first, mut receive) = runtime.register(origin("a", 1)).unwrap();
        let (second, mut other) = runtime.register(origin("b", 2)).unwrap();
        runtime.cancel_where(|task| task.binding_id == "a");
        receive.try_recv().unwrap();
        assert!(matches!(
            other.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(runtime.registry.lock().unwrap().tasks.len(), 2);
        assert!(runtime.drain(Duration::ZERO).await.is_err());
        other.try_recv().unwrap();
        assert!(runtime.register(origin("c", 3)).is_err());
        drop(first);
        drop(second);
        runtime.drain(Duration::ZERO).await.unwrap();
        runtime.resume();
        assert!(runtime.register(origin("c", 3)).is_ok());
    }
}
