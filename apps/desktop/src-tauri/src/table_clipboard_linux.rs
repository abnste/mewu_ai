// SPDX-License-Identifier: MPL-2.0
//! Prepared MIME payloads. GTK retains their owner after the invocation ends;
//! selection callbacks neither read files nor acquire application locks.
use super::{
    image_size, text_size, validate_png, PreparedTableClipboard, MAX_PAYLOAD_BYTES, MAX_PNG_BYTES,
};
use std::sync::Arc;

pub(crate) struct Data(Vec<(&'static str, Arc<[u8]>)>);
impl Data {
    pub(crate) fn text(text: String) -> Result<Self, String> {
        text_size(&text)?;
        let bytes: Arc<[u8]> = text.into_bytes().into();
        Self::checked(vec![
            ("UTF8_STRING", bytes.clone()),
            ("text/plain;charset=utf-8", bytes.clone()),
            ("text/plain", bytes),
        ])
    }
    pub(crate) fn table(prepared: PreparedTableClipboard) -> Result<Self, String> {
        image_size(prepared.image.width(), prepared.image.height())?;
        text_size(&prepared.html)?;
        text_size(&prepared.csv)?;
        text_size(&prepared.markdown)?;
        let png = read_png(&prepared.png_path, prepared.image.dimensions())?;
        let uri = url::Url::from_file_path(&prepared.png_path).map_err(|_| "表格图片路径无效")?;
        let markdown: Arc<[u8]> = prepared.markdown.into_bytes().into();
        Self::checked(vec![
            ("text/html", prepared.html.into_bytes().into()),
            ("text/csv", prepared.csv.into_bytes().into()),
            ("text/markdown", markdown.clone()),
            ("UTF8_STRING", markdown.clone()),
            ("text/plain;charset=utf-8", markdown.clone()),
            ("text/plain", markdown),
            ("image/png", png.into()),
            ("text/uri-list", format!("{uri}\r\n").into_bytes().into()),
        ])
    }
    fn checked(values: Vec<(&'static str, Arc<[u8]>)>) -> Result<Self, String> {
        let total = values.iter().try_fold(0usize, |sum, (_, value)| {
            sum.checked_add(value.len()).ok_or("剪贴板数据长度溢出")
        })?;
        if total > MAX_PAYLOAD_BYTES {
            return Err("表格剪贴板数据超过复制限制".into());
        }
        Ok(Self(values))
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn publish(
        self,
        window: tauri::WebviewWindow,
        token: usize,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), String> {
        crate::window_actions_linux::with_display(window, token, cancel, move |display| {
            self.publish_on(display)
        })
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn publish_on(self, display: &gtk::gdk::Display) -> Result<(), String> {
        if !gtk::is_initialized_main_thread() {
            return Err("只能在界面线程发布剪贴板".into());
        }
        let targets: Vec<_> = self
            .0
            .iter()
            .enumerate()
            .map(|(index, (mime, _))| {
                gtk::TargetEntry::new(mime, gtk::TargetFlags::empty(), index as u32)
            })
            .collect();
        let clipboard = gtk::Clipboard::for_display(display, &gtk::gdk::SELECTION_CLIPBOARD);
        let success = clipboard.set_with_data(&targets, move |_, selection, index| {
            // gtk-rs invokes this closure from a C signal boundary. A collector
            // failure leaves the request unanswered, without unwinding into C.
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some((_, bytes)) = self.0.get(index as usize) {
                    selection.set(&selection.target(), 8, bytes);
                }
            })) {
                std::mem::forget(payload);
            }
        });
        display.sync();
        if success {
            Ok(())
        } else {
            Err("无法取得表格剪贴板所有权".into())
        }
    }
}

#[cfg(windows)]
fn read_png(path: &std::path::Path, dimensions: (u32, u32)) -> Result<Vec<u8>, String> {
    super::read_png(path, dimensions)
}

#[cfg(target_os = "linux")]
fn read_png(path: &std::path::Path, dimensions: (u32, u32)) -> Result<Vec<u8>, String> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|value| value.as_encoded_bytes().eq_ignore_ascii_case(b"png"))
    {
        return Err("表格图片必须是绝对路径的 PNG 文件".into());
    }
    let before = std::fs::symlink_metadata(path).map_err(|_| "表格 PNG 文件不可用")?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > MAX_PNG_BYTES as u64 {
        return Err("表格 PNG 文件无效或超过复制限制".into());
    }
    // Linux UAPI asm-generic/fcntl.h O_NOFOLLOW (0400000). Never follow a
    // substituted final-component link; also compare the actual opened inode.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x20000)
        .open(path)
        .map_err(|_| "无法读取表格 PNG 文件")?;
    let actual = file.metadata().map_err(|_| "无法检查表格 PNG 文件")?;
    if !actual.is_file()
        || (actual.dev(), actual.ino(), actual.len()) != (before.dev(), before.ino(), before.len())
    {
        return Err("表格 PNG 文件已改变".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(actual.len() as usize)
        .map_err(|_| "没有足够内存读取表格 PNG")?;
    file.take(MAX_PNG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取表格 PNG 失败")?;
    validate_png(&bytes, dimensions)?;
    Ok(bytes)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read_png(_: &std::path::Path, _: (u32, u32)) -> Result<Vec<u8>, String> {
    Err("此平台未启用 Linux 剪贴板".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_payloads_are_raw_bounded_and_share_owned_storage() {
        let data = Data::text("表格甲\t乙\n1\t2".into()).unwrap();
        assert_eq!(data.0[0].1.as_ref(), "表格甲\t乙\n1\t2".as_bytes());
        assert!(Arc::ptr_eq(&data.0[0].1, &data.0[1].1));
        assert!(Data::text("bad\0text".into()).is_err());
    }
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn prepared_table_keeps_png_html_and_escaped_file_uri_without_clipboard_io() {
        let dir = std::env::temp_dir().join(format!("mewu-table-data-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("表格 fixture.png");
        let image =
            image::RgbaImage::from_raw(2, 1, vec![19, 41, 73, 255, 211, 101, 7, 255]).unwrap();
        image.save(&path).unwrap();
        let data = Data::table(PreparedTableClipboard {
            html: "<table><tr><td>甲</td></tr></table>".into(),
            csv: "甲,7\n".into(),
            markdown: "甲 | 7".into(),
            image: image.clone(),
            png_path: path.clone(),
        })
        .unwrap();
        assert_eq!(data.0.len(), 8);
        let png = data
            .0
            .iter()
            .find(|(mime, _)| *mime == "image/png")
            .unwrap();
        assert_eq!(image::load_from_memory(&png.1).unwrap().into_rgba8(), image);
        let uri = data
            .0
            .iter()
            .find(|(mime, _)| *mime == "text/uri-list")
            .unwrap();
        let uri = std::str::from_utf8(&uri.1).unwrap();
        assert!(uri.ends_with("\r\n"));
        assert_eq!(
            url::Url::parse(uri.trim_end())
                .unwrap()
                .to_file_path()
                .unwrap(),
            path
        );
        assert!(!uri.contains(' '));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

#[cfg(all(test, target_os = "linux", feature = "linux-x11-tests"))]
pub(crate) fn native_clipboard_fixture(display: &gtk::gdk::Display) {
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    // A second GDK display is a separate native X11 connection. Keep this in
    // the existing single GTK-thread fixture; do not initialize GTK twice.
    let peer = gtk::gdk::Display::open(&display.name()).expect("independent X11 connection");
    assert_ne!(peer, *display);
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("mewu-table-mime-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let path = dir.join("synthetic table.png");
    let image = image::RgbaImage::from_raw(2, 1, vec![19, 41, 73, 255, 211, 101, 7, 255]).unwrap();
    image.save(&path).unwrap();
    let data = Data::table(PreparedTableClipboard {
        html: "<table><tr><td>synthetic</td></tr></table>".into(),
        csv: "synthetic,7\n".into(),
        markdown: "独立客户端".into(),
        image,
        png_path: path.clone(),
    })
    .unwrap();
    let expected = data.0.clone();
    assert_eq!(expected.len(), 8);
    data.publish_on(display).unwrap();
    let clipboard = gtk::Clipboard::for_display(&peer, &gtk::gdk::SELECTION_CLIPBOARD);
    for (mime, expected) in expected {
        let receipt = Rc::new(RefCell::new(None));
        let response = receipt.clone();
        clipboard.request_contents(&gtk::gdk::Atom::intern(mime), move |_, data| {
            let value = if data.length() >= 0 {
                Some(data.data())
            } else {
                None
            };
            *response.borrow_mut() = Some(value);
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(actual) = receipt.borrow_mut().take() {
                assert_eq!(actual.expect("native selection data"), expected.as_ref());
                break;
            }
            assert!(Instant::now() < deadline, "native MIME request timed out");
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Data::text("synthetic replacement".into())
        .unwrap()
        .publish_on(display)
        .unwrap();
    peer.close();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
    eprintln!("GTK clipboard: eight exact MIME payloads served to independent native X11 client after publisher drop PASS");
}
