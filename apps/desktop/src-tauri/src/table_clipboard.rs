// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0

//! Eager, bounded Windows clipboard publication. The caller supplies escaped HTML,
//! canonical table text and an immutable, app-owned PNG matching `image`. This module
//! does not interpret model output, add formula prefixes, or delete the PNG. Keep the
//! PNG after errors too: a failed native call can leave already-published formats.
//!
//! Win32 has no multi-format clipboard transaction. Preparation failures leave the
//! clipboard untouched; publication failures attempt to clear the partial batch,
//! but cannot restore what another application had copied before EmptyClipboard.
//! The separate video-file entry point commits at CF_HDROP and never clears a
//! successfully published file when an auxiliary format fails.
//! Sources: Microsoft Learn /windows/win32/{dataxchg/html-clipboard-format,
//! api/winuser/nf-winuser-setclipboarddata,shell/clipboard}; dotnet/wpf v9.0.0
//! PresentationCore/System/Windows/{DataFormats.cs,dataobject.cs} (CSV is CP_ACP).

use image::RgbaImage;
use std::path::PathBuf;

pub struct PreparedTableClipboard {
    /// Escaped table fragment, without a CF_HTML header. Content policy is the host's.
    pub html: String,
    pub markdown: String,
    pub csv: String,
    pub image: RgbaImage,
    /// Absolute, immutable app-owned PNG. Its lifetime must outlast clipboard use.
    pub png_path: PathBuf,
}

pub fn write(prepared: PreparedTableClipboard) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::write(prepared)
    }
    #[cfg(not(windows))]
    {
        let _ = prepared;
        Err("完整表格复制目前仅支持 Windows".into())
    }
}

/// A successful primary file-drop format is permanent publication, even if an
/// auxiliary format or native close later fails. The host must retain its file.
#[derive(Debug)]
pub(crate) enum FileDropPublication {
    Unpublished { error: String },
    Published { warning: Option<String> },
}

/// `path` belongs to a verified, independently copied host file. Keep its lease
/// through this call; stable files must not be deleted on an unknown outcome.
pub(crate) fn publish_file_drop(
    path: &std::path::Path,
    accept: impl FnOnce() -> Result<(), String>,
) -> FileDropPublication {
    #[cfg(windows)]
    {
        windows::publish_file_drop(path, accept)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, accept);
        FileDropPublication::Unpublished {
            error: "视频文件复制目前仅支持 Windows".into(),
        }
    }
}

const MAX_SIDE: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 128 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const MAX_PATH_UNITS: usize = 32_760;

fn image_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("表格图片尺寸超过复制限制".into());
    }
    Ok(())
}

fn text_size(text: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Err("表格复制文本为空、过长或包含空字符".into());
    }
    Ok(())
}

fn allocated_bytes(size: usize) -> Result<Vec<u8>, String> {
    if size > MAX_PAYLOAD_BYTES {
        return Err("表格剪贴板数据超过内存限制".into());
    }
    let mut data = Vec::new();
    data.try_reserve_exact(size)
        .map_err(|_| "没有足够内存准备表格复制".to_string())?;
    data.resize(size, 0);
    Ok(data)
}

fn html_clipboard(fragment: &str) -> Result<Vec<u8>, String> {
    text_size(fragment)?;
    const PREFIX: &str = "<html><body><!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment--></body></html>";
    fn header(start: usize, end: usize, fragment_start: usize, fragment_end: usize) -> String {
        format!("Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{fragment_start:010}\r\nEndFragment:{fragment_end:010}\r\n")
    }
    // All offsets count UTF-8 bytes, including the ASCII header, excluding its NUL.
    let start = header(0, 0, 0, 0).len();
    let fragment_start = start + PREFIX.len();
    let fragment_end = fragment_start + fragment.len();
    let end = fragment_end + SUFFIX.len();
    let mut result = allocated_bytes(end + 1)?;
    result[..start].copy_from_slice(header(start, end, fragment_start, fragment_end).as_bytes());
    result[start..fragment_start].copy_from_slice(PREFIX.as_bytes());
    result[fragment_start..fragment_end].copy_from_slice(fragment.as_bytes());
    result[fragment_end..end].copy_from_slice(SUFFIX.as_bytes());
    Ok(result)
}

