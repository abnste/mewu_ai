// SPDX-License-Identifier: MPL-2.0
//! Video save selection sharing the native dialog lifecycle.

pub use crate::save_dialog::SaveDialogError;
use crate::save_dialog::{self, DialogSpec, FormatSpec};
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSaveFormat {
    Mp4,
    Gif,
}

pub type SaveDialogOutcome = save_dialog::SaveDialogOutcome<VideoSaveFormat>;

static FORMATS: [FormatSpec<VideoSaveFormat>; 2] = [
    FormatSpec {
        format: VideoSaveFormat::Mp4,
        label: "MP4 视频",
        extensions: &["mp4"],
    },
    FormatSpec {
        format: VideoSaveFormat::Gif,
        label: "GIF 动图（无声）",
        extensions: &["gif"],
    },
];

pub(crate) fn spec(allow_gif: bool) -> DialogSpec<VideoSaveFormat> {
    DialogSpec {
        basename: "录屏",
        formats: if allow_gif { &FORMATS } else { &FORMATS[..1] },
        default_index: 1,
    }
}

/// Compatible with the current product host. Only this request's offered table
/// may map Shell indices; GIF unavailable means there is no second entry.
pub fn pick_video_destination(
    parent_hwnd: isize,
    allow_gif: bool,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome, SaveDialogError> {
    save_dialog::pick_destination(parent_hwnd, spec(allow_gif), cancel, allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_mp4_gif_indices_and_default_remain_compatible() {
        for allow in [false, true] {
            let value = spec(allow);
            value.validate().unwrap();
            assert_eq!(value.default_index, 1);
            assert_eq!(value.selected(1).unwrap().format, VideoSaveFormat::Mp4);
            assert_eq!(value.formats.len(), if allow { 2 } else { 1 });
        }
        assert_eq!(spec(true).selected(2).unwrap().format, VideoSaveFormat::Gif);
        for index in [0, 2, 3, u32::MAX] {
            assert!(spec(false).selected(index).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn final_path_is_not_renamed_after_confirmation() {
        use std::path::Path;
        for file in [
            r"D:\test\clip.gif",
            r"D:\test\片段.GIF",
            r"D:\test\clip.v2.gif",
            r"D:\test\clip.mp4.gif",
        ] {
            assert_eq!(
                spec(true).destination(Path::new(file), 2).unwrap(),
                VideoSaveFormat::Gif
            );
        }
        for file in [
            r"D:\test\clip",
            r"D:\test\clip.mp4",
            r"D:\test\clip.txt",
            r"clip.gif",
        ] {
            assert!(spec(true).destination(Path::new(file), 2).is_err());
        }
    }
}
