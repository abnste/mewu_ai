// SPDX-License-Identifier: MPL-2.0
//! A Store-owned maintenance connection. Admission, schema, FULL and file
//! identity are checked by the caller before ownership crosses the thread.
//! There is no path reopening, domain mutation, WAL hook or Engine dependency.
use rusqlite::{Connection, ErrorCode};
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
};

pub(crate) struct CheckpointWorker {
    sender: Option<SyncSender<()>>,
    stopping: Arc<AtomicBool>,
    healthy: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    joined_ok: bool,
}

enum Attempt {
    CompleteOrPartial,
    Deferred,
    Failed,
}

struct MarkClosed(Arc<AtomicBool>);
impl Drop for MarkClosed {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CheckpointWorker {
    /// The connection must already be a verified, existing RW connection in
    /// WAL/FULL mode with no active transaction. Spawn failure drops it without
    /// altering the caller's existing writer fallback.
    pub(crate) fn start(connection: Connection) -> io::Result<Self> {
        if !connection.is_autocommit() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "checkpoint transaction active",
            ));
        }
        let mut changed_notifications = 0u32;
        Self::spawn(move |force| scheduled(&connection, &mut changed_notifications, force))
    }

    fn spawn(mut attempt: impl FnMut(bool) -> Attempt + Send + 'static) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let healthy = Arc::new(AtomicBool::new(true));
        let stop = Arc::clone(&stopping);
        let live = Arc::clone(&healthy);
        let thread = thread::Builder::new()
            .name("mewu-wal-maintenance".into())
            .spawn(move || {
                let _closed = MarkClosed(live);
                if ready.send(()).is_err() {
                    return;
                }
                let mut deferred = false;
                loop {
                    let message = if deferred {
                        // Keep ordinary pauses between drawing/selection gestures in
                        // one WAL generation. FULL still syncs every commit;
                        // frame/count pressure and final shutdown drain sooner.
                        receiver.recv_timeout(std::time::Duration::from_secs(30))
                    } else {
                        receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    let force = match message {
                        Ok(()) if !stop.load(Ordering::Acquire) => false,
                        Err(mpsc::RecvTimeoutError::Timeout) => true,
                        _ => break,
                    };
                    match attempt(force) {
                        Attempt::Deferred => deferred = true,
                        Attempt::CompleteOrPartial => deferred = false,
                        Attempt::Failed => return,
                    }
                }
                // A successful COMMIT is already FULL-durable. One final
                // PASSIVE drains deferred maintenance without waiting for old
                // readers or adding an exclusive/FULL/TRUNCATE checkpoint.
                let _ = attempt(true);
                // attempt owns the connection; it is closed before join returns.
            })?;
        if ready_receiver.recv().is_err() {
            let _ = thread.join();
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "checkpoint worker startup interrupted",
            ));
        }
        Ok(Self {
            sender: Some(sender),
            stopping,
            healthy,
            thread: Some(thread),
            joined_ok: true,
        })
    }

    /// Call only after a successful business COMMIT. Full means a wakeup is
    /// already queued, so coalescing never waits for the maintenance thread.
    /// False lets the writer restore its normal synchronous fallback.
    pub(crate) fn notify(&self) -> bool {
        if !self.is_healthy() {
            return false;
        }
        match self.sender.as_ref().map(|sender| sender.try_send(())) {
            Some(Ok(())) | Some(Err(TrySendError::Full(()))) => true,
            _ => {
                self.healthy.store(false, Ordering::Release);
                false
            }
        }
    }

    pub(crate) fn is_healthy(&self) -> bool {
        !self.stopping.load(Ordering::Acquire) && self.healthy.load(Ordering::Acquire)
    }

    pub(crate) fn request_stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    /// Finish an in-flight PASSIVE, coalesce queued work into one final PASSIVE, close its
    /// connection and join. No final FULL/TRUNCATE or unbounded retry loop.
    /// The owning Store must retain its writer until this returns. Call outside
    /// the Engine/UI lock for explicit application shutdown.
    pub(crate) fn shutdown(&mut self) -> bool {
        self.request_stop();
        drop(self.sender.take());
        if let Some(thread) = self.thread.take() {
            self.joined_ok &= thread.join().is_ok();
        }
        self.joined_ok
    }
}

