// SPDX-License-Identifier: MPL-2.0
//! Explicitly enabled, content-free observations of one durable persistence.
//! The domain layer never opens a log or chooses a diagnostic output path.
use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PersistencePhase {
    GeometryCleanup,
    ValidateCandidate,
    TransactionAndLedger,
    ValidateReconciled,
    ValidateLedgers,
    Serialize,
    SqlUpdate,
    DurableCommit,
    AdoptState,
    CloneResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceMark {
    pub phase: PersistencePhase,
    /// Elapsed since entering persistence, not an application timestamp.
    pub elapsed_micros: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceTiming {
    pub success: bool,
    pub total_micros: u64,
    pub completed: [Option<PersistenceMark>; 10],
}

pub type PersistenceObserver = fn(PersistenceTiming);

struct Active {
    observer: PersistenceObserver,
    started: Instant,
    completed: [Option<PersistenceMark>; 10],
    count: usize,
}

pub(crate) struct Observation(Option<Active>);
impl Observation {
    pub(crate) fn start(observer: Option<PersistenceObserver>) -> Self {
        Self::with_clock(observer, Instant::now)
    }
    fn with_clock(observer: Option<PersistenceObserver>, now: fn() -> Instant) -> Self {
        // In normal operation there is neither a clock read nor an allocation.
        Self(observer.map(|observer| Active {
            observer, started: now(), completed: [None; 10], count: 0,
        }))
    }
    pub(crate) fn mark(&mut self, phase: PersistencePhase) {
        let Some(active) = self.0.as_mut() else { return };
        if active.count < active.completed.len() {
            active.completed[active.count] = Some(PersistenceMark {
                phase, elapsed_micros: micros(active.started),
            });
            active.count += 1;
        }
    }
    pub(crate) fn finish(mut self) {
        self.record(true);
    }
    fn record(&mut self, success: bool) {
        let Some(active) = self.0.take() else { return };
        let result = PersistenceTiming {
            success, total_micros: micros(active.started), completed: active.completed,
        };
        // Diagnostics must not change a commit's outcome, including a faulty
        // host callback. No business data is passed to this function pointer.
        let _ = std::panic::catch_unwind(|| (active.observer)(result));
    }
}
impl Drop for Observation {
    fn drop(&mut self) { self.record(false); }
}
fn micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    static RESULT: Mutex<Vec<PersistenceTiming>> = Mutex::new(Vec::new());
    fn observe(result: PersistenceTiming) { RESULT.lock().unwrap().push(result); }
    fn unexpected_clock() -> Instant { panic!("disabled observation must not read the clock") }

    #[test]
    fn default_off_never_reads_clock_or_calls_observer() {
        let mut observation=Observation::with_clock(None, unexpected_clock);
        observation.mark(PersistencePhase::GeometryCleanup);
        observation.finish();
    }

    #[test]
    fn enabled_observation_has_only_bounded_ordered_durations_and_drop_failure() {
        RESULT.lock().unwrap().clear();
        let mut observation=Observation::start(Some(observe));
        observation.mark(PersistencePhase::GeometryCleanup);
        observation.mark(PersistencePhase::ValidateCandidate);
        observation.finish();
        let mut failed=Observation::start(Some(observe));
        for _ in 0..20 { failed.mark(PersistencePhase::GeometryCleanup); }
        drop(failed);
        let result=RESULT.lock().unwrap();
        assert_eq!(result.len(),2);
        assert!(result[0].success);
        assert_eq!(result[0].completed.iter().flatten().count(),2);
        assert!(result[0].completed.iter().flatten().all(|mark|mark.elapsed_micros<=result[0].total_micros));
        assert!(result[0].completed[0].unwrap().elapsed_micros<=result[0].completed[1].unwrap().elapsed_micros);
        assert!(!result[1].success);
        assert_eq!(result[1].completed.iter().flatten().count(),10);
        let json=serde_json::to_value(result[0]).unwrap();
        assert_eq!(json.as_object().unwrap().len(),3);
        assert_eq!(json["completed"][0].as_object().unwrap().len(),2);
    }
}
