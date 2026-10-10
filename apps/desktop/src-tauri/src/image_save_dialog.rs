// SPDX-License-Identifier: MPL-2.0
//! Typed image save selection using the encoder's format enum.

pub use crate::image_export::ImageSaveFormat;
pub use crate::save_dialog::SaveDialogError;
use crate::save_dialog::{self, DialogSpec, FormatSpec};
use std::sync::{atomic::AtomicBool, Arc};

pub type SaveDialogOutcome = save_dialog::SaveDialogOutcome<ImageSaveFormat>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSaveName {
    Screenshot,
    PinnedImage,
}

impl ImageSaveName {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 2] = [Self::Screenshot, Self::PinnedImage];

    fn basename(self) -> &'static str {
        match self {
            Self::Screenshot => "截图",
            Self::PinnedImage => "Mewu-贴图",
        }
    }
}

static FORMATS: [FormatSpec<ImageSaveFormat>; 2] = [
    FormatSpec {
        format: ImageSaveFormat::Png,
        label: "PNG 图片",
        extensions: &["png"],
    },
    FormatSpec {
        format: ImageSaveFormat::Jpeg,
        label: "JPEG 图片",
        extensions: &["jpg", "jpeg"],
    },
];

pub(crate) fn spec(name: ImageSaveName) -> DialogSpec<ImageSaveFormat> {
    spec_with_default(name, ImageSaveFormat::Png)
}
pub(crate) fn spec_with_default(
    name: ImageSaveName,
    format: ImageSaveFormat,
) -> DialogSpec<ImageSaveFormat> {
    DialogSpec {
        basename: name.basename(),
        formats: &FORMATS,
        default_index: match format {
            ImageSaveFormat::Png => 1,
            ImageSaveFormat::Jpeg => 2,
        },
    }
}

/// The host passes the current trusted space/pin HWND and retains its operation
/// lease through this call. After selection it must call ImageDestination::prepare
/// then validate protected roots/source on that final canonical path. No I/O here.
pub fn pick_image_destination(
    parent_hwnd: isize,
    name: ImageSaveName,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome, SaveDialogError> {
    pick_image_destination_with_default(parent_hwnd, name, ImageSaveFormat::Png, cancel, allowed)
}
pub fn pick_image_destination_with_default(
    parent_hwnd: isize,
    name: ImageSaveName,
    format: ImageSaveFormat,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome, SaveDialogError> {
    save_dialog::pick_destination(
        parent_hwnd,
        spec_with_default(name, format),
        cancel,
        allowed,
    )
}
pub fn preference_format(app: &tauri::AppHandle) -> Result<ImageSaveFormat, String> {
    use tauri::Manager;
    let preferences = app
        .state::<crate::Host>()
        .capture_preferences
        .view()
        .map_err(|e| e.to_string())?;
    Ok(match preferences.default_image_format {
        crate::capture_preferences::DefaultImageFormat::Png => ImageSaveFormat::Png,
        crate::capture_preferences::DefaultImageFormat::Jpeg => ImageSaveFormat::Jpeg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_default_changes_only_initial_filter_and_explicit_choice_wins() {
        let directory = std::env::temp_dir();
        for name in ImageSaveName::ALL {
            let value = spec_with_default(name, ImageSaveFormat::Jpeg);
            value.validate().unwrap();
            assert_eq!(value.default_index, 2);
            assert_eq!(
                value.destination(&directory.join("回答.png"), 1).unwrap(),
                ImageSaveFormat::Png
            );
            assert_eq!(
                value.destination(&directory.join("回答.jpg"), 2).unwrap(),
                ImageSaveFormat::Jpeg
            );
        }
    }

    #[test]
    fn both_entries_use_the_same_typed_png_jpeg_table() {
        for name in ImageSaveName::ALL {
            let value = spec(name);
            value.validate().unwrap();
            assert_eq!(value.default_index, 1);
            assert_eq!(value.selected(1).unwrap().format, ImageSaveFormat::Png);
            assert_eq!(value.selected(2).unwrap().format, ImageSaveFormat::Jpeg);
            assert_eq!(value.selected(2).unwrap().extensions, &["jpg", "jpeg"]);
            assert!(value.selected(3).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn jpeg_aliases_are_exact_case_insensitive_final_suffixes() {
        use std::path::Path;
        let value = spec(ImageSaveName::Screenshot);
        for path in [
            r"D:\images\截图.jpg",
            r"D:\images\截图.jpeg",
            r"D:\images\SHOT.JPEG",
            r"D:\images\image.png.jpg",
        ] {
            assert_eq!(
                value.destination(Path::new(path), 2).unwrap(),
                ImageSaveFormat::Jpeg
            );
            assert!(value.destination(Path::new(path), 1).is_err());
        }
        assert_eq!(
            value
                .destination(Path::new(r"D:\images\截图.PNG"), 1)
                .unwrap(),
            ImageSaveFormat::Png
        );
        for path in [
            r"D:\images\image.png",
            r"D:\images\image.jpe",
            r"D:\images\image.jpg.exe",
            r"D:\images\image",
            r"image.jpg",
        ] {
            assert!(value.destination(Path::new(path), 2).is_err());
        }
    }
}