impl Drop for CheckpointWorker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// NOOP only observes the current WAL; it does not backfill or issue a sync.
// Amortize WAL restart/header sync over a burst, with the existing 8192-frame
// writer fallback unchanged. An old ExportView may still delay reclamation,
// exactly as under SQLite's original policy; business writes are not rejected.
fn scheduled(connection: &Connection, changed: &mut u32, force: bool) -> Attempt {
    let status = connection.query_row("PRAGMA main.wal_checkpoint(NOOP)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    match status {
        Ok((busy, logged, copied))
            if (busy == 0 || busy == 1) && logged >= 0 && copied >= 0 && copied <= logged =>
        {
            if copied == logged {
                *changed = 0;
                return Attempt::CompleteOrPartial;
            }
            *changed = changed.saturating_add(1);
            if !force && logged - copied < 512 && *changed < 16 {
                return Attempt::Deferred;
            }
            let outcome = passive(connection);
            if matches!(outcome, Attempt::CompleteOrPartial) {
                *changed = 0;
            }
            outcome
        }
        Ok((0, -1, -1)) => {
            // An existing WAL database can have no initialized log yet. A
            // non-WAL connection remains a permanent maintenance failure.
            let mode: rusqlite::Result<String> =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0));
            if mode.is_ok_and(|value| value.eq_ignore_ascii_case("wal")) {
                *changed = 0;
                Attempt::CompleteOrPartial
            } else {
                Attempt::Failed
            }
        }
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            Attempt::CompleteOrPartial
        }
        _ => Attempt::Failed,
    }
}

fn passive(connection: &Connection) -> Attempt {
    let result = connection.query_row("PRAGMA main.wal_checkpoint(PASSIVE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    match result {
        Ok((busy, logged, copied))
            if (busy == 0 || busy == 1) && logged >= 0 && copied >= 0 && copied <= logged =>
        {
            Attempt::CompleteOrPartial
        }
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            Attempt::CompleteOrPartial
        }
        _ => Attempt::Failed,
    }
}

fn control_error(code: i32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
}

/// No CREATE and no reopening occur here. Existing host root/DB lifetime pins
/// remain the caller's responsibility. False/unsupported means do not enable
/// the worker, retaining the baseline writer checkpoint threshold.
pub(crate) fn same_file_identity(
    writer: &Connection,
    maintenance: &Connection,
    path: &Path,
) -> rusqlite::Result<bool> {
    same_file_identity_impl(writer, maintenance, path)
}

#[cfg(windows)]
fn same_file_identity_impl(
    writer: &Connection,
    maintenance: &Connection,
    path: &Path,
) -> rusqlite::Result<bool> {
    use std::{
        ffi::c_void,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
    };
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut Information) -> i32;
    }
    fn native_id(handle: *mut c_void) -> Option<(u32, u32, u32)> {
        if handle.is_null() || handle == (-1isize as *mut c_void) {
            return None;
        }
        let mut information = Information::default();
        if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
            return None;
        }
        if information.attributes & (0x10 | 0x400) != 0 {
            return None;
        }
        Some((
            information.volume,
            information.index_high,
            information.index_low,
        ))
    }
    fn file_id(connection: &Connection) -> rusqlite::Result<Option<(u32, u32, u32)>> {
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SQLite owns this HANDLE; it is only borrowed, never closed here.
        let code = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                b"main\0".as_ptr().cast(),
                rusqlite::ffi::SQLITE_FCNTL_WIN32_GET_HANDLE,
                (&mut handle as *mut *mut c_void).cast(),
            )
        };
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(control_error(code));
        }
        Ok(native_id(handle))
    }
    let ordinary = std::fs::symlink_metadata(path)
        .ok()
        .is_some_and(|value| value.is_file() && value.file_attributes() & 0x400 == 0);
    if !ordinary {
        return Ok(false);
    }
    let pin = match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x00200000)
        .open(path)
    {
        Ok(pin) => pin,
        Err(_) => return Ok(false),
    };
    let expected = native_id(pin.as_raw_handle());
    let first = file_id(writer)?;
    let second = file_id(maintenance)?;
    Ok(expected.is_some() && expected == first && first == second)
}

