// SPDX-License-Identifier: MPL-2.0
//! Bounded Type 2 x86-64 AppImage preflight. No extraction or execution.
//! Runtime offset follows the official type2-runtime ELF section-table contract:
//! https://github.com/AppImage/type2-runtime/blob/75849dc/src/runtime/runtime.c
use std::io::{Cursor, Read, Seek, SeekFrom};

const INVALID: &str = "update_invalid";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_RUNTIME: u64 = 16 * 1024 * 1024;

fn read_at<const N: usize>(
    reader: &mut (impl Read + Seek),
    offset: u64,
    length: u64,
) -> Result<[u8; N], &'static str> {
    if offset.checked_add(N as u64).is_none_or(|end| end > length) {
        return Err(INVALID);
    }
    reader.seek(SeekFrom::Start(offset)).map_err(|_| INVALID)?;
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes).map_err(|_| INVALID)?;
    Ok(bytes)
}
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn end(offset: u64, size: u64, bound: u64) -> Result<u64, &'static str> {
    offset
        .checked_add(size)
        .filter(|end| *end <= bound)
        .ok_or(INVALID)
}

pub fn validate(reader: &mut (impl Read + Seek), length: u64) -> Result<(), &'static str> {
    if !(256..=MAX_PACKAGE).contains(&length) {
        return Err(INVALID);
    }
    let header = read_at::<64>(reader, 0, length)?;
    if &header[..7] != b"\x7fELF\x02\x01\x01"
        || &header[8..11] != b"AI\x02"
        || !matches!(u16_at(&header, 16), 2 | 3)
        || u16_at(&header, 18) != 62
        || u32_at(&header, 20) != 1
        || u16_at(&header, 52) != 64
        || u16_at(&header, 54) != 56
        || u16_at(&header, 58) != 64
    {
        return Err(INVALID);
    }
    let (phoff, shoff) = (u64_at(&header, 32), u64_at(&header, 40));
    let (phnum, shnum) = (u16_at(&header, 56), u16_at(&header, 60));
    if phoff < 64
        || shoff < 64
        || !(1..=128).contains(&phnum)
        || !(1..=4096).contains(&shnum)
        || u16_at(&header, 62) >= shnum
    {
        return Err(INVALID);
    }
    let table_end = end(shoff, u64::from(shnum) * 64, MAX_RUNTIME.min(length))?;
    let last = read_at::<64>(reader, table_end - 64, length)?;
    let runtime_end = table_end.max(end(
        u64_at(&last, 24),
        u64_at(&last, 32),
        MAX_RUNTIME.min(length),
    )?);
    end(phoff, u64::from(phnum) * 56, runtime_end)?;
    // Refuse malformed ELF extents before the native runtime ever sees them.
    for index in 0..shnum {
        let section = read_at::<64>(reader, shoff + u64::from(index) * 64, length)?;
        if u32_at(&section, 4) != 8 {
            end(u64_at(&section, 24), u64_at(&section, 32), runtime_end)?;
        }
    }
    let mut executable_segment = false;
    for index in 0..phnum {
        let segment = read_at::<56>(reader, phoff + u64::from(index) * 56, length)?;
        end(u64_at(&segment, 8), u64_at(&segment, 32), runtime_end)?;
        if u32_at(&segment, 0) == 1 {
            if u64_at(&segment, 40) < u64_at(&segment, 32) {
                return Err(INVALID);
            }
            executable_segment |= u32_at(&segment, 4) & 1 != 0 && u64_at(&segment, 32) > 0;
        }
    }
    if !executable_segment {
        return Err(INVALID);
    }
    let superblock = read_at::<96>(reader, runtime_end, length)?;
    let block_size = u32_at(&superblock, 12);
    let block_log = u16_at(&superblock, 22);
    let bytes_used = u64_at(&superblock, 40);
    if &superblock[..4] != b"hsqs"
        || !(1..=1_000_000).contains(&u32_at(&superblock, 4))
        || !(12..=20).contains(&block_log)
        || block_size != 1_u32 << block_log
        || !(1..=6).contains(&u16_at(&superblock, 20))
        || u16_at(&superblock, 26) == 0
        || u16_at(&superblock, 28) != 4
        || u16_at(&superblock, 30) != 0
        || bytes_used < 96
    {
        return Err(INVALID);
    }
    let package_end = end(runtime_end, bytes_used, length)?;
    if length - package_end > 1024 * 1024 {
        return Err(INVALID);
    }
    for offset in [48, 56, 64, 72, 80, 88] {
        let table = u64_at(&superblock, offset);
        if table != u64::MAX && !(96..bytes_used).contains(&table) {
            return Err(INVALID);
        }
    }
    Ok(())
}

