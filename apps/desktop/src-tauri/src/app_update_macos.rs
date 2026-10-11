// SPDX-License-Identifier: MPL-2.0
//! Admission for the signed macOS updater. Paths come only from current_exe,
//! never from IPC. The updater plugin performs signature verification and install.
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};

const INVALID: &str = "update_invalid";
const INSTALLATION: &str = "update_install_location";
const ROOT: &str = "MewuAI.app";
const EXECUTABLE: &str = "Contents/MacOS/MewuAI";
const PLIST: &str = "Contents/Info.plist";
const MAX_EXPANDED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_PLIST: u64 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    uid: u32,
}
fn identity(path: &Path, directory: bool) -> Result<Identity, &'static str> {
    let m = fs::symlink_metadata(path).map_err(|_| INSTALLATION)?;
    if m.file_type().is_symlink() || if directory { !m.is_dir() } else { !m.is_file() } {
        return Err(INSTALLATION);
    }
    Ok(Identity {
        dev: m.dev(),
        ino: m.ino(),
        uid: m.uid(),
    })
}
fn digest(path: &Path, limit: u64) -> Result<[u8; 32], &'static str> {
    let mut reader = fs::File::open(path)
        .map_err(|_| INSTALLATION)?
        .take(limit + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0;
    loop {
        let count = reader.read(&mut buffer).map_err(|_| INSTALLATION)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > limit {
            return Err(INSTALLATION);
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}
fn plist_matches(bytes: &[u8], version: &str) -> Result<(), &'static str> {
    let value = plist::Value::from_reader(Cursor::new(bytes)).map_err(|_| INVALID)?;
    let dict = value.as_dictionary().ok_or(INVALID)?;
    for (key, expected) in [
        ("CFBundleIdentifier", "dev.mewu.remake"),
        ("CFBundleExecutable", "MewuAI"),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", version),
        ("CFBundleVersion", version),
    ] {
        if dict.get(key).and_then(plist::Value::as_string) != Some(expected) {
            return Err(INVALID);
        }
    }
    Ok(())
}
fn arm64_executable(header: &[u8]) -> bool {
    // Our release target is a thin little-endian MH_EXECUTE arm64 Mach-O.
    header.len() >= 32
        && header[..4] == [0xcf, 0xfa, 0xed, 0xfe]
        && u32::from_le_bytes(header[4..8].try_into().unwrap()) == 0x0100_000c
        && u32::from_le_bytes(header[12..16].try_into().unwrap()) == 2
}
fn installed_path(exe: &Path, home: &Path) -> Result<PathBuf, &'static str> {
    if exe.file_name() != Some("MewuAI".as_ref()) {
        return Err(INSTALLATION);
    }
    let macos = exe.parent().ok_or(INSTALLATION)?;
    let contents = macos.parent().ok_or(INSTALLATION)?;
    let bundle = contents.parent().ok_or(INSTALLATION)?;
    if macos.file_name() != Some("MacOS".as_ref())
        || contents.file_name() != Some("Contents".as_ref())
        || bundle.file_name() != Some(ROOT.as_ref())
    {
        return Err(INSTALLATION);
    }
    let parent = bundle.parent().ok_or(INSTALLATION)?;
    if parent != Path::new("/Applications") && parent != home.join("Applications") {
        return Err(INSTALLATION);
    }
    Ok(bundle.to_owned())
}
fn admit() -> Result<PathBuf, &'static str> {
    if !cfg!(target_arch = "aarch64") {
        return Err(INSTALLATION);
    }
    let exe = std::env::current_exe().map_err(|_| INSTALLATION)?;
    let bundle = installed_path(&exe, &dirs::home_dir().ok_or(INSTALLATION)?)?;
    // Reject symlinked parents/components and App Translocation/development output.
    if fs::canonicalize(&exe).map_err(|_| INSTALLATION)? != exe {
        return Err(INSTALLATION);
    }
    identity(&bundle, true)?;
    identity(&bundle.join(EXECUTABLE), false)?;
    let info = bundle.join(PLIST);
    identity(&info, false)?;
    if fs::metadata(&info).map_err(|_| INSTALLATION)?.len() > MAX_PLIST {
        return Err(INSTALLATION);
    }
    let mut bytes = Vec::new();
    fs::File::open(info)
        .map_err(|_| INSTALLATION)?
        .take(MAX_PLIST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INSTALLATION)?;
    if bytes.len() as u64 > MAX_PLIST {
        return Err(INSTALLATION);
    }
    plist_matches(&bytes, env!("CARGO_PKG_VERSION"))?;
    Ok(bundle)
}
pub fn supported() -> bool {
    admit().is_ok()
}