#[cfg(unix)]
fn same_file_identity_impl(
    writer: &Connection,
    maintenance: &Connection,
    path: &Path,
) -> rusqlite::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    fn path_id(path: &Path) -> Option<(u64, u64)> {
        std::fs::symlink_metadata(path)
            .ok()
            .filter(|value| value.is_file() && !value.file_type().is_symlink())
            .map(|value| (value.dev(), value.ino()))
    }
    fn has_moved(connection: &Connection) -> rusqlite::Result<bool> {
        let mut moved = 1i32;
        let code = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                b"main\0".as_ptr().cast(),
                rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                (&mut moved as *mut i32).cast(),
            )
        };
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(control_error(code));
        }
        Ok(moved != 0)
    }
    // Public Unix VFS check binds each opened connection to its current path.
    // Do not unpack SQLite's private unixFile ABI to obtain an fd. Existing
    // root/db pins are required; this is not an adversarial ABA-rename defence.
    let before = path_id(path);
    let writer_path = writer
        .path()
        .filter(|value| !value.is_empty())
        .map(Path::new);
    let maintenance_path = maintenance
        .path()
        .filter(|value| !value.is_empty())
        .map(Path::new);
    if before.is_none()
        || writer_path.and_then(path_id) != before
        || maintenance_path.and_then(path_id) != before
    {
        return Ok(false);
    }
    if before.is_none() || has_moved(writer)? || has_moved(maintenance)? {
        return Ok(false);
    }
    let after = path_id(path);
    if before != after || has_moved(writer)? || has_moved(maintenance)? {
        return Ok(false);
    }
    Ok(before == path_id(path)
        && writer_path.and_then(path_id) == before
        && maintenance_path.and_then(path_id) == before)
}

