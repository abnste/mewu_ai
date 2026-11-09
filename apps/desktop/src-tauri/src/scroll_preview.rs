// SPDX-License-Identifier: MPL-2.0
//! One active preview and one replaceable pending preview; never queues capture frames.
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

struct State<T> {
    pending: Option<T>,
    stopped: bool,
    error: Option<String>,
}

pub(super) struct LatestPreview<T> {
    shared: Arc<(Mutex<State<T>>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> LatestPreview<T> {
    pub(super) fn start(
        mut consume: impl FnMut(T) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new((
            Mutex::new(State {
                pending: None,
                stopped: false,
                error: None,
            }),
            Condvar::new(),
        ));
        let work = shared.clone();
        let worker = std::thread::Builder::new()
            .name("scroll-preview".into())
            .spawn(move || {
                let (lock, ready) = &*work;
                loop {
                    let value = {
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        while state.pending.is_none() && !state.stopped {
                            state = ready.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                        if state.stopped {
                            return;
                        }
                        state.pending.take().unwrap()
                    };
                    if let Err(error) = consume(value) {
                        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                        state.error = Some(error);
                        state.pending = None;
                        state.stopped = true;
                        return;
                    }
                }
            })
            .map_err(|_| "无法启动长截图预览")?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub(super) fn submit(&self, value: T) -> Result<(), String> {
        let (lock, ready) = &*self.shared;
        let mut state = lock.lock().map_err(|_| "长截图预览状态不可用")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if state.stopped {
            return Err("长截图预览已结束".into());
        }
        state.pending = Some(value);
        ready.notify_one();
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<(), String> {
        self.stop();
        self.shared
            .0
            .lock()
            .map_err(|_| "长截图预览状态不可用")?
            .error
            .clone()
            .map_or(Ok(()), Err)
    }
}

impl<T> LatestPreview<T> {
    fn stop(&mut self) {
        let (lock, ready) = &*self.shared;
        {
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            state.stopped = true;
            state.pending = None;
            ready.notify_one();
        }
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            lock.lock().unwrap_or_else(|e| e.into_inner()).error =
                Some("长截图预览任务中断".into());
        }
    }
}

impl<T> Drop for LatestPreview<T> {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn pending_previews_replace_without_blocking_the_producer() {
        let (seen, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let worker = LatestPreview::start(move |value| {
            seen.send(value).unwrap();
            if value == 1 {
                gate.recv().unwrap();
            }
            Ok(())
        })
        .unwrap();
        worker.submit(1).unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        worker.submit(2).unwrap();
        worker.submit(3).unwrap();
        release.send(()).unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 3);
        worker.finish().unwrap();
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn stopping_discards_pending_and_joins_the_active_task() {
        let (seen, received) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let worker = LatestPreview::start(move |value| {
            seen.send(value).unwrap();
            gate.recv().unwrap();
            Ok(())
        })
        .unwrap();
        worker.submit(1).unwrap();
        assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        worker.submit(2).unwrap();
        let shared = worker.shared.clone();
        // Make shutdown deterministic while the consumer is held in its first task.
        let closer = std::thread::spawn(move || worker.finish());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !shared.0.lock().unwrap().stopped {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        closer.join().unwrap().unwrap();
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn consumer_failure_is_returned_by_finish() {
        let (done, received) = mpsc::channel();
        let worker = LatestPreview::start(move |_: ()| {
            done.send(()).unwrap();
            Err("encode failed".into())
        })
        .unwrap();
        worker.submit(()).unwrap();
        received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(worker.finish().unwrap_err(), "encode failed");
    }
}
