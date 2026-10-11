// SPDX-License-Identifier: MPL-2.0
//! Bounded, independent MPEG Layer III framing/duration check before publication.
use crate::video_clip_contract::{ClipError, VideoMetadata, VideoRange, MIN_RANGE_TICKS};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::AtomicBool,
};

pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub duration_ticks: u64,
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: u32,
}
impl Metadata {
    pub fn validate_for(
        &self,
        source: &VideoMetadata,
        range: Option<&VideoRange>,
    ) -> Result<(), ClipError> {
        source.validate_range(range)?;
        if !source.has_audio {
            return Err(ClipError::NoAudio);
        }
        let duration = range.map_or(source.duration_ticks, |v| v.end_ticks - v.start_ticks);
        if self.frames == 0
            || ![8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000]
                .contains(&self.sample_rate)
            || ![1, 2].contains(&self.channels)
            || self.duration_ticks.abs_diff(duration) > MIN_RANGE_TICKS
        {
            return Err(ClipError::OutputInvalid);
        }
        Ok(())
    }
}
pub fn inspect(path: &Path, cancel: &AtomicBool) -> Result<Metadata, ClipError> {
    let mut file = File::open(path).map_err(|_| ClipError::FileUnavailable)?;
    let size = file
        .metadata()
        .map_err(|_| ClipError::FileUnavailable)?
        .len();
    if !(24..=MAX_BYTES).contains(&size) {
        return Err(ClipError::OutputInvalid);
    }
    scan(&mut file, size, cancel)
}
fn scan(
    file: &mut (impl Read + Seek),
    size: u64,
    cancel: &AtomicBool,
) -> Result<Metadata, ClipError> {
    let invalid = || ClipError::OutputInvalid;
    let mut start = [0; 10];
    file.read_exact(&mut start).map_err(|_| invalid())?;
    let mut position = 0;
    if &start[..3] == b"ID3" {
        if ![3, 4].contains(&start[3]) || start[4] == 255 || start[6..10].iter().any(|v| *v >= 128)
        {
            return Err(invalid());
        }
        let length = start[6..10]
            .iter()
            .fold(0u64, |n, v| (n << 7) | u64::from(*v));
        position = 10
            + length
            + if start[3] == 4 && start[5] & 0x10 != 0 {
                10
            } else {
                0
            };
        if position > 16 * 1024 * 1024 || position >= size {
            return Err(invalid());
        }
    }
    let mut end = size;
    if size >= 128 {
        file.seek(SeekFrom::Start(size - 128))
            .map_err(|_| invalid())?;
        let mut tag = [0; 3];
        file.read_exact(&mut tag).map_err(|_| invalid())?;
        if &tag == b"TAG" {
            end -= 128;
        }
    }
    let mut result = Metadata {
        duration_ticks: 0,
        sample_rate: 0,
        channels: 0,
        frames: 0,
    };
    let mut samples = 0u64;
    while position < end {
        crate::video_clip_backend::check_cancel(cancel)?;
        file.seek(SeekFrom::Start(position))
            .map_err(|_| invalid())?;
        let mut b = [0; 4];
        file.read_exact(&mut b).map_err(|_| invalid())?;
        let version = (b[1] >> 3) & 3;
        if b[0] != 255 || b[1] & 0xe0 != 0xe0 || version == 1 || (b[1] >> 1) & 3 != 1 {
            return Err(invalid());
        }
        let bitrate_index = (b[2] >> 4) as usize;
        let rate_index = ((b[2] >> 2) & 3) as usize;
        if bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
            return Err(invalid());
        }
        let rate = [44100, 48000, 32000][rate_index]
            / match version {
                3 => 1,
                2 => 2,
                _ => 4,
            };
        let bitrate: u32 = if version == 3 {
            [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
            ][bitrate_index]
        } else {
            [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ][bitrate_index]
        };
        let channels = if b[3] >> 6 == 3 { 1 } else { 2 };
        let frame_size = (if version == 3 { 144000_u64 } else { 72000 }) * u64::from(bitrate)
            / u64::from(rate)
            + u64::from((b[2] >> 1) & 1);
        if frame_size < 4 || position + frame_size > end {
            return Err(invalid());
        }
        if result.frames != 0 && (result.sample_rate != rate || result.channels != channels) {
            return Err(invalid());
        }
        result.sample_rate = rate;
        result.channels = channels;
        result.frames += 1;
        samples += if version == 3 { 1152 } else { 576 };
        position += frame_size;
    }
    if result.frames == 0 || position != end {
        return Err(invalid());
    }
    result.duration_ticks = samples * 10_000_000 / u64::from(result.sample_rate);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn encoded(frames: usize) -> Vec<u8> {
        let mut bytes = vec![];
        for _ in 0..frames {
            let mut frame = vec![0; 626];
            frame[..4].copy_from_slice(&[255, 251, 176, 0]);
            bytes.extend(frame);
        }
        bytes
    }
    #[test]
    fn duration_comes_from_all_frames_and_tags_are_excluded() {
        let audio = encoded(40);
        let read = |v: Vec<u8>| {
            scan(
                &mut Cursor::new(&v),
                v.len() as u64,
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let result = read(audio.clone());
        assert_eq!(result.frames, 40);
        assert_eq!(result.duration_ticks, 40 * 1152 * 10_000_000 / 44100);
        assert_eq!(result.sample_rate, 44100);
        assert_eq!(result.channels, 2);
        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x03abc".to_vec();
        tagged.extend(audio);
        tagged.extend(b"TAG");
        tagged.extend([0; 125]);
        assert_eq!(read(tagged), result);
    }
    #[test]
    fn malformed_truncated_changed_rate_and_cancelled_files_are_rejected() {
        for mut bytes in [encoded(4), encoded(4), encoded(4), encoded(4)]
            .into_iter()
            .enumerate()
        {
            match bytes.0 {
                0 => {
                    bytes.1.pop();
                }
                1 => bytes.1[1] = 255,
                2 => bytes.1[626 + 2] |= 4,
                _ => {
                    bytes.1.extend([0; 4]);
                }
            }
            assert!(scan(
                &mut Cursor::new(&bytes.1),
                bytes.1.len() as u64,
                &AtomicBool::new(false)
            )
            .is_err());
        }
        let bytes = encoded(4);
        assert_eq!(
            scan(
                &mut Cursor::new(&bytes),
                bytes.len() as u64,
                &AtomicBool::new(true)
            ),
            Err(ClipError::Cancelled)
        );
    }
}
