// SPDX-License-Identifier: MPL-2.0
//! Video save selection sharing the native dialog lifecycle.

pub use crate::save_dialog::SaveDialogError;
use crate::save_dialog::{self, DialogSpec, FormatSpec};
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSaveFormat {
    Mp4,
    Mp3,
    Gif,
}

pub type SaveDialogOutcome = save_dialog::SaveDialogOutcome<VideoSaveFormat>;

static FORMATS: [FormatSpec<VideoSaveFormat>; 3] = [
    FormatSpec {
        format: VideoSaveFormat::Mp4,
        label: "MP4 视频",
        extensions: &["mp4"],
    },
    FormatSpec {
        format: VideoSaveFormat::Mp3,
        label: "MP3 音频",
        extensions: &["mp3"],
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
        formats: if allow_gif { &FORMATS } else { &FORMATS[..2] },
        default_index: 1,
    }
}

/// Compatible with the current product host. Only this request's offered table
/// may map Shell indices; MP3 is built in and GIF availability is independent.
pub fn pick_video_destination(
    parent_hwnd: isize,
    allow_gif: bool,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome, SaveDialogError> {
    save_dialog::pick_destination(parent_hwnd, spec(allow_gif), cancel, allowed)
}

#[cfg(target_os = "macos")]
pub fn pick_video_destination_macos(
    window: tauri::WebviewWindow,
    token: usize,
    allow_gif: bool,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome, SaveDialogError> {
    save_dialog::macos::pick_destination(window, token, spec(allow_gif), cancel, allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_formats_have_a_stable_default_and_typed_indices() {
        for allow in [false, true] {
            let value = spec(allow);
            value.validate().unwrap();
            assert_eq!(value.default_index, 1);
            assert_eq!(value.selected(1).unwrap().format, VideoSaveFormat::Mp4);
            assert_eq!(value.selected(2).unwrap().format, VideoSaveFormat::Mp3);
            assert_eq!(value.formats.len(), if allow { 3 } else { 2 });
        }
        assert_eq!(spec(true).selected(3).unwrap().format, VideoSaveFormat::Gif);
        for index in [0, 3, 4, u32::MAX] {
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
                spec(true).destination(Path::new(file), 3).unwrap(),
                VideoSaveFormat::Gif
            );
        }
        for file in [
            r"D:\test\clip",
            r"D:\test\clip.mp4",
            r"D:\test\clip.txt",
            r"clip.gif",
        ] {
            assert!(spec(true).destination(Path::new(file), 3).is_err());
        }
        assert_eq!(
            spec(false)
                .destination(Path::new(r"D:\test\片段.MP3"), 2)
                .unwrap(),
            VideoSaveFormat::Mp3
        );
        assert!(spec(true)
            .destination(Path::new(r"D:\test\clip.mp4"), 2)
            .is_err());
    }
}