pub struct Installation {
    bundle: PathBuf,
    parent: Identity,
    directory: Identity,
    executable: Identity,
    binary_hash: [u8; 32],
    plist_hash: [u8; 32],
}
impl Installation {
    pub fn capture() -> Result<Self, &'static str> {
        let bundle = admit()?;
        let value = Self {
            parent: identity(bundle.parent().ok_or(INSTALLATION)?, true)?,
            directory: identity(&bundle, true)?,
            executable: identity(&bundle.join(EXECUTABLE), false)?,
            binary_hash: digest(&bundle.join(EXECUTABLE), 512 * 1024 * 1024)?,
            plist_hash: digest(&bundle.join(PLIST), MAX_PLIST)?,
            bundle,
        };
        value.check()?;
        Ok(value)
    }
    pub fn check(&self) -> Result<(), &'static str> {
        if admit()? != self.bundle
            || identity(self.bundle.parent().ok_or(INSTALLATION)?, true)? != self.parent
            || identity(&self.bundle, true)? != self.directory
            || identity(&self.bundle.join(EXECUTABLE), false)? != self.executable
            || digest(&self.bundle.join(EXECUTABLE), 512 * 1024 * 1024)? != self.binary_hash
            || digest(&self.bundle.join(PLIST), MAX_PLIST)? != self.plist_hash
        {
            return Err(INSTALLATION);
        }
        Ok(())
    }
}

