// SPDX-License-Identifier: MPL-2.0
//! History exports are document operations, independent of an installed plugin.
//! The caller supplies message identities, never paths, HTML or cell payloads.
use crate::{table_clipboard, table_document, table_staging, Host};
use serde::Deserialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tokio::sync::Semaphore;

static REQUESTS: Semaphore = Semaphore::const_new(16);
static WORKERS: Semaphore = Semaphore::const_new(2);
pub(crate) static CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Table,
    Markdown,
    Csv,
    Tsv,
    Png,
}

fn owner(label: &str) -> Result<(), String> {
    if label != "space" {
        return Err("此窗口不能导出回答".into());
    }
    Ok(())
}
fn read(app: &AppHandle, scene: &str, message: &str) -> Result<String, String> {
    let host = app.state::<Host>();
    let engine = host.lock()?;
    host.exit.ensure_running()?;
    engine
        .store
        .completed_message_text(scene, message)
        .map_err(|e| e.to_string())
}
fn unchanged(app: &AppHandle, scene: &str, message: &str, text: &str) -> Result<(), String> {
    if read(app, scene, message)? != text {
        return Err("回答已变更，请重试".into());
    }
    Ok(())
}

fn parse_answer(text: &str) -> Result<table_document::TableMessage, String> {
    table_document::parse(&crate::inline_reasoning::split_text(text).text)
}

#[tauri::command]
pub async fn get_message_tables(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    message_id: String,
) -> Result<table_document::TableMessage, String> {
    owner(window.label())?;
    let request = REQUESTS
        .try_acquire()
        .map_err(|_| "表格正在处理，请稍后重试")?;
    let worker = WORKERS.acquire().await.map_err(|_| "表格处理不可用")?;
    let text = read(&app, &scene_id, &message_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let result = parse_answer(&text)?;
        unchanged(&app, &scene_id, &message_id, &text)?;
        Ok(result)
    })
    .await
    .map_err(|_| "表格读取中断")?
}

#[tauri::command]
pub async fn export_message_table(
    app: AppHandle,
    window: WebviewWindow,
    scene_id: String,
    message_id: String,
    table_index: usize,
    format: ExportFormat,
) -> Result<(), String> {
    owner(window.label())?;
    let request = REQUESTS
        .try_acquire()
        .map_err(|_| "表格正在处理，请稍后重试")?;
    let worker = WORKERS.acquire().await.map_err(|_| "表格处理不可用")?;
    let text = read(&app, &scene_id, &message_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (_request, _worker) = (request, worker);
        let parsed = parse_answer(&text)?;
        let table = parsed.tables.get(table_index).ok_or("表格不存在")?;
        match format {
            ExportFormat::Markdown | ExportFormat::Csv | ExportFormat::Tsv => {
                let output = match format {
                    ExportFormat::Markdown => table.markdown(),
                    ExportFormat::Csv => table.delimited(','),
                    _ => table.delimited('\t'),
                };
                let _clipboard = CLIPBOARD.lock().map_err(|_| "剪贴板正在使用")?;
                unchanged(&app, &scene_id, &message_id, &text)?;
                arboard::Clipboard::new()
                    .map_err(|_| "无法打开剪贴板")?
                    .set_text(output)
                    .map_err(|_| "无法复制表格")?;
            }
            ExportFormat::Png => {
                let image = table_document::render(table)?;
                unchanged(&app, &scene_id, &message_id, &text)?;
                if let Some(path) = rfd::FileDialog::new()
                    .set_parent(&window)
                    .add_filter("PNG", &["png"])
                    .set_file_name("表格.png")
                    .save_file()
                {
                    unchanged(&app, &scene_id, &message_id, &text)?;
                    save_png_atomic(&image, &path, || {
                        unchanged(&app, &scene_id, &message_id, &text)
                    })?;
                }
            }
            ExportFormat::Table => {
                let image = table_document::render(table)?;
                let _clipboard = CLIPBOARD.lock().map_err(|_| "剪贴板正在使用")?;
                unchanged(&app, &scene_id, &message_id, &text)?;
                let path = table_staging::stage(&app.state::<Host>().root, &image)?;
                // Keep this immutable file even if clipboard publication fails:
                // Windows may have published CF_HDROP before reporting failure.
                unchanged(&app, &scene_id, &message_id, &text)?;
                table_clipboard::write(table_clipboard::PreparedTableClipboard {
                    html: table.html(),
                    markdown: table.markdown(),
                    csv: table.delimited(','),
                    image,
                    png_path: path,
                })?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "表格导出中断")?
}

fn save_png_atomic(
    image: &image::RgbaImage,
    destination: &Path,
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    use image::ImageEncoder;
    write_atomic(
        destination,
        |file| {
            image::codecs::png::PngEncoder::new(file)
                .write_image(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|_| "无法编码表格图片".to_string())
        },
        before_commit,
    )
}

struct PendingExport(Option<PathBuf>);

impl Drop for PendingExport {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            // Exact UUID temporary created exclusively by this operation. Never
            // remove the selected destination, even after a failed replacement.
            let _ = std::fs::remove_file(path);
        }
    }
}

