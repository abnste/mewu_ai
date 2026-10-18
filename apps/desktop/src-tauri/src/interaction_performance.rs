// SPDX-License-Identifier: MPL-2.0
//! Opt-in, bounded timing receipts. No screen contents or application identities.
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};

const MAX_RECORDS: usize = 128;
const MAX_STAGES: usize = 16;
static RECORDER: OnceLock<Option<Arc<Recorder>>> = OnceLock::new();

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Kind {
    Capture,
    AddRegion,
    Publish,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Stage {
    Admitted,
    Hidden,
    Captured,
    Expanded,
    Adopted,
    Published,
    Decoded,
    Shown,
    Validated,
    Applied,
    Referenced,
    Locked,
    SnapshotReady,
    Emitted,
}

/// Call once, before admitting interactions. Failure disables measurement only.
/// The default path does not open a file or create a directory.
pub(crate) fn initialize(enabled: bool) {
    RECORDER.get_or_init(|| create_recorder(enabled, &std::env::temp_dir()));
}

fn create_recorder(enabled: bool, directory: &Path) -> Option<Arc<Recorder>> {
    if !enabled {
        return None;
    }
    let name = format!(
        "MewuAI-performance-{}-{}.jsonl",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
        .ok()?;
    Some(Arc::new(Recorder {
        file: Mutex::new(file),
        admitted: AtomicUsize::new(0),
        failed: AtomicBool::new(false),
    }))
}

struct Recorder {
    file: Mutex<File>,
    admitted: AtomicUsize,
    failed: AtomicBool,
}
impl Recorder {
    fn reserve(&self) -> bool {
        if self.failed.load(Ordering::Relaxed) {
            return false;
        }
        let mut count = self.admitted.load(Ordering::Relaxed);
        while count < MAX_RECORDS {
            match self.admitted.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => count = current,
            }
        }
        false
    }
    fn write(&self, record: &Record<'_>) {
        let Ok(mut encoded) = serde_json::to_vec(record) else {
            return;
        };
        encoded.push(b'\n');
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        // A failed receipt is never retried and cannot fail an interaction.
        match self.file.lock() {
            Ok(mut file) if !self.failed.load(Ordering::Relaxed) => {
                if file.write_all(&encoded).is_err() {
                    self.failed.store(true, Ordering::Relaxed);
                }
            }
            Ok(_) => {}
            Err(_) => self.failed.store(true, Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Mark {
    stage: Stage,
    elapsed_micros: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    version: u8,
    kind: Kind,
    success: bool,
    total_micros: u64,
    stages: &'a [Mark],
}

struct Active {
    recorder: Arc<Recorder>,
    kind: Kind,
    started: Instant,
    marks: [Mark; MAX_STAGES],
    count: usize,
}

pub(crate) struct Trace(Option<Active>);
impl Trace {
    pub(crate) fn start(kind: Kind) -> Self {
        Self::from_recorder(RECORDER.get().and_then(Option::as_ref).cloned(), kind)
    }
    fn from_recorder(recorder: Option<Arc<Recorder>>, kind: Kind) -> Self {
        Self(
            recorder
                .filter(|recorder| recorder.reserve())
                .map(|recorder| Active {
                    recorder,
                    kind,
                    started: Instant::now(),
                    marks: [Mark {
                        stage: Stage::Admitted,
                        elapsed_micros: 0,
                    }; MAX_STAGES],
                    count: 0,
                }),
        )
    }
    pub(crate) fn mark(&mut self, stage: Stage) {
        let Some(active) = self.0.as_mut() else {
            return;
        };
        if active.count < MAX_STAGES {
            active.marks[active.count] = Mark {
                stage,
                elapsed_micros: elapsed_micros(active.started),
            };
            active.count += 1;
        }
    }
    pub(crate) fn finish(mut self, success: bool) {
        self.record(success);
    }
    fn record(&mut self, success: bool) {
        let Some(active) = self.0.take() else {
            return;
        };
        active.recorder.write(&Record {
            version: 1,
            kind: active.kind,
            success,
            total_micros: elapsed_micros(active.started),
            stages: &active.marks[..active.count],
        });
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.record(false);
    }
}
fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mewu-interaction-performance-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn records(&self) -> Vec<Value> {
            let files: Vec<_> = fs::read_dir(&self.0).unwrap().map(Result::unwrap).collect();
            assert_eq!(files.len(), 1);
            let name = files[0].file_name().to_string_lossy().into_owned();
            assert!(name.starts_with(&format!("MewuAI-performance-{}-", std::process::id())));
            assert!(name.ends_with(".jsonl"));
            let raw = fs::read_to_string(files[0].path()).unwrap();
            raw.lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn disabled_measurement_never_creates_a_file_or_directory() {
        let missing =
            std::env::temp_dir().join(format!("mewu-performance-absent-{}", uuid::Uuid::new_v4()));
        let recorder = create_recorder(false, &missing);
        assert!(recorder.is_none());
        let mut trace = Trace::from_recorder(recorder, Kind::Capture);
        trace.mark(Stage::Hidden);
        trace.finish(true);
        assert!(!missing.exists());
    }

    #[test]
    fn bounds_records_and_marks_and_only_serializes_fixed_fields() {
        let fixture = Fixture::new();
        let recorder = create_recorder(true, &fixture.0).unwrap();
        for _ in 0..MAX_RECORDS + 8 {
            let mut trace = Trace::from_recorder(Some(recorder.clone()), Kind::AddRegion);
            for _ in 0..MAX_STAGES + 8 {
                trace.mark(Stage::Applied);
            }
            trace.finish(true);
        }
        drop(recorder);
        let records = fixture.records();
        assert_eq!(records.len(), MAX_RECORDS);
        for record in records {
            let fields = record.as_object().unwrap();
            assert_eq!(fields.len(), 5);
            assert_eq!(record["version"], 1);
            assert_eq!(record["kind"], "addRegion");
            assert_eq!(record["success"], true);
            let total = record["totalMicros"].as_u64().unwrap();
            let marks = record["stages"].as_array().unwrap();
            assert_eq!(marks.len(), MAX_STAGES);
            let mut previous = 0;
            for mark in marks {
                assert_eq!(mark.as_object().unwrap().len(), 2);
                assert_eq!(mark["stage"], "applied");
                let elapsed = mark["elapsedMicros"].as_u64().unwrap();
                assert!((previous..=total).contains(&elapsed));
                previous = elapsed;
            }
        }
    }

    #[test]
    fn abandoned_trace_is_failed_once_and_io_failure_is_optional() {
        let fixture = Fixture::new();
        let recorder = create_recorder(true, &fixture.0).unwrap();
        {
            let mut trace = Trace::from_recorder(Some(recorder.clone()), Kind::Publish);
            trace.mark(Stage::Emitted);
        }
        Trace::from_recorder(Some(recorder.clone()), Kind::Capture).finish(true);
        drop(recorder);
        let records = fixture.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["kind"], "publish");
        assert_eq!(records[0]["success"], false);
        assert_eq!(records[1]["kind"], "capture");
        assert_eq!(records[1]["success"], true);
        assert!(create_recorder(true, &fixture.0.join("missing")).is_none());
    }

    #[test]
    fn a_write_failure_disables_further_receipts_without_affecting_callers() {
        let fixture = Fixture::new();
        let path = fixture.0.join("read-only-for-this-handle");
        fs::write(&path, b"").unwrap();
        let recorder = Arc::new(Recorder {
            file: Mutex::new(File::open(path).unwrap()),
            admitted: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
        });
        Trace::from_recorder(Some(recorder.clone()), Kind::Capture).finish(true);
        assert!(recorder.failed.load(Ordering::Relaxed));
        assert!(Trace::from_recorder(Some(recorder.clone()), Kind::Publish)
            .0
            .is_none());
        assert_eq!(recorder.admitted.load(Ordering::Relaxed), 1);
    }
}
