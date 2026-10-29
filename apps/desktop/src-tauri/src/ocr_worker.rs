// SPDX-License-Identifier: MPL-2.0
//! Isolate Apple Vision's process-local model state from the long-lived GUI.
//! Only owned RGBA pixels cross anonymous pipes; no image path or host handle
//! is accepted. The existing process-group reaper owns IO and capacity until
//! actual exit, including cancellation, malformed output and native hangs.
use crate::owned_process::ManagedProcess;
use mewu_core::{validate_ocr_document, OcrDocument};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"MEWUOCR1";
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_SIDE: u32 = 16_384;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(35);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, String> {
        ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map(|_| Self)
            .map_err(|_| "已有文字识别正在进行，请稍后重试".into())
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

fn pixel_length(width: u32, height: u32) -> Result<usize, String> {
    let count = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || count > MAX_PIXELS {
        return Err("文字识别图片为空或超过 3200 万像素".into());
    }
    Ok((count * 4) as usize)
}
fn write_pixels(pixels: image::RgbaImage, out: &mut impl Write) -> Result<(), String> {
    let width = pixels.width();
    let height = pixels.height();
    let bytes = Zeroizing::new(pixels.into_raw());
    if bytes.len() != pixel_length(width, height)? {
        return Err("文字识别图片缓冲区大小无效".into());
    }
    out.write_all(MAGIC)
        .and_then(|_| out.write_all(&width.to_le_bytes()))
        .and_then(|_| out.write_all(&height.to_le_bytes()))
        .and_then(|_| out.write_all(&bytes))
        .and_then(|_| out.flush())
        .map_err(|_| "文字识别连接已断开".into())
}
fn read_pixels(input: &mut impl Read) -> Result<image::RgbaImage, String> {
    let mut header = [0u8; 16];
    input
        .read_exact(&mut header)
        .map_err(|_| "文字识别输入未完成")?;
    if &header[..8] != MAGIC {
        return Err("文字识别输入协议无效".into());
    }
    let width = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let mut bytes = Zeroizing::new(vec![0; pixel_length(width, height)?]);
    input
        .read_exact(&mut bytes)
        .map_err(|_| "文字识别图片未完成")?;
    let mut trailing = [0u8; 1];
    if input
        .read(&mut trailing)
        .map_err(|_| "文字识别输入未结束")?
        != 0
    {
        return Err("文字识别输入存在额外内容".into());
    }
    image::RgbaImage::from_raw(width, height, std::mem::take(&mut *bytes))
        .ok_or_else(|| "文字识别图片无效".into())
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Completed { document: OcrDocument },
    Failed {},
}
fn read_response(input: &mut impl Read) -> Result<Response, String> {
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take(MAX_OUTPUT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取文字识别结果")?;
    if bytes.len() > MAX_OUTPUT {
        return Err("文字识别结果超过限制".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "文字识别结果格式无效".into())
}
fn validate_response(response: Response, width: u32, height: u32) -> Result<OcrDocument, String> {
    let Response::Completed { document } = response else {
        return Err("本机文字识别失败，请重试".into());
    };
    validate_ocr_document(&document).map_err(|error| error.to_string())?;
    if document.width != width || document.height != height {
        return Err("文字识别结果尺寸不匹配".into());
    }
    Ok(document)
}

#[cfg(target_os = "macos")]
pub(crate) fn recognize(
    bitmap: image::RgbaImage,
    cancel: Arc<AtomicBool>,
) -> Result<OcrDocument, String> {
    let width = bitmap.width();
    let height = bitmap.height();
    if bitmap.as_raw().len() != pixel_length(width, height)? {
        return Err("文字识别图片缓冲区大小无效".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("文字识别已取消".into());
    }
    let permit = Permit::acquire()?;
    let started = Instant::now();
    let exe = std::env::current_exe().map_err(|_| "文字识别程序不可用")?;
    let mut command = Command::new(exe);
    command
        .arg("--ocr-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear();
    let mut process =
        ManagedProcess::spawn(&mut command, permit).map_err(|_| "无法启动文字识别")?;
    let mut stdin = process
        .child_mut()
        .stdin
        .take()
        .ok_or("文字识别输入不可用")?;
    let mut stdout = process
        .child_mut()
        .stdout
        .take()
        .ok_or("文字识别输出不可用")?;
    let (writer_tx, writer_rx) = mpsc::sync_channel(1);
    process.track_io(
        std::thread::Builder::new()
            .name("mewu-ocr-input".into())
            .spawn(move || {
                let result = write_pixels(bitmap, &mut stdin);
                drop(stdin);
                let _ = writer_tx.send(result);
            })
            .map_err(|_| "文字识别输入不可用")?,
    );
    let (reader_tx, reader_rx) = mpsc::sync_channel(1);
    process.track_io(
        std::thread::Builder::new()
            .name("mewu-ocr-output".into())
            .spawn(move || {
                let result = read_response(&mut stdout);
                drop(stdout);
                let _ = reader_tx.send(result);
            })
            .map_err(|_| "文字识别输出不可用")?,
    );
    let mut written = false;
    let mut response = None;
    let mut status = None;
    loop {
        if cancel.load(Ordering::Acquire) {
            process.kill();
            return Err("文字识别已取消".into());
        }
        if started.elapsed() >= TIMEOUT {
            process.kill();
            return Err("文字识别超时，请缩小选区后重试".into());
        }
        if !written {
            match writer_rx.try_recv() {
                Ok(result) => {
                    result?;
                    written = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => return Err("文字识别输入中断".into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if response.is_none() {
            match reader_rx.try_recv() {
                Ok(result) => response = Some(result?),
                Err(mpsc::TryRecvError::Disconnected) => return Err("文字识别输出中断".into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if status.is_none() {
            status = process.try_wait().map_err(|_| "无法确认文字识别状态")?;
        }
        if written && response.is_some() && status.is_some() && process.fully_exited() {
            if !status.unwrap().success() {
                return Err("本机文字识别未完成".into());
            }
            // Recheck after real process/IO completion, before returning any text.
            if cancel.load(Ordering::Acquire) {
                return Err("文字识别已取消".into());
            }
            return validate_response(response.unwrap(), width, height);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn worker_main() -> i32 {
    let result = read_pixels(&mut std::io::stdin().lock())
        .and_then(|bitmap| crate::ocr::recognize_native(bitmap, Arc::new(AtomicBool::new(false))));
    let response = match result {
        Ok(document) => Response::Completed { document },
        Err(_) => Response::Failed {},
    };
    let Ok(bytes) = serde_json::to_vec(&response) else {
        return 1;
    };
    if bytes.len() > MAX_OUTPUT {
        return 1;
    }
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&bytes)
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return 1;
    }
    if matches!(response, Response::Completed { .. }) {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_rgba_round_trip_and_strict_message_boundary() {
        let pixels =
            image::RgbaImage::from_raw(2, 1, vec![0, 255, 128, 255, 255, 0, 10, 30]).unwrap();
        let mut bytes = Vec::new();
        write_pixels(pixels.clone(), &mut bytes).unwrap();
        assert_eq!(read_pixels(&mut &bytes[..]).unwrap(), pixels);
        for n in [0, 7, 15, bytes.len() - 1] {
            assert!(read_pixels(&mut &bytes[..n]).is_err());
        }
        bytes.push(1);
        assert!(read_pixels(&mut &bytes[..]).is_err());
    }
    #[test]
    fn rejects_oversized_geometry_before_allocating_or_reading_pixels() {
        for (width, height) in [
            (0_u32, 1_u32),
            (1, 0),
            (16385, 1),
            (16384, 16384),
            (u32::MAX, u32::MAX),
        ] {
            let mut header = MAGIC.to_vec();
            header.extend(width.to_le_bytes());
            header.extend(height.to_le_bytes());
            assert!(read_pixels(&mut &header[..]).is_err());
        }
    }
    #[test]
    fn result_protocol_rejects_trailing_unknown_and_oversized_messages() {
        assert!(matches!(
            read_response(&mut &b"{\"type\":\"failed\"}"[..]).unwrap(),
            Response::Failed {}
        ));
        for bytes in [
            b"{\"type\":\"failed\",\"path\":\"x\"}".as_slice(),
            b"{\"type\":\"failed\"}{}".as_slice(),
            b"".as_slice(),
        ] {
            assert!(read_response(&mut &bytes[..]).is_err());
        }
        assert!(read_response(&mut &vec![b' '; MAX_OUTPUT + 1][..]).is_err());
        assert!(validate_response(Response::Failed {}, 1, 1).is_err());
    }

    #[test]
    fn accepts_only_valid_documents_for_the_exact_owned_source_dimensions() {
        let document = || serde_json::from_value::<OcrDocument>(serde_json::json!({
            "width": 2, "height": 1, "engine": "apple-vision", "language": "en-US", "lines": []
        })).unwrap();
        assert!(validate_response(Response::Completed { document: document() }, 2, 1).is_ok());
        assert!(validate_response(Response::Completed { document: document() }, 1, 2).is_err());
        let mut invalid = document(); invalid.engine.clear();
        assert!(validate_response(Response::Completed { document: invalid }, 2, 1).is_err());
    }

    #[test]
    fn capacity_is_retained_until_the_actual_owned_permit_is_released() {
        let first = Permit::acquire().unwrap(); let second = Permit::acquire().unwrap();
        assert!(Permit::acquire().is_err());
        drop(first);
        let replacement = Permit::acquire().unwrap();
        assert!(Permit::acquire().is_err());
        drop(second); drop(replacement);
        assert_eq!(ACTIVE.load(Ordering::Acquire), 0);
    }
}
