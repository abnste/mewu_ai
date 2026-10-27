// SPDX-License-Identifier: MPL-2.0
//! ISO BMFF edit-list encoder lead-in, for MF sources exposing unedited PTS.
//! Never infer a shift from the first timestamp alone: a delayed track is valid.
//! A single unit-rate edit must name exactly the first decoded media timestamp.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const MAX_MOOV: u64 = 16 * 1024 * 1024;

fn children(data: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut at = 0usize;
    let mut boxes = Vec::new();
    while at < data.len() {
        if boxes.len() == 128 || data.len() - at < 8 {
            return None;
        }
        let size = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as u64;
        let kind = data[at + 4..at + 8].try_into().ok()?;
        let (size, header) = if size == 1 {
            (
                u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?),
                16,
            )
        } else if size == 0 {
            ((data.len() - at) as u64, 8)
        } else {
            (size, 8)
        };
        let size = usize::try_from(size).ok()?;
        let end = at.checked_add(size)?;
        if size < header || end > data.len() {
            return None;
        }
        boxes.push((kind, &data[at + header..end]));
        at = end;
    }
    Some(boxes)
}
fn one<'a>(boxes: &[([u8; 4], &'a [u8])], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut matching = boxes.iter().filter(|(name, _)| name == kind);
    let data = matching.next()?.1;
    if matching.next().is_some() {
        return None;
    }
    Some(data)
}
fn ticks(value: u64, scale: u32) -> Option<u64> {
    if scale == 0 {
        return None;
    }
    u64::try_from(u128::from(value).checked_mul(10_000_000)? / u128::from(scale)).ok()
}
fn lead_in(moov: &[u8], first_pts: u64) -> Option<u64> {
    if first_pts == 0 {
        return None;
    }
    let movie = children(moov)?;
    let mut video = None;
    for (_, track) in movie.iter().filter(|(kind, _)| kind == b"trak") {
        let track = children(track)?;
        let media = children(one(&track, b"mdia")?)?;
        let handler = one(&media, b"hdlr")?;
        if handler.get(8..12)? != b"vide" {
            continue;
        }
        // MF stream order must not be guessed in a multi-video file.
        if video.is_some() {
            return None;
        }
        video = Some((track, media));
    }
    let (track, media) = video?;
    let mdhd = one(&media, b"mdhd")?;
    let scale_at = match *mdhd.first()? {
        0 => 12,
        1 => 20,
        _ => return None,
    };
    let scale = u32::from_be_bytes(mdhd.get(scale_at..scale_at + 4)?.try_into().ok()?);
    let edits = children(one(&track, b"edts")?)?;
    let elst = one(&edits, b"elst")?;
    if elst.get(1..4)? != [0, 0, 0] || elst.get(4..8)? != [0, 0, 0, 1] {
        return None;
    }
    let (duration, start, rate_at) = match (*elst.first()?, elst.len()) {
        (0, 20) => (
            u32::from_be_bytes(elst[8..12].try_into().ok()?) as u64,
            i32::from_be_bytes(elst[12..16].try_into().ok()?) as i64,
            16,
        ),
        (1, 28) => (
            u64::from_be_bytes(elst[8..16].try_into().ok()?),
            i64::from_be_bytes(elst[16..24].try_into().ok()?),
            24,
        ),
        _ => return None,
    };
    if duration == 0 || start <= 0 || elst.get(rate_at..rate_at + 4)? != [0, 1, 0, 0] {
        return None;
    }
    let lead = ticks(start as u64, scale)?;
    // An already corrected decoder starts at zero. A real trim starting after
    // the first media sample does not satisfy this equality and is not shifted.
    (lead == first_pts).then_some(lead)
}

pub(super) fn encoder_lead_in(path: &Path, first_pts: u64) -> u64 {
    fn read(path: &Path, first_pts: u64) -> Option<u64> {
        if first_pts == 0 {
            return None;
        }
        let mut file = File::open(path).ok()?;
        let length = file.metadata().ok()?.len();
        let mut at = 0u64;
        for _ in 0..128 {
            if length.checked_sub(at)? < 8 {
                return None;
            }
            file.seek(SeekFrom::Start(at)).ok()?;
            let mut header = [0; 8];
            file.read_exact(&mut header).ok()?;
            let mut size = u32::from_be_bytes(header[..4].try_into().ok()?) as u64;
            let header_size = if size == 1 {
                let mut large = [0; 8];
                file.read_exact(&mut large).ok()?;
                size = u64::from_be_bytes(large);
                16
            } else {
                8
            };
            if size == 0 {
                size = length - at;
            }
            if size < header_size || size > length - at {
                return None;
            }
            if &header[4..] == b"moov" {
                let count = size - header_size;
                if count > MAX_MOOV {
                    return None;
                }
                let mut bytes = vec![0; usize::try_from(count).ok()?];
                file.read_exact(&mut bytes).ok()?;
                return lead_in(&bytes, first_pts);
            }
            at = at.checked_add(size)?;
        }
        None
    }
    read(path, first_pts).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boxed(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        [
            (data.len() as u32 + 8).to_be_bytes().as_slice(),
            kind.as_slice(),
            data,
        ]
        .concat()
    }
    fn movie(start: i32, count: u32, rate: i32) -> Vec<u8> {
        let mut mdhd = vec![0; 24];
        mdhd[12..16].copy_from_slice(&10240u32.to_be_bytes());
        let mut hdlr = vec![0; 12];
        hdlr[8..12].copy_from_slice(b"vide");
        let elst = [
            0u32.to_be_bytes(),
            count.to_be_bytes(),
            6000u32.to_be_bytes(),
            start.to_be_bytes(),
            rate.to_be_bytes(),
        ]
        .concat();
        boxed(
            b"trak",
            &[
                boxed(
                    b"mdia",
                    &[boxed(b"mdhd", &mdhd), boxed(b"hdlr", &hdlr)].concat(),
                ),
                boxed(b"edts", &boxed(b"elst", &elst)),
            ]
            .concat(),
        )
    }
    #[test]
    fn matches_only_the_proven_encoder_lead_in() {
        let data = movie(2048, 1, 65536);
        assert_eq!(lead_in(&data, 2_000_000), Some(2_000_000));
        assert_eq!(lead_in(&data, 0), None); // decoder already applies the edit
        assert_eq!(lead_in(&data, 1_000_000), None); // real delay or unsupported edit
        assert_eq!(lead_in(&movie(-1, 1, 65536), 2_000_000), None);
        assert_eq!(lead_in(&movie(2048, 2, 65536), 2_000_000), None);
        assert_eq!(lead_in(&movie(2048, 1, 0), 2_000_000), None);
        assert_eq!(lead_in(&[data.clone(), data].concat(), 2_000_000), None);
    }
    #[test]
    fn bounded_and_truncated_boxes_never_shift() {
        let data = movie(2048, 1, 65536);
        for n in 0..data.len() {
            assert_eq!(lead_in(&data[..n], 2_000_000), None);
        }
        assert_eq!(children(&[0xff; 8]), None);
        assert_eq!(ticks(u64::MAX, 1), None);
        assert_eq!(ticks(1, 0), None);
    }
}