fn write_atomic(
    destination: &Path,
    encode: impl FnOnce(&mut File) -> Result<(), String>,
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !destination.is_absolute() || destination.file_name().is_none() {
        return Err("表格图片保存位置无效".into());
    }
    let parent = destination
        .parent()
        .ok_or("表格图片保存位置无效")?
        .canonicalize()
        .map_err(|_| "表格图片保存目录不可用")?;
    let destination = parent.join(destination.file_name().ok_or("表格图片保存位置无效")?);
    let temporary = parent.join(format!(".mewu-table-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "无法创建表格图片临时文件")?;
    let mut pending = PendingExport(Some(temporary.clone()));
    let encoded = (|| {
        encode(&mut file)?;
        file.flush().map_err(|_| "无法写入完整表格图片")?;
        file.sync_all().map_err(|_| "无法保存完整表格图片")?;
        Ok::<(), String>(())
    })();
    drop(file);
    encoded?;
    // Recheck exit/scene/message after encoding as well as after the file dialog.
    before_commit()?;
    // Same-directory rename replaces atomically without a cross-volume copy.
    // Rust uses MoveFileExW / FileRenameInfoEx on Windows; the existing file is
    // never opened/truncated by us. This is not a promise against disk power loss.
    // https://doc.rust-lang.org/std/fs/fn.rename.html
    std::fs::rename(&temporary, &destination)
        .map_err(|_| "无法完成表格图片保存，原文件未被覆盖")?;
    pending.0 = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_reasoning_tables_are_not_answer_tables_or_export_sources() {
        let answer = "| X | Y |\n|---|---|\n|1|2|";
        let raw =
            format!("<think>| Private | Thought |\n|---|---|\n|SECRET|SECRET|</think>{answer}");
        assert_eq!(
            serde_json::to_value(parse_answer(&raw).unwrap()).unwrap(),
            serde_json::to_value(table_document::parse(answer).unwrap()).unwrap()
        );
        assert!(parse_answer("<think>| X | Y |\n|---|---|\n|1|2|")
            .unwrap()
            .tables
            .is_empty());
    }

    #[test]
    fn only_space_can_export_and_only_declared_formats_deserialize() {
        assert!(owner("space").is_ok());
        for label in ["settings", "frozen-widget", "recording-controls", ""] {
            assert!(owner(label).is_err());
        }
        for value in ["table", "markdown", "csv", "tsv", "png"] {
            assert!(serde_json::from_value::<ExportFormat>(serde_json::json!(value)).is_ok());
        }
        for value in ["html", "file", "../../file", "PNG"] {
            assert!(serde_json::from_value::<ExportFormat>(serde_json::json!(value)).is_err());
        }
    }

    struct Directory {
        root: PathBuf,
        parent: PathBuf,
    }
    impl Directory {
        fn new() -> Self {
            let parent = std::env::temp_dir().canonicalize().unwrap();
            let root = parent.join(format!("mewu-table-export-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            Self { root, parent }
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            if let Ok(root) = self.root.canonicalize() {
                if root == self.root
                    && root.parent() == Some(self.parent.as_path())
                    && root.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("mewu-table-export-test-")
                    })
                {
                    let _ = std::fs::remove_dir_all(root);
                }
            }
        }
    }

    #[test]
    fn completed_png_replaces_existing_file_only_after_successful_encoding() {
        let directory = Directory::new();
        let destination = directory.root.join("用户表格.png");
        let old = b"previous user content";
        std::fs::write(&destination, old).unwrap();
        let image = image::RgbaImage::from_pixel(5, 3, image::Rgba([10, 30, 60, 255]));
        save_png_atomic(&image, &destination, || {
            assert_eq!(std::fs::read(&destination).unwrap(), old);
            Ok(())
        })
        .unwrap();
        assert_eq!(image::open(&destination).unwrap().to_rgba8(), image);
        assert_eq!(std::fs::read_dir(&directory.root).unwrap().count(), 1);
    }

    #[test]
    fn failed_encoder_or_late_cancellation_preserves_previous_file_and_cleans_temporary() {
        let directory = Directory::new();
        let destination = directory.root.join("previous.png");
        let old = b"previous user content";
        std::fs::write(&destination, old).unwrap();
        let failed = write_atomic(
            &destination,
            |file| {
                file.write_all(b"partial PNG bytes").unwrap();
                Err("injected encoder/write failure".into())
            },
            || panic!("must not commit failed encoding"),
        );
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), old);
        assert_eq!(std::fs::read_dir(&directory.root).unwrap().count(), 1);
        let image = image::RgbaImage::from_pixel(5, 3, image::Rgba([10, 30, 60, 255]));
        assert!(
            save_png_atomic(&image, &destination, || Err("已取消退出期间导出".into())).is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), old);
        assert_eq!(std::fs::read_dir(&directory.root).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_never_deletes_destination_or_leaves_own_partial_file() {
        let directory = Directory::new();
        let destination = directory.root.join("existing-directory.png");
        std::fs::create_dir(&destination).unwrap();
        let inner = destination.join("keep.txt");
        std::fs::write(&inner, b"keep").unwrap();
        let image = image::RgbaImage::from_pixel(5, 3, image::Rgba([10, 30, 60, 255]));
        assert!(save_png_atomic(&image, &destination, || Ok(())).is_err());
        assert_eq!(std::fs::read(&inner).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&directory.root).unwrap().count(), 1);
    }
}