/// Validate the already signature-verified bytes before entering exit preparation.
/// This does not extract or execute anything. The official plugin extracts only
/// after the host reaches READY. Limit decoded bytes as well as entry/file counts.
pub fn validate_archive(bytes: &[u8], version: &str) -> Result<(), &'static str> {
    let decoder = flate2::read::GzDecoder::new(bytes).take(MAX_EXPANDED + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut names = HashSet::new();
    let mut info = false;
    let mut executable = false;
    let mut declared_bytes = 0u64;
    for entry in archive.entries().map_err(|_| INVALID)? {
        let mut entry = entry.map_err(|_| INVALID)?;
        if names.len() >= MAX_ENTRIES {
            return Err(INVALID);
        }
        let path = entry.path().map_err(|_| INVALID)?.into_owned();
        let components: Vec<_> = path.components().collect();
        if components.is_empty()
            || components[0] != Component::Normal(ROOT.as_ref())
            || components
                .iter()
                .any(|v| !matches!(v, Component::Normal(_)))
        {
            return Err(INVALID);
        }
        let normalized: PathBuf = components.iter().collect();
        let name = normalized.to_str().ok_or(INVALID)?.to_ascii_lowercase();
        if !names.insert(name) {
            return Err(INVALID);
        }
        let kind = entry.header().entry_type();
        // This app has no bundled frameworks or links. Refuse links and devices
        // rather than permit archive paths to redirect the plugin's extraction.
        if !kind.is_file() && !kind.is_dir() {
            return Err(INVALID);
        }
        if components.len() == 1 && !kind.is_dir() {
            return Err(INVALID);
        }
        let size = entry.size();
        declared_bytes = declared_bytes.checked_add(size).ok_or(INVALID)?;
        if declared_bytes > MAX_EXPANDED
            || entry.header().mode().map_err(|_| INVALID)? & 0o7000 != 0
        {
            return Err(INVALID);
        }
        if path == Path::new(ROOT).join(PLIST) {
            if !kind.is_file() || size > MAX_PLIST {
                return Err(INVALID);
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|_| INVALID)?;
            plist_matches(&bytes, version)?;
            info = true;
        } else if path == Path::new(ROOT).join(EXECUTABLE) {
            let mut header = [0u8; 32];
            if !kind.is_file() || entry.header().mode().map_err(|_| INVALID)? & 0o111 == 0 {
                return Err(INVALID);
            }
            entry.read_exact(&mut header).map_err(|_| INVALID)?;
            if !arm64_executable(&header) {
                return Err(INVALID);
            }
            executable = true;
        }
    }
    // Consume the gzip trailer too; a valid tar prefix with a broken CRC is not
    // accepted. take's remaining limit includes skipped entries and tar headers.
    let mut decoded = archive.into_inner();
    std::io::copy(&mut decoded, &mut std::io::sink()).map_err(|_| INVALID)?;
    if decoded.limit() == 0 || !info || !executable {
        return Err(INVALID);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package(version: &str, root: &str, link: bool, architecture: u32) -> Vec<u8> {
        let mut dict = plist::Dictionary::new();
        for (k, v) in [
            ("CFBundleIdentifier", "dev.mewu.remake"),
            ("CFBundleExecutable", "MewuAI"),
            ("CFBundlePackageType", "APPL"),
            ("CFBundleShortVersionString", version),
            ("CFBundleVersion", version),
        ] {
            dict.insert(k.into(), plist::Value::String(v.into()));
        }
        let mut xml = Vec::new();
        plist::Value::Dictionary(dict)
            .to_writer_xml(&mut xml)
            .unwrap();
        let mut binary = [0u8; 32];
        binary[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        binary[4..8].copy_from_slice(&architecture.to_le_bytes());
        binary[12..16].copy_from_slice(&2u32.to_le_bytes());
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        for (name, data, mode) in [
            (PLIST, xml.as_slice(), 0o644),
            (EXECUTABLE, binary.as_slice(), 0o755),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            archive
                .append_data(&mut header, format!("{root}/{name}"), data)
                .unwrap();
        }
        if link {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            archive
                .append_link(&mut header, format!("{root}/Contents/link"), "/tmp")
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn only_an_installed_exact_bundle_can_admit_updates() {
        let home = Path::new("/Users/mini");
        for path in [
            "/Applications/MewuAI.app/Contents/MacOS/MewuAI",
            "/Users/mini/Applications/MewuAI.app/Contents/MacOS/MewuAI",
        ] {
            assert!(installed_path(Path::new(path), home).is_ok());
        }
        for path in [
            "/Users/mini/Developer/MewuAI.app/Contents/MacOS/MewuAI",
            "/Applications/Other.app/Contents/MacOS/MewuAI",
            "/Applications/MewuAI.app/Contents/MacOS/Other",
            "/Applications/MewuAI.app/fakeContents/MacOS/MewuAI",
            "/Applications/nested/MewuAI.app/Contents/MacOS/MewuAI",
        ] {
            assert!(installed_path(Path::new(path), home).is_err(), "{path}");
        }
    }
    #[test]
    fn archive_requires_its_announced_version_arm64_and_single_regular_bundle() {
        assert!(validate_archive(&package("1.0.1", ROOT, false, 0x0100_000c), "1.0.1").is_ok());
        for bytes in [
            package("1.0.0", ROOT, false, 0x0100_000c),
            package("1.0.1", "Other.app", false, 0x0100_000c),
            package("1.0.1", ROOT, true, 0x0100_000c),
            package("1.0.1", ROOT, false, 0x0100_0007),
        ] {
            assert!(validate_archive(&bytes, "1.0.1").is_err());
        }
        let mut broken = package("1.0.1", ROOT, false, 0x0100_000c);
        let end = broken.len();
        broken[end - 5] ^= 0xff;
        assert!(validate_archive(&broken, "1.0.1").is_err());
    }
}
