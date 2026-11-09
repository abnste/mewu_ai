// SPDX-License-Identifier: MPL-2.0
//! Shared eager publication for existing native clipboard workers. Callers keep
//! platform thread/owner requirements and supply a fresh authority check.
pub(crate) fn text(value: &str, check: impl Fn() -> Result<(), String>) -> Result<(), String> {
    #[cfg(windows)]
    { crate::clipboard_write_windows::text(value, check) }
    #[cfg(not(windows))]
    {
        check()?;
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(value))
            .map_err(|_| "无法复制文字".into())
    }
}

pub(crate) fn image(value: &image::RgbaImage, check: impl Fn() -> Result<(), String>) -> Result<(), String> {
    #[cfg(windows)]
    { crate::clipboard_write_windows::image(value, check) }
    #[cfg(not(windows))]
    {
        check()?;
        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(arboard::ImageData {
            width: value.width() as usize, height: value.height() as usize,
            bytes: std::borrow::Cow::Borrowed(value.as_raw()),
        })).map_err(|_| "无法复制图片".into())
    }
}