#[cfg(not(any(windows, unix)))]
fn same_file_identity_impl(_: &Connection, _: &Connection, _: &Path) -> rusqlite::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Duration};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("mewu-checkpoint-worker-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn database(&self) -> PathBuf {
            self.0.join("spaces.db")
        }
        fn writer(&self) -> Connection {
            let connection = Connection::open(self.database()).unwrap();
            connection.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;PRAGMA wal_autocheckpoint=0;CREATE TABLE IF NOT EXISTS sample(id INTEGER PRIMARY KEY,value TEXT NOT NULL);INSERT OR IGNORE INTO sample VALUES(1,'unchanged');").unwrap();
            connection
        }
        fn maintenance(&self) -> Connection {
            let connection = Connection::open_with_flags(
                self.database(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            connection
                .execute_batch("PRAGMA synchronous=FULL;")
                .unwrap();
            connection
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in ["spaces.db-wal", "spaces.db-shm", "spaces.db"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn verifies_real_file_identity_and_rejects_other_file_or_memory() {
        let fixture = Fixture::new();
        let other = Fixture::new();
        let writer = fixture.writer();
        let maintenance = fixture.maintenance();
        let foreign = other.writer();
        assert!(same_file_identity(&writer, &maintenance, &fixture.database()).unwrap());
        assert!(!same_file_identity(&writer, &foreign, &fixture.database()).unwrap());
        assert!(!same_file_identity(&writer, &maintenance, &other.database()).unwrap());
        let memory = Connection::open_in_memory().unwrap();
        assert!(!same_file_identity(&writer, &memory, &fixture.database()).unwrap_or(false));
    }

    #[test]
    fn real_wal_checkpoint_preserves_reader_snapshot_and_business_rows() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        let reader = Connection::open_with_flags(
            fixture.database(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        reader.execute_batch("BEGIN DEFERRED").unwrap();
        assert_eq!(
            reader
                .query_row("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "unchanged"
        );
        writer
            .execute("UPDATE sample SET value='committed' WHERE id=1", [])
            .unwrap();
        let maintenance = fixture.maintenance();
        assert!(same_file_identity(&writer, &maintenance, &fixture.database()).unwrap());
        // A protected reader may leave only partial progress; this is healthy.
        assert!(matches!(passive(&maintenance), Attempt::CompleteOrPartial));
        assert_eq!(
            reader
                .query_row("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "unchanged"
        );
        reader.execute_batch("ROLLBACK").unwrap();
        assert!(matches!(passive(&maintenance), Attempt::CompleteOrPartial));
        let result: (i64, i64, i64) = maintenance
            .query_row("PRAGMA main.wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(result.0, 0);
        assert_eq!(result.1, result.2);
        assert_eq!(
            writer
                .query_row("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "committed"
        );
        let mut worker = CheckpointWorker::start(maintenance).unwrap();
        assert!(worker.is_healthy());
        assert!(worker.notify());
        assert!(worker.shutdown());
        drop(worker);
        drop(reader);
        drop(writer);
        let reopened = Connection::open_with_flags(
            fixture.database(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .unwrap();
        assert_eq!(
            reopened
                .query_row("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "committed"
        );
    }

    #[test]
    fn rejects_active_transaction_without_spawning_worker() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("BEGIN").unwrap();
        assert_eq!(
            CheckpointWorker::start(connection).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn coalesces_queued_work_and_joins_inflight_before_closing() {
        let (entered, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let drains = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let drained = Arc::clone(&drains);
        let mut worker = CheckpointWorker::spawn(move |_force| {
            if _force {
                drained.fetch_add(1, Ordering::Relaxed);
                return Attempt::CompleteOrPartial;
            }
            if entered.send(()).is_err() || wait.recv_timeout(Duration::from_secs(3)).is_err() {
                return Attempt::Failed;
            }
            Attempt::CompleteOrPartial
        })
        .unwrap();
        assert!(worker.notify());
        seen.recv_timeout(Duration::from_secs(3)).unwrap();
        for _ in 0..1000 {
            assert!(worker.notify());
        }
        release.send(()).unwrap();
        seen.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.request_stop();
        release.send(()).unwrap();
        assert!(worker.shutdown());
        assert!(worker.shutdown());
        assert!(!worker.is_healthy());
        assert!(!worker.notify());
        assert!(seen.recv_timeout(Duration::from_millis(10)).is_err());
        assert_eq!(drains.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn failed_or_panicked_worker_rejects_notifications_and_joins() {
        for panic in [false, true] {
            let (entered, seen) = mpsc::channel();
            let mut worker = CheckpointWorker::spawn(move |_force| {
                entered.send(()).unwrap();
                assert!(!panic, "synthetic maintenance panic");
                Attempt::Failed
            })
            .unwrap();
            assert!(worker.notify());
            seen.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(worker.shutdown(), !panic);
            assert!(!worker.is_healthy());
            assert!(!worker.notify());
        }
    }

    #[test]
    fn passive_reports_non_wal_failure_without_mutating_domain_rows() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE sample(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO sample VALUES(1,'unchanged');").unwrap();
        assert!(matches!(passive(&connection), Attempt::Failed));
        assert_eq!(
            connection
                .query_row("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "unchanged"
        );
    }
    fn status(connection: &Connection) -> (i64, i64, i64) {
        connection
            .query_row("PRAGMA wal_checkpoint(NOOP)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
    }
    #[test]
    fn scheduled_small_commits_append_without_reset_and_force_drains_same_full_connection() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        let maintenance = fixture.maintenance();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut changed = 0;
        let mut prior_frames = 0;
        for index in 0..6 {
            writer
                .execute(
                    "UPDATE sample SET value=?1 WHERE id=1",
                    [format!("small {index}")],
                )
                .unwrap();
            assert!(matches!(
                scheduled(&maintenance, &mut changed, false),
                Attempt::Deferred
            ));
            let (_, frames, copied) = status(&maintenance);
            assert!(frames > prior_frames);
            assert_eq!(copied, 0);
            prior_frames = frames;
        }
        assert!(matches!(
            scheduled(&maintenance, &mut changed, true),
            Attempt::CompleteOrPartial
        ));
        let (_, frames, copied) = status(&maintenance);
        assert_eq!(frames, copied);
        assert!(frames > 0);
        assert_eq!(
            writer
                .pragma_query_value::<i64, _>(None, "synchronous", |r| r.get(0))
                .unwrap(),
            2
        );
        assert_eq!(
            maintenance
                .pragma_query_value::<i64, _>(None, "synchronous", |r| r.get(0))
                .unwrap(),
            2
        );
        assert_eq!(
            writer
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |r| r.get(0))
                .unwrap(),
            "small 5"
        );
    }
    #[test]
    fn scheduled_real_frame_pressure_backfills_without_waiting_for_notification_count() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        let maintenance = fixture.maintenance();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        writer
            .execute(
                "UPDATE sample SET value=zeroblob(3*1024*1024) WHERE id=1",
                [],
            )
            .unwrap();
        assert!(status(&maintenance).1 > 512);
        let mut changed = 0;
        assert!(matches!(
            scheduled(&maintenance, &mut changed, false),
            Attempt::CompleteOrPartial
        ));
        let (_, frames, copied) = status(&maintenance);
        assert_eq!(frames, copied);
        assert!(frames > 512);
        assert_eq!(
            writer
                .query_row::<i64, _, _>("SELECT length(value) FROM sample WHERE id=1", [], |r| r
                    .get(0))
                .unwrap(),
            3 * 1024 * 1024
        );
    }
    #[test]
    fn scheduled_many_tiny_commits_eventually_backfill_before_fallback_without_per_commit_reset() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        let maintenance = fixture.maintenance();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut changed = 0;
        let mut first_backfill = None;
        for index in 1..=32 {
            writer
                .execute(
                    "UPDATE sample SET value=?1 WHERE id=1",
                    [format!("count {index}")],
                )
                .unwrap();
            assert!(!matches!(
                scheduled(&maintenance, &mut changed, false),
                Attempt::Failed
            ));
            let (_, frames, copied) = status(&maintenance);
            assert!(frames < 512);
            if copied > 0 {
                first_backfill = Some(index);
                break;
            }
        }
        let ordinal =
            first_backfill.expect("bounded notification policy must reclaim tiny WAL writes");
        assert!(ordinal > 1 && ordinal <= 32);
    }
    #[test]
    fn real_worker_idle_flushes_a_deferred_small_commit_and_shutdown_releases_its_handle() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut worker = CheckpointWorker::start(fixture.maintenance()).unwrap();
        writer
            .execute("UPDATE sample SET value='idle committed' WHERE id=1", [])
            .unwrap();
        assert!(worker.notify());
        let deadline = std::time::Instant::now() + Duration::from_secs(35);
        loop {
            let (_, frames, copied) = status(&writer);
            if frames > 0 && frames == copied {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(worker.is_healthy());
        assert!(worker.shutdown());
        drop(worker);
        drop(writer);
        let reopened = Connection::open(fixture.database()).unwrap();
        assert_eq!(
            reopened
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |r| r.get(0))
                .unwrap(),
            "idle committed"
        );
    }
    #[test]
    fn real_worker_shutdown_forces_deferred_work_without_waiting_for_old_export_reader() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let reader = Connection::open(fixture.database()).unwrap();
        reader.execute_batch("BEGIN DEFERRED").unwrap();
        assert_eq!(
            reader
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |r| r.get(0))
                .unwrap(),
            "unchanged"
        );
        let mut worker = CheckpointWorker::start(fixture.maintenance()).unwrap();
        writer
            .execute("UPDATE sample SET value='deferred durable' WHERE id=1", [])
            .unwrap();
        assert!(worker.notify());
        assert!(worker.shutdown());
        assert_eq!(
            reader
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |r| r.get(0))
                .unwrap(),
            "unchanged"
        );
        let (_, frames, copied) = status(&writer);
        assert!(copied < frames);
        writer
            .execute("UPDATE sample SET value='after drain' WHERE id=1", [])
            .unwrap();
        reader.execute_batch("ROLLBACK").unwrap();
        drop(reader);
        let mut final_worker = CheckpointWorker::start(fixture.maintenance()).unwrap();
        assert!(final_worker.shutdown());
        let (_, frames, copied) = status(&writer);
        assert_eq!(frames, copied);
        assert_eq!(
            writer
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |r| r.get(0))
                .unwrap(),
            "after drain"
        );
    }
    #[test]
    fn scheduled_empty_wal_stays_healthy_but_nonwal_remains_a_failure() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let maintenance = fixture.maintenance();
        let mut changed = 0;
        assert!(matches!(
            scheduled(&maintenance, &mut changed, false),
            Attempt::CompleteOrPartial
        ));
        let memory = Connection::open_in_memory().unwrap();
        assert!(matches!(
            scheduled(&memory, &mut changed, false),
            Attempt::Failed
        ));
    }

    #[test]
    fn real_worker_preserves_append_across_multi_second_gesture_pauses() {
        let fixture = Fixture::new();
        let writer = fixture.writer();
        writer
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut worker = CheckpointWorker::start(fixture.maintenance()).unwrap();
        let mut previous = 0;
        for index in 0..3 {
            writer
                .execute(
                    "UPDATE sample SET value=?1 WHERE id=1",
                    [format!("gesture {index}")],
                )
                .unwrap();
            assert!(worker.notify());
            thread::sleep(Duration::from_millis(2100));
            let (_, logged, copied) = status(&writer);
            assert!(
                logged > previous,
                "an ordinary gesture pause must not reset the WAL"
            );
            assert_eq!(
                copied, 0,
                "the quiet policy must not checkpoint after only 2.1s"
            );
            previous = logged;
            assert_eq!(
                writer
                    .pragma_query_value::<i64, _>(None, "synchronous", |row| row.get(0))
                    .unwrap(),
                2
            );
        }
        assert!(worker.shutdown());
        let (_, logged, copied) = status(&writer);
        assert_eq!(logged, copied);
        assert_eq!(
            writer
                .query_row::<String, _, _>("SELECT value FROM sample WHERE id=1", [], |row| row
                    .get(0))
                .unwrap(),
            "gesture 2"
        );
    }
}