pub fn validate_bytes(bytes: &[u8]) -> Result<(), &'static str> {
    validate(&mut Cursor::new(bytes), bytes.len() as u64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(data: &mut [u8], at: usize, value: u64) {
        data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    pub(crate) fn fixture() -> Vec<u8> {
        let mut data = vec![0; 768];
        data[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        data[8..11].copy_from_slice(b"AI\x02");
        put16(&mut data, 16, 3);
        put16(&mut data, 18, 62);
        put32(&mut data, 20, 1);
        put64(&mut data, 32, 64);
        put64(&mut data, 40, 128);
        for (at, value) in [(52, 64), (54, 56), (56, 1), (58, 64), (60, 2), (62, 1)] {
            put16(&mut data, at, value);
        }
        put32(&mut data, 64, 1);
        put32(&mut data, 68, 5);
        put64(&mut data, 96, 120);
        put64(&mut data, 104, 120);
        put32(&mut data, 196, 3);
        put64(&mut data, 216, 120);
        put64(&mut data, 224, 8);
        data[256..260].copy_from_slice(b"hsqs");
        put32(&mut data, 260, 8);
        put32(&mut data, 268, 131072);
        for (at, value) in [(276, 6), (278, 17), (282, 1), (284, 4), (286, 0)] {
            put16(&mut data, at, value);
        }
        put64(&mut data, 296, 512);
        for offset in [48, 56, 64, 72, 80, 88] {
            put64(
                &mut data,
                256 + offset,
                if offset == 48 || offset == 64 {
                    u64::MAX
                } else {
                    128
                },
            );
        }
        data
    }
    #[test]
    fn admits_bounded_type_two_runtime_with_zstd_filesystem() {
        assert_eq!(validate_bytes(&fixture()), Ok(()));
    }
    #[test]
    fn rejects_wrong_architecture_type_archive_and_extents() {
        for (at, value) in [
            (4, 1),
            (5, 2),
            (10, 1),
            (18, 183),
            (54, 0),
            (60, 0),
            (260, 0),
            (276, 7),
            (284, 3),
        ] {
            let mut data = fixture();
            data[at] = value;
            assert!(validate_bytes(&data).is_err(), "byte {at}");
        }
        for (at, value) in [
            (32, u64::MAX),
            (40, u64::MAX),
            (216, u64::MAX),
            (224, u64::MAX),
            (296, u64::MAX),
            (304, 512),
            (312, 512),
        ] {
            let mut data = fixture();
            put64(&mut data, at, value);
            assert!(validate_bytes(&data).is_err(), "extent {at}");
        }
        assert!(validate_bytes(b"\x1f\x8b signed archives are not raw AppImages").is_err());
    }
    #[test]
    fn rejects_every_truncation_and_extra_payload_without_panicking() {
        let data = fixture();
        for length in 0..data.len() {
            assert!(validate_bytes(&data[..length]).is_err(), "length {length}");
        }
        let mut data = data;
        data.resize(data.len() + 1024 * 1024 + 1, 0);
        assert!(validate_bytes(&data).is_err());
    }
}
