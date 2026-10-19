// SPDX-License-Identifier: MPL-2.0
//! Opt-in input facts, separate from timing receipts and their 128/16 bounds.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
};

const MAX_BYTES: usize = 8192;
const MAX_BATCH: usize = 16;
const MAX_ROWS: usize = 256;
static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Row {
    sequence: u16,
    at_micros: u64,
    phase: u8,
    target: u8,
    active: u8,
    button: i8,
    buttons: u8,
    pointer: u8,
    focused: bool,
    flags: u16,
    code: u8,
    count: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    version: u8,
    rows: Vec<Row>,
}
#[derive(Serialize)]
struct Record<'a> {
    version: u8,
    kind: &'static str,
    rows: &'a [Row],
}
struct Sink {
    file: File,
    sequence: u16,
    at_micros: u64,
    rows: usize,
    failed: bool,
}

fn authorize(label: &str, enabled: bool) -> Result<(), String> {
    if label != "space" {
        return Err("此窗口不能提交输入诊断".into());
    }
    if !enabled {
        return Err("输入诊断未启用".into());
    }
    Ok(())
}
fn parse(payload: &str) -> Result<Batch, String> {
    if payload.len() > MAX_BYTES {
        return Err("输入诊断超出限制".into());
    }
    let value: Batch = serde_json::from_str(payload).map_err(|_| "输入诊断格式无效")?;
    if value.version != 1 || value.rows.is_empty() || value.rows.len() > MAX_BATCH {
        return Err("输入诊断超出限制".into());
    }
    for (index, row) in value.rows.iter().enumerate() {
        let valid_code = match row.phase {
            1..=6 | 10 | 12 | 15 => row.code == 0,
            11 => (1..=7).contains(&row.code),
            13 => (1..=6).contains(&row.code),
            14 => (1..=4).contains(&row.code),
            16..=17 => row.code <= 1,
            20 => row.code <= 3,
            21 => row.code <= 10,
            22 => row.code <= 6,
            23 => row.code <= 7,
            24 => row.code <= 5,
            _ => false,
        };
        if !valid_code
            || row.sequence == 0
            || usize::from(row.sequence) > MAX_ROWS
            || row.at_micros > 600_000_000
            || row.target > 10
            || row.active > 10
            || !(-1..=5).contains(&row.button)
            || row.buttons > 31
            || row.pointer > 3
            || row.flags > 4095
            || row.count > 4096
            || index > 0
                && (row.sequence <= value.rows[index - 1].sequence
                    || row.at_micros < value.rows[index - 1].at_micros)
        {
            return Err("输入诊断数值无效".into());
        }
    }
    Ok(value)
}
impl Sink {
    fn create(directory: &Path) -> Option<Self> {
        let path = directory.join(format!(
            "MewuAI-drawing-diagnostics-{}-{}.jsonl",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        Some(Self {
            file: OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .ok()?,
            sequence: 0,
            at_micros: 0,
            rows: 0,
            failed: false,
        })
    }
    fn write(&mut self, batch: Batch) -> Result<(), String> {
        if self.failed
            || self.rows + batch.rows.len() > MAX_ROWS
            || batch.rows[0].sequence <= self.sequence
            || batch.rows[0].at_micros < self.at_micros
        {
            return Err("输入诊断已结束".into());
        }
        let mut encoded = serde_json::to_vec(&Record {
            version: 1,
            kind: "drawingInput",
            rows: &batch.rows,
        })
        .map_err(|_| "输入诊断不可用")?;
        if encoded.len() > MAX_BYTES {
            return Err("输入诊断超出限制".into());
        }
        encoded.push(b'\n');
        if self.file.write_all(&encoded).is_err() {
            self.failed = true;
            return Err("输入诊断不可用".into());
        }
        let last = batch.rows.last().unwrap();
        self.sequence = last.sequence;
        self.at_micros = last.at_micros;
        self.rows += batch.rows.len();
        Ok(())
    }
}
fn record(
    label: &str,
    enabled: bool,
    payload: &str,
    sink: &Mutex<Option<Sink>>,
    directory: &Path,
) -> Result<(), String> {
    authorize(label, enabled)?;
    let batch = parse(payload)?;
    let mut guard = sink.lock().map_err(|_| "输入诊断不可用")?;
    if guard.is_none() {
        *guard = Sink::create(directory);
    }
    guard.as_mut().ok_or("输入诊断不可用")?.write(batch)
}

#[tauri::command]
pub(crate) async fn record_drawing_diagnostics(
    window: tauri::WebviewWindow,
    payload: String,
) -> Result<(), String> {
    authorize(window.label(), crate::interaction_performance::enabled())?;
    // Byte admission precedes worker dispatch; no host/Store lock is involved.
    if payload.len() > MAX_BYTES {
        return Err("输入诊断超出限制".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        record(
            window.label(),
            crate::interaction_performance::enabled(),
            &payload,
            SINK.get_or_init(|| Mutex::new(None)),
            &std::env::temp_dir(),
        )
    })
    .await
    .map_err(|_| "输入诊断中断")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "MewuAI-drawing-diagnostic-test-{}",
                uuid::Uuid::new_v4()
            )))
        }
        fn create(&self) {
            std::fs::create_dir(&self.0).unwrap();
        }
        fn records(&self) -> Vec<Value> {
            std::fs::read_dir(&self.0)
                .unwrap()
                .flat_map(|entry| {
                    std::fs::read_to_string(entry.unwrap().path())
                        .unwrap()
                        .lines()
                        .map(|line| serde_json::from_str(line).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }
    fn row(sequence: u16) -> Value {
        json!({"sequence":sequence,"atMicros":u64::from(sequence)*100,"phase":10,"target":1,"active":3,
            "button":0,"buttons":1,"pointer":1,"focused":true,"flags":16,"code":0,"count":0})
    }
    fn payload(rows: Vec<Value>) -> String {
        json!({"version":1,"rows":rows}).to_string()
    }
    #[test]
    fn default_off_and_foreign_window_admission_never_create_or_parse() {
        let fixture = Fixture::new();
        let sink = Mutex::new(None);
        assert!(record("space", false, "not json", &sink, &fixture.0).is_err());
        for label in ["settings", "pin-test", "frozen", ""] {
            assert!(record(label, true, "not json", &sink, &fixture.0).is_err());
        }
        assert!(!fixture.0.exists());
        assert!(sink.lock().unwrap().is_none());
    }
    #[test]
    fn fixed_numeric_rows_are_written_without_any_application_content() {
        let fixture = Fixture::new();
        fixture.create();
        let sink = Mutex::new(None);
        record(
            "space",
            true,
            &payload(vec![row(1), row(2)]),
            &sink,
            &fixture.0,
        )
        .unwrap();
        let rows = fixture.records();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0],
            json!({"version":1,"kind":"drawingInput","rows":[row(1),row(2)]})
        );
    }
    #[test]
    fn window_snap_observations_keep_the_fixed_schema_code_and_window_limits() {
        for (phase, maximum) in [(20, 3), (21, 10), (22, 6), (23, 7), (24, 5)] {
            for code in 0..=maximum {
                let mut observed = row(1);
                observed["phase"] = json!(phase);
                observed["code"] = json!(code);
                assert!(parse(&payload(vec![observed])).is_ok());
            }
            for target in [8, 9, 10] {
                let mut observed = row(1);
                observed["phase"] = json!(phase);
                observed["target"] = json!(target);
                observed["active"] = json!(target);
                assert!(parse(&payload(vec![observed])).is_ok());
            }
            let mut invalid = row(1);
            invalid["phase"] = json!(phase);
            invalid["code"] = json!(maximum + 1);
            assert!(parse(&payload(vec![invalid])).is_err());
            let mut observed = row(1);
            observed["phase"] = json!(phase);
            let fixture = Fixture::new();
            let sink = Mutex::new(None);
            for label in ["settings", "pin-test", "frozen", ""] {
                assert!(record(
                    label,
                    true,
                    &payload(vec![observed.clone()]),
                    &sink,
                    &fixture.0
                )
                .is_err());
            }
            assert!(record("space", false, &payload(vec![observed]), &sink, &fixture.0).is_err());
            assert!(!fixture.0.exists());
        }
    }
    #[test]
    fn unknown_content_and_byte_or_batch_budgets_are_rejected_before_file_creation() {
        let fixture = Fixture::new();
        let sink = Mutex::new(None);
        let mut content = row(1);
        content["text"] = json!("forbidden");
        for value in [
            payload(vec![content]),
            json!({"version":1,"rows":[row(1)],"url":"forbidden"}).to_string(),
            payload(vec![]),
            payload((1..=17).map(row).collect()),
            " ".repeat(MAX_BYTES + 1),
        ] {
            assert!(record("space", true, &value, &sink, &fixture.0).is_err());
        }
        assert!(!fixture.0.exists());
        assert!(sink.lock().unwrap().is_none());
    }
    #[test]
    fn invalid_numbers_phases_and_nonmonotonic_batches_never_reserve_rows() {
        let fixture = Fixture::new();
        fixture.create();
        let sink = Mutex::new(None);
        for (field, value) in [
            ("sequence", json!(0)),
            ("sequence", json!(257)),
            ("atMicros", json!(600_000_001u64)),
            ("phase", json!(7)),
            ("phase", json!(255)),
            ("target", json!(11)),
            ("active", json!(11)),
            ("button", json!(-2)),
            ("buttons", json!(32)),
            ("pointer", json!(4)),
            ("flags", json!(4096)),
            ("code", json!(31)),
            ("count", json!(4097)),
            ("focused", json!("true")),
            ("button", json!(0.5)),
        ] {
            let mut value_row = row(1);
            value_row[field] = value;
            assert!(
                record("space", true, &payload(vec![value_row]), &sink, &fixture.0).is_err(),
                "{field}"
            );
        }
        assert!(record(
            "space",
            true,
            &payload(vec![row(2), row(1)]),
            &sink,
            &fixture.0
        )
        .is_err());
        let mut reverse_time = row(2);
        reverse_time["atMicros"] = json!(1);
        assert!(record(
            "space",
            true,
            &payload(vec![row(1), reverse_time]),
            &sink,
            &fixture.0
        )
        .is_err());
        assert!(sink.lock().unwrap().is_none());
        record("space", true, &payload(vec![row(1)]), &sink, &fixture.0).unwrap();
        assert_eq!(fixture.records().len(), 1);
    }
    #[test]
    fn native_total_is_bounded_and_replayed_batches_cannot_append_again() {
        let fixture = Fixture::new();
        fixture.create();
        let sink = Mutex::new(None);
        for first in (1..=256).step_by(16) {
            record(
                "space",
                true,
                &payload((first..first + 16).map(row).collect()),
                &sink,
                &fixture.0,
            )
            .unwrap();
        }
        assert!(record("space", true, &payload(vec![row(256)]), &sink, &fixture.0).is_err());
        let records = fixture.records();
        assert_eq!(records.len(), 16);
        assert_eq!(
            records
                .iter()
                .map(|record| record["rows"].as_array().unwrap().len())
                .sum::<usize>(),
            MAX_ROWS
        );
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 1);
    }
    #[test]
    fn write_failure_is_terminal_and_cannot_retry_into_more_files() {
        let fixture = Fixture::new();
        fixture.create();
        let file = fixture.0.join("readonly.jsonl");
        std::fs::write(&file, b"").unwrap();
        let mut sink = Sink {
            file: File::open(&file).unwrap(),
            sequence: 0,
            at_micros: 0,
            rows: 0,
            failed: false,
        };
        assert!(sink.write(parse(&payload(vec![row(1)])).unwrap()).is_err());
        assert!(sink.failed);
        assert!(sink.write(parse(&payload(vec![row(2)])).unwrap()).is_err());
        assert_eq!(std::fs::metadata(&file).unwrap().len(), 0);
    }
}