fn utf8_text(text: &str) -> Result<Vec<u8>, String> {
    text_size(text)?;
    let mut bytes = allocated_bytes(text.len() + 1)?;
    bytes[..text.len()].copy_from_slice(text.as_bytes());
    Ok(bytes)
}

fn unicode_text(text: &str) -> Result<Vec<u8>, String> {
    text_size(text)?;
    let mut bytes = allocated_bytes((text.encode_utf16().count() + 1) * 2)?;
    for (index, unit) in text.encode_utf16().enumerate() {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    Ok(bytes)
}

fn dropfiles(path: &[u16]) -> Result<Vec<u8>, String> {
    if path.is_empty() || path.len() > MAX_PATH_UNITS || path.contains(&0) {
        return Err("复制文件路径无效或过长".into());
    }
    // DROPFILES: DWORD pFiles, POINT pt, BOOL fNC, BOOL fWide. It is 20 bytes
    // on both x86 and x64; a single path is followed by TWO UTF-16 NULs.
    let mut bytes = allocated_bytes(20 + (path.len() + 2) * 2)?;
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    for (index, unit) in path.iter().enumerate() {
        bytes[20 + index * 2..22 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    Ok(bytes)
}

fn dib24(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let (width, height) = image.dimensions();
    image_size(width, height)?;
    let stride = (width as usize * 3 + 3) & !3;
    let size = stride
        .checked_mul(height as usize)
        .ok_or("表格位图长度溢出")?;
    let mut bytes = allocated_bytes(40 + size)?;
    bytes[..4].copy_from_slice(&40u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes()); // planes
    bytes[14..16].copy_from_slice(&24u16.to_le_bytes()); // BI_RGB (zero) BGR
    bytes[20..24].copy_from_slice(&(size as u32).to_le_bytes());
    for output_y in 0..height {
        let source_y = height - output_y - 1; // Positive height is bottom-up.
        for x in 0..width {
            let rgba = image.get_pixel(x, source_y).0;
            let alpha = u32::from(rgba[3]);
            let offset = 40 + output_y as usize * stride + x as usize * 3;
            for (channel, source) in [rgba[2], rgba[1], rgba[0]].iter().enumerate() {
                // Standard 24-bit DIB has no alpha. Composite onto the table's white page.
                bytes[offset + channel] =
                    ((u32::from(*source) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
            }
        }
    }
    Ok(bytes)
}

fn validate_png(bytes: &[u8], dimensions: (u32, u32)) -> Result<(), String> {
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("表格图片不是有效的有界 PNG".into());
    }
    let decoded =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png)
            .into_dimensions()
            .map_err(|_| "无法读取表格 PNG 尺寸".to_string())?;
    if decoded != dimensions {
        return Err("表格 PNG 与复制图片尺寸不一致".into());
    }
    Ok(())
}

#[cfg(windows)]
fn read_png(path: &std::path::Path, dimensions: (u32, u32)) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.as_encoded_bytes().eq_ignore_ascii_case(b"png"))
    {
        return Err("表格图片必须是绝对路径的 PNG 文件".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "表格 PNG 文件不可用")?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err("表格 PNG 不能是目录或文件系统链接".into());
    }
    // Reject a final-component reparse substitution and prevent writes/deletes while
    // reading. The caller is responsible for trusted ancestors and immutable storage.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1) // FILE_SHARE_READ only
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
        .open(path)
        .map_err(|_| "无法读取表格 PNG 文件")?;
    let metadata = file.metadata().map_err(|_| "无法检查表格 PNG 文件")?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > MAX_PNG_BYTES as u64
    {
        return Err("表格 PNG 文件无效或超过复制限制".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| "没有足够内存读取表格 PNG")?;
    file.take(MAX_PNG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取表格 PNG 失败")?;
    validate_png(&bytes, dimensions)?;
    Ok(bytes)
}

/// Allocation ownership transfers only on a successful `set`. This small boundary
/// permits failure/cleanup tests without opening or changing the real clipboard.
trait ClipboardSink {
    type Allocation;
    fn clear(&mut self) -> Result<(), String>;
    fn set(&mut self, format: u32, data: &mut Self::Allocation) -> Result<(), String>;
}

trait FileDropSink: ClipboardSink {
    fn close(&mut self) -> Result<(), String>;
}

fn publish_file_drop_parts<S: FileDropSink>(
    sink: &mut S,
    mut primary: (u32, S::Allocation),
    mut preference: (u32, S::Allocation),
    accept: impl FnOnce() -> Result<(), String>,
) -> FileDropPublication {
    let publication = (|| -> Result<Option<String>, String> {
        accept()?;
        sink.clear()?;
        sink.set(primary.0, &mut primary.1)?;
        // Never clear or return Unpublished after the primary transfer succeeds.
        Ok(sink.set(preference.0, &mut preference.1).err())
    })();
    let closed = sink.close();
    match publication {
        Err(error) => FileDropPublication::Unpublished { error },
        Ok(preference_error) => FileDropPublication::Published {
            warning: match (preference_error, closed) {
                (None, Ok(())) => None,
                _ => Some("视频文件已复制，剪贴板收尾未确认".into()),
            },
        },
    }
}

fn publish<S: ClipboardSink>(
    sink: &mut S,
    mut formats: Vec<(u32, S::Allocation)>,
) -> Result<(), String> {
    sink.clear()?;
    for (format, data) in &mut formats {
        if let Err(error) = sink.set(*format, data) {
            // Previously transferred objects are now owned by Windows. Only Windows
            // can free them via EmptyClipboard. Untransferred objects drop locally.
            return match sink.clear() {
                Ok(()) => Err(format!("{error}；本次未完成的剪贴板格式已清除")),
                Err(_) => Err(format!("{error}；剪贴板可能保留部分格式，请重新复制")),
            };
        }
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::marker::PhantomData;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use std::rc::Rc;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, SetLastError, HGLOBAL, HWND};
    use windows_sys::Win32::Globalization::{WideCharToMultiByte, CP_ACP};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE,
    };

    const CF_DIB: u32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;
    static WRITER: Mutex<()> = Mutex::new(());

    fn native_error(action: &str) -> String {
        // Never include clipboard text, image paths or pixel bytes in diagnostics.
        format!("{action}（Windows 错误 {}）", unsafe { GetLastError() })
    }

    fn registered(name: &str) -> Result<u32, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
        if format == 0 {
            Err(native_error("无法注册剪贴板格式"))
        } else {
            Ok(format)
        }
    }

    fn ansi_csv(text: &str) -> Result<Vec<u8>, String> {
        text_size(text)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // WPF DataFormats.CommaSeparatedValue is "CSV", written as ANSI/DBCS.
        // Flags/default-character pointers stay zero/null for ACP=65001 systems too.
        let size = unsafe {
            WideCharToMultiByte(
                CP_ACP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                null_mut(),
                0,
                null(),
                null_mut(),
            )
        };
        if size <= 0 {
            return Err(native_error("无法编码传统 CSV 格式"));
        }
        let mut bytes = allocated_bytes(size as usize + 1)?;
        let written = unsafe {
            WideCharToMultiByte(
                CP_ACP,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                bytes.as_mut_ptr(),
                size,
                null(),
                null_mut(),
            )
        };
        if written != size {
            return Err(native_error("无法编码传统 CSV 格式"));
        }
        Ok(bytes)
    }

    struct GlobalMemory(Option<HGLOBAL>);

    impl GlobalMemory {
        fn new(bytes: &[u8]) -> Result<Self, String> {
            if bytes.is_empty() {
                return Err("不能发布空的剪贴板内存对象".into());
            }
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
            if handle.is_null() {
                return Err(native_error("无法分配剪贴板内存"));
            }
            let allocation = Self(Some(handle));
            let pointer = unsafe { GlobalLock(handle) };
            if pointer.is_null() {
                return Err(native_error("无法访问剪贴板内存"));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                SetLastError(0);
                // Zero means the last lock was released, unless last-error is nonzero.
                if GlobalUnlock(handle) == 0 && GetLastError() != 0 {
                    return Err(native_error("无法解锁剪贴板内存"));
                }
            }
            Ok(allocation)
        }
    }

    impl Drop for GlobalMemory {
        fn drop(&mut self) {
            if let Some(handle) = self.0.take() {
                unsafe { GlobalFree(handle) };
            }
        }
    }

    struct OwnerWindow {
        hwnd: HWND,
        // A window must be destroyed on its creating thread, never moved to an await.
        _thread: PhantomData<Rc<()>>,
    }

    impl OwnerWindow {
        fn new() -> Result<Self, String> {
            // An existing system class needs no custom WndProc or class registration.
            // HWND_MESSAGE creates no visible UI and eager publication needs no pump.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    windows_sys::core::w!("STATIC"),
                    windows_sys::core::w!("Mewu table clipboard"),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    null_mut(),
                    null_mut(),
                    null(),
                )
            };
            if hwnd.is_null() {
                return Err(native_error("无法创建剪贴板所有者"));
            }
            Ok(Self {
                hwnd,
                _thread: PhantomData,
            })
        }
    }

    impl Drop for OwnerWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.hwnd) };
        }
    }

    struct Clipboard {
        open: bool,
        _thread: PhantomData<Rc<()>>,
    }

    impl Clipboard {
        fn open(hwnd: HWND) -> Result<Self, String> {
            for attempt in 0..5 {
                if unsafe { OpenClipboard(hwnd) } != 0 {
                    return Ok(Self {
                        open: true,
                        _thread: PhantomData,
                    });
                }
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            Err(native_error("剪贴板正被占用，请稍后再试"))
        }

        fn close(&mut self) -> Result<(), String> {
            if self.open {
                if unsafe { CloseClipboard() } == 0 {
                    return Err(native_error("剪贴板已写入但关闭失败，请检查后重新复制"));
                }
                self.open = false;
            }
            Ok(())
        }
    }

    impl Drop for Clipboard {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }

    impl ClipboardSink for Clipboard {
        type Allocation = GlobalMemory;

        fn clear(&mut self) -> Result<(), String> {
            if unsafe { EmptyClipboard() } == 0 {
                Err(native_error("无法清空剪贴板"))
            } else {
                Ok(())
            }
        }

        fn set(&mut self, format: u32, data: &mut GlobalMemory) -> Result<(), String> {
            let handle = data.0.ok_or("剪贴板内存已移交")?;
            if unsafe { SetClipboardData(format, handle) }.is_null() {
                Err(native_error("发布剪贴板格式失败"))
            } else {
                data.0 = None; // Windows owns it now; neither free nor modify it.
                Ok(())
            }
        }
    }

    impl FileDropSink for Clipboard {
        fn close(&mut self) -> Result<(), String> {
            Clipboard::close(self)
        }
    }

    pub(super) fn publish_file_drop(
        path: &std::path::Path,
        accept: impl FnOnce() -> Result<(), String>,
    ) -> FileDropPublication {
        let prepared = (|| {
            let writer = WRITER.try_lock().map_err(|_| "剪贴板操作尚未结束")?;
            if !path.is_absolute() {
                return Err("视频文件路径无效".into());
            }
            let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            let primary = (CF_HDROP, GlobalMemory::new(&dropfiles(&wide)?)?);
            let preference = (
                registered("Preferred DropEffect")?,
                GlobalMemory::new(&1u32.to_le_bytes())?,
            );
            let owner = OwnerWindow::new()?;
            let clipboard = Clipboard::open(owner.hwnd)?;
            Ok::<_, String>((writer, owner, clipboard, primary, preference))
        })();
        match prepared {
            Err(error) => FileDropPublication::Unpublished { error },
            Ok((_writer, _owner, mut clipboard, primary, preference)) => {
                // All allocations and file preparation precede Open/Empty. The
                // callback only checks host authority; it must release its locks
                // and never access UI APIs or perform I/O before returning.
                publish_file_drop_parts(&mut clipboard, primary, preference, accept)
            }
        }
    }

    pub(super) fn write(prepared: PreparedTableClipboard) -> Result<(), String> {
        let _writer = WRITER
            .try_lock()
            .map_err(|_| "已有表格正在复制，请稍后再试")?;
        image_size(prepared.image.width(), prepared.image.height())?;
        text_size(&prepared.html)?;
        text_size(&prepared.csv)?;
        text_size(&prepared.markdown)?;
        let path: Vec<u16> = prepared.png_path.as_os_str().encode_wide().collect();
        let file_drop = dropfiles(&path)?;
        let png = read_png(&prepared.png_path, prepared.image.dimensions())?;
        // Rich formats first; Windows synthesizes CF_BITMAP from CF_DIB, and ANSI
        // text from CF_UNICODETEXT. No device-dependent GDI bitmap handle is needed.
        let payloads = vec![
            (registered("HTML Format")?, html_clipboard(&prepared.html)?),
            (registered("CSV")?, ansi_csv(&prepared.csv)?),
            (registered("text/csv")?, utf8_text(&prepared.csv)?),
            (registered("text/markdown")?, utf8_text(&prepared.markdown)?),
            (CF_UNICODETEXT, unicode_text(&prepared.markdown)?),
            (registered("PNG")?, png),
            (CF_DIB, dib24(&prepared.image)?),
            (CF_HDROP, file_drop),
            // CF_HDROP should COPY the durable staged PNG, not remove it on paste.
            (
                registered("Preferred DropEffect")?,
                1u32.to_le_bytes().to_vec(),
            ),
        ];
        drop(prepared); // Release the RGBA image before allocating Windows copies.
        let total = payloads.iter().try_fold(0usize, |total, (_, bytes)| {
            total.checked_add(bytes.len()).ok_or("剪贴板数据长度溢出")
        })?;
        if total > MAX_PAYLOAD_BYTES {
            return Err("表格剪贴板总数据超过 256 MiB 限制".into());
        }
        // All native allocations also succeed BEFORE Open/EmptyClipboard. Each Vec
        // is released as its native copy is prepared, bounding duplicate buffer peaks.
        let formats = payloads
            .into_iter()
            .map(|(format, bytes)| Ok((format, GlobalMemory::new(&bytes)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let owner = OwnerWindow::new()?;
        let mut clipboard = Clipboard::open(owner.hwnd)?;
        let result = publish(&mut clipboard, formats);
        let close = clipboard.close();
        match (result, close) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(close)) => Err(format!("{error}；{close}")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn csv_ansi_keeps_ascii_and_is_null_terminated() {
            assert_eq!(
                ansi_csv("\"001\",\"-3\"\r\n").unwrap(),
                b"\"001\",\"-3\"\r\n\0"
            );
            // CP_ACP varies by machine. Never assume it can encode every Unicode
            // character: the separately published UTF-8 text/csv preserves those.
            assert_eq!(utf8_text("中文,😀").unwrap(), "中文,😀\0".as_bytes());
        }

        #[test]
        #[ignore = "Writes the real Windows clipboard; explicit authorization required"]
        fn real_windows_multiformat_clipboard() {
            use windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable;
            let directory =
                std::env::temp_dir().join(format!("mewu-table-clipboard-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let png_path = directory.join("synthetic-table.png");
            let image = RgbaImage::from_pixel(8, 8, image::Rgba([240, 245, 255, 255]));
            image.save(&png_path).unwrap();
            write(PreparedTableClipboard {
                html: "<table><tr><td>中文 001</td></tr></table>".into(),
                markdown: "| 中文 |\r\n| --- |\r\n| 001 |".into(),
                csv: "\"中文\"\r\n\"001\"".into(),
                image,
                png_path: png_path.clone(),
            })
            .unwrap();
            for format in [
                CF_DIB,
                CF_UNICODETEXT,
                CF_HDROP,
                registered("HTML Format").unwrap(),
                registered("CSV").unwrap(),
                registered("text/csv").unwrap(),
                registered("text/markdown").unwrap(),
                registered("PNG").unwrap(),
                registered("Preferred DropEffect").unwrap(),
            ] {
                assert_ne!(unsafe { IsClipboardFormatAvailable(format) }, 0);
            }
            // Deliberately retained: deleting it would break the clipboard's CF_HDROP.
            println!("Synthetic clipboard PNG retained at {}", png_path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgba};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn html_offsets_count_utf8_bytes_and_preserve_escaped_fragment() {
        let fragment = "<table><tr><td>中文 😀 &lt;script&gt; &amp;</td></tr></table>";
        let bytes = html_clipboard(fragment).unwrap();
        let text = std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap();
        let offset = |name: &str| -> usize {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .unwrap()
                .parse()
                .unwrap()
        };
        let start = offset("StartHTML:");
        let end = offset("EndHTML:");
        let fragment_start = offset("StartFragment:");
        let fragment_end = offset("EndFragment:");
        assert_eq!(
            &bytes[start..fragment_start],
            b"<html><body><!--StartFragment-->"
        );
        assert_eq!(&bytes[fragment_start..fragment_end], fragment.as_bytes());
        assert_eq!(
            &bytes[fragment_end..end],
            b"<!--EndFragment--></body></html>"
        );
        assert_eq!(bytes[end], 0);
        assert_eq!(end + 1, bytes.len());
    }

    #[test]
    fn unicode_text_and_dropfiles_preserve_utf16_and_double_terminator() {
        let text = "中文 😀\r\n=1+1";
        let expected: Vec<u8> = text
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(unicode_text(text).unwrap(), expected);
        let path: Vec<u16> = "D:\\表格\\😀.png".encode_utf16().collect();
        let bytes = dropfiles(&path).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 20);
        assert_eq!(&bytes[4..16], &[0; 12]);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        let actual: Vec<u16> = bytes[20..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut expected = path;
        expected.extend([0, 0]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn dib_is_bottom_up_bgr_with_white_alpha_and_dword_padding() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, Rgba([0, 255, 0, 255]));
        image.put_pixel(0, 1, Rgba([0, 0, 255, 255]));
        image.put_pixel(1, 1, Rgba([0, 0, 0, 128]));
        let bytes = dib24(&image).unwrap();
        assert_eq!(bytes.len(), 40 + 8 * 2);
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 40);
        assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
        assert_eq!(&bytes[12..16], &[1, 0, 24, 0]);
        assert_eq!(&bytes[40..48], &[255, 0, 0, 127, 127, 127, 0, 0]);
        assert_eq!(&bytes[48..56], &[0, 0, 255, 0, 255, 0, 0, 0]);
        let transparent = dib24(&RgbaImage::from_pixel(1, 1, Rgba([19, 44, 22, 0]))).unwrap();
        assert_eq!(&transparent[40..], &[255, 255, 255, 0]);
        assert_eq!(
            u32::from_le_bytes(transparent[20..24].try_into().unwrap()),
            4
        );
    }

    #[test]
    fn png_dimensions_are_checked_without_interpreting_html_or_text() {
        let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(image.as_raw(), 3, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        validate_png(&png, (3, 2)).unwrap();
        assert!(validate_png(&png, (2, 3)).is_err());
        assert!(validate_png(b"not PNG", (3, 2)).is_err());
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8(), image);
    }

    #[test]
    fn invalid_inputs_are_rejected_before_clipboard_access() {
        assert!(image_size(0, 1).is_err());
        assert!(image_size(MAX_SIDE + 1, 1).is_err());
        assert!(image_size(8192, 8192).is_err());
        assert!(image_size(7680, 4320).is_ok());
        assert!(html_clipboard("bad\0html").is_err());
        assert!(utf8_text("").is_err());
        assert!(unicode_text(&"a".repeat(MAX_TEXT_BYTES + 1)).is_err());
        assert!(dropfiles(&[]).is_err());
        assert!(dropfiles(&[65, 0, 66]).is_err());
        assert!(dropfiles(&vec![65; MAX_PATH_UNITS + 1]).is_err());
        assert!(allocated_bytes(MAX_PAYLOAD_BYTES + 1).is_err());
    }

    #[derive(Default)]
    struct Ledger {
        freed_locally: Vec<u32>,
        owned_by_system: Vec<u32>,
        freed_by_system: Vec<u32>,
        clear_count: usize,
        set_count: usize,
    }

    struct Allocation {
        id: Option<u32>,
        ledger: Rc<RefCell<Ledger>>,
    }

    impl Drop for Allocation {
        fn drop(&mut self) {
            if let Some(id) = self.id.take() {
                self.ledger.borrow_mut().freed_locally.push(id);
            }
        }
    }

    struct Sink {
        ledger: Rc<RefCell<Ledger>>,
        fail_clear: Option<usize>,
        fail_set: Option<usize>,
    }

    impl ClipboardSink for Sink {
        type Allocation = Allocation;

        fn clear(&mut self) -> Result<(), String> {
            let mut ledger = self.ledger.borrow_mut();
            ledger.clear_count += 1;
            if self.fail_clear == Some(ledger.clear_count) {
                return Err("clear failed".into());
            }
            let owned = std::mem::take(&mut ledger.owned_by_system);
            ledger.freed_by_system.extend(owned);
            Ok(())
        }

        fn set(&mut self, _: u32, data: &mut Allocation) -> Result<(), String> {
            let mut ledger = self.ledger.borrow_mut();
            ledger.set_count += 1;
            if self.fail_set == Some(ledger.set_count) {
                return Err("set failed".into());
            }
            ledger.owned_by_system.push(data.id.take().unwrap());
            Ok(())
        }
    }

    fn batch(ledger: &Rc<RefCell<Ledger>>) -> Vec<(u32, Allocation)> {
        (1..=3)
            .map(|id| {
                (
                    id,
                    Allocation {
                        id: Some(id),
                        ledger: ledger.clone(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn success_clears_once_and_transfers_every_allocation() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut sink = Sink {
            ledger: ledger.clone(),
            fail_clear: None,
            fail_set: None,
        };
        publish(&mut sink, batch(&ledger)).unwrap();
        let ledger = ledger.borrow();
        assert_eq!(ledger.clear_count, 1);
        assert_eq!(ledger.owned_by_system, [1, 2, 3]);
        assert!(ledger.freed_locally.is_empty());
        assert!(ledger.freed_by_system.is_empty());
    }

    #[test]
    fn failed_set_clears_transferred_only_and_frees_pending_locally() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut sink = Sink {
            ledger: ledger.clone(),
            fail_clear: None,
            fail_set: Some(2),
        };
        let error = publish(&mut sink, batch(&ledger)).unwrap_err();
        assert!(error.contains("已清除"));
        let ledger = ledger.borrow();
        assert_eq!(ledger.clear_count, 2);
        assert_eq!(ledger.freed_by_system, [1]);
        assert_eq!(ledger.freed_locally, [2, 3]);
        assert!(ledger.owned_by_system.is_empty());
    }

    #[test]
    fn failed_partial_cleanup_reports_remaining_formats_without_double_free() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut sink = Sink {
            ledger: ledger.clone(),
            fail_clear: Some(2),
            fail_set: Some(2),
        };
        assert!(publish(&mut sink, batch(&ledger))
            .unwrap_err()
            .contains("可能保留部分格式"));
        let ledger = ledger.borrow();
        assert_eq!(ledger.owned_by_system, [1]);
        assert_eq!(ledger.freed_locally, [2, 3]);
        assert!(ledger.freed_by_system.is_empty());
    }

    #[test]
    fn failed_initial_clear_publishes_nothing_and_reclaims_all_allocations() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut sink = Sink {
            ledger: ledger.clone(),
            fail_clear: Some(1),
            fail_set: None,
        };
        assert!(publish(&mut sink, batch(&ledger)).is_err());
        let ledger = ledger.borrow();
        assert_eq!(ledger.clear_count, 1);
        assert_eq!(ledger.set_count, 0);
        assert_eq!(ledger.freed_locally, [1, 2, 3]);
        assert!(ledger.owned_by_system.is_empty());
    }

    struct DropSink {
        inner: Sink,
        fail_close: bool,
    }
    impl ClipboardSink for DropSink {
        type Allocation = Allocation;
        fn clear(&mut self) -> Result<(), String> {
            self.inner.clear()
        }
        fn set(&mut self, format: u32, data: &mut Allocation) -> Result<(), String> {
            self.inner.set(format, data)
        }
    }
    impl FileDropSink for DropSink {
        fn close(&mut self) -> Result<(), String> {
            if self.fail_close {
                Err("close failed".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn file_drop_publication_keeps_primary_after_auxiliary_or_close_failure() {
        for (fail_set, fail_close) in [
            (None, false),
            (Some(2), false),
            (None, true),
            (Some(2), true),
        ] {
            let ledger = Rc::new(RefCell::new(Ledger::default()));
            let mut sink = DropSink {
                inner: Sink {
                    ledger: ledger.clone(),
                    fail_clear: None,
                    fail_set,
                },
                fail_close,
            };
            let mut allocations = batch(&ledger).into_iter();
            let result = publish_file_drop_parts(
                &mut sink,
                allocations.next().unwrap(),
                allocations.next().unwrap(),
                || Ok(()),
            );
            drop(allocations);
            let FileDropPublication::Published { warning } = result else {
                panic!("primary was transferred")
            };
            assert_eq!(warning.is_some(), fail_set.is_some() || fail_close);
            let ledger = ledger.borrow();
            assert_eq!(ledger.clear_count, 1);
            assert_eq!(ledger.set_count, 2);
            assert!(ledger.owned_by_system.contains(&1));
            assert!(!ledger.freed_locally.contains(&1));
            assert!(ledger.freed_by_system.is_empty());
            assert_eq!(ledger.owned_by_system.contains(&2), fail_set.is_none());
            assert_eq!(ledger.freed_locally.contains(&2), fail_set.is_some());
        }
    }

    #[test]
    fn file_drop_refused_admission_does_not_empty_clipboard() {
        let ledger = Rc::new(RefCell::new(Ledger::default()));
        let mut sink = DropSink {
            inner: Sink {
                ledger: ledger.clone(),
                fail_clear: None,
                fail_set: None,
            },
            fail_close: false,
        };
        let mut allocations = batch(&ledger).into_iter();
        let result = publish_file_drop_parts(
            &mut sink,
            allocations.next().unwrap(),
            allocations.next().unwrap(),
            || Err("stale".into()),
        );
        drop(allocations);
        assert!(matches!(result, FileDropPublication::Unpublished { error } if error == "stale"));
        let ledger = ledger.borrow();
        assert_eq!(ledger.clear_count, 0);
        assert_eq!(ledger.set_count, 0);
        assert_eq!(ledger.freed_locally.len(), 3);
    }

    #[test]
    fn file_drop_primary_failure_never_claims_publication_or_restores_old_data() {
        for fail_clear in [None, Some(1)] {
            let ledger = Rc::new(RefCell::new(Ledger::default()));
            ledger.borrow_mut().owned_by_system.push(99);
            let mut sink = DropSink {
                inner: Sink {
                    ledger: ledger.clone(),
                    fail_clear,
                    fail_set: Some(1),
                },
                fail_close: true,
            };
            let mut allocations = batch(&ledger).into_iter();
            let result = publish_file_drop_parts(
                &mut sink,
                allocations.next().unwrap(),
                allocations.next().unwrap(),
                || Ok(()),
            );
            drop(allocations);
            assert!(matches!(result, FileDropPublication::Unpublished { .. }));
            let ledger = ledger.borrow();
            assert_eq!(ledger.clear_count, 1);
            assert_eq!(ledger.set_count, usize::from(fail_clear.is_none()));
            assert_eq!(ledger.freed_locally.len(), 3);
            assert_eq!(ledger.freed_by_system.contains(&99), fail_clear.is_none());
            assert_eq!(ledger.owned_by_system.contains(&99), fail_clear.is_some());
        }
    }
}
