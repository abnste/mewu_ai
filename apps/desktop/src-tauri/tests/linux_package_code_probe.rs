// SPDX-License-Identifier: MPL-2.0
//! Native CI launcher only. The actual AppDir Mewu ELF owns its own sealed
//! factory; this test executable never substitutes itself for that ELF.
#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = native::run() {
        eprintln!("feature-only AppDir Code probe failed: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    println!("SKIP: actual AppDir Code probe requires native Linux");
}

#[cfg(target_os = "linux")]
mod native {
    use image::{GrayImage, ImageFormat, Luma};
    use rxing::{BarcodeFormat, EncodeHints, MultiFormatWriter, Writer};
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        path::Path,
        process::{Child, Command, Stdio},
        thread,
        time::Instant,
    };
    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const FLAG: &str = "--mewu-linux-package-code-probe";
    const SIDE: u32 = 300;
    #[derive(Debug, Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Probe {
        schema_version: u32,
        kind: String,
        known_qr_verified: bool,
        actual_ready_verified: bool,
        actual_child_tree_io_drained: bool,
        permit_released: bool,
        production_admitted: bool,
        gui_verified: bool,
    }
    struct Launcher(Child);
    impl Drop for Launcher {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    // Keep draining to actual EOF while bounding retained bytes. Overflow can
    // never become a receipt. GNU timeout bounds the real launcher invocation.
    fn capture(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
        let mut output = Vec::new();
        let mut overflow = false;
        let mut buffer = [0u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(output.len());
            output.extend_from_slice(&buffer[..read.min(remaining)]);
            overflow |= read > remaining;
        }
        Ok((output, overflow))
    }
    fn hash(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn exclusive(path: &Path, bytes: &[u8]) -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }
    pub fn run() -> Result<()> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo = manifest
            .ancestors()
            .nth(3)
            .ok_or("missing repository parent")?
            .canonicalize()?;
        let git = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()?;
        let source_commit = std::str::from_utf8(&git.stdout)?.trim();
        if !git.status.success()
            || source_commit.len() != 40
            || !source_commit.bytes().all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
            || std::env::var("MEWU_BUILD_COMMIT").as_deref() != Ok(source_commit)
        {
            return Err("explicit build source does not match this repository".into());
        }
        let clean = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["status", "--porcelain", "-z"])
            .output()?;
        if !clean.status.success() || !clean.stdout.is_empty() {
            return Err("actual package probe source must remain clean".into());
        }
        let staged: serde_json::Value = serde_json::from_slice(&fs::read(
            repo.join("out/release-linux-x86_64/.metadata/build.json"),
        )?)?;
        let normalized: serde_json::Value = serde_json::from_slice(&fs::read(
            repo.join("target/release/bundle/appimage/permission-normalization.json"),
        )?)?;
        if staged["sourceCommit"].as_str() != Some(source_commit)
            || staged["platform"].as_str() != Some("linux-x86_64")
            || normalized["sourceCommit"].as_str() != Some(source_commit)
            || normalized["contentAndModesVerified"].as_bool() != Some(true)
            || normalized["candidateExecuted"].as_bool() != Some(false)
        {
            return Err("normal staging and feature permission proofs do not bind this source".into());
        }
        let expected = repo
            .join("target/release/bundle/appimage/MewuAI.AppDir")
            .canonicalize()?;
        let supplied = std::env::var_os("MEWU_LINUX_PACKAGE_CODE_PROBE_APPDIR")
            .ok_or("explicit native CI AppDir is required")?;
        let appdir = Path::new(&supplied).canonicalize()?;
        if appdir != expected {
            return Err("AppDir is not this build's fixed bundle directory".into());
        }
        let executable = appdir.join("usr/bin/MewuAI");
        if fs::symlink_metadata(&executable)?.file_type().is_symlink()
            || executable.canonicalize()? != executable
        {
            return Err("Mewu ELF must be the actual regular AppDir entry".into());
        }
        let mut held = File::open(&executable)?;
        let before = held.metadata()?;
        if !before.is_file() || before.mode() & 0o111 == 0 || before.len() > 512 * 1024 * 1024 {
            return Err("invalid actual Mewu ELF".into());
        }
        let mut elf = Vec::new();
        held.read_to_end(&mut elf)?;
        if !elf.starts_with(b"\x7fELF") {
            return Err("AppDir entry is not an ELF".into());
        }
        let elf_hash = hash(&elf);
        drop(elf);
        let qr = MultiFormatWriter.encode_with_hints(
            "MEWU CODE ONLY",
            &BarcodeFormat::QR_CODE,
            SIDE as i32,
            SIDE as i32,
            &EncodeHints::default(),
        )?;
        let mut image = GrayImage::from_pixel(SIDE, SIDE, Luma([255]));
        for y in 0..SIDE {
            for x in 0..SIDE {
                if qr.get(x, y) {
                    image.put_pixel(x, y, Luma([0]));
                }
            }
        }
        let luma = image.as_raw();
        let mut input = serde_json::to_vec(&serde_json::json!({
            "version":1, "width":SIDE, "height":SIDE, "length":luma.len(), "mode":"codes"
        }))?;
        input.push(b'\n');
        input.extend_from_slice(luma);
        let work = repo
            .join("target/linux-package-probe")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&work)?;
        fs::set_permissions(&work, fs::Permissions::from_mode(0o700))?;
        exclusive(&work.join("input.bin"), &input)?;
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png)?;
        exclusive(&work.join("known-qr.png"), png.get_ref())?;
        let started = Instant::now();
        let mut command = Command::new("/usr/bin/timeout");
        command
            .args(["--signal=KILL", "40s"])
            .arg(&executable)
            .arg(FLAG)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C.UTF-8")
            .env("APPDIR", &appdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut launcher = Launcher(command.spawn()?);
        let mut stdin = launcher.0.stdin.take().ok_or("missing input pipe")?;
        let stdout = launcher.0.stdout.take().ok_or("missing output pipe")?;
        let stderr = launcher.0.stderr.take().ok_or("missing error pipe")?;
        let writer = thread::spawn(move || stdin.write_all(&input));
        let output = thread::spawn(move || capture(stdout, 16 * 1024));
        let errors = thread::spawn(move || capture(stderr, 64 * 1024));
        let status = launcher.0.wait()?;
        let written = writer.join().map_err(|_| "input writer panicked")?;
        let (stdout, stdout_overflow) = output.join().map_err(|_| "output reader panicked")??;
        let (stderr, stderr_overflow) = errors.join().map_err(|_| "error reader panicked")??;
        exclusive(&work.join("stdout.json"), &stdout)?;
        exclusive(&work.join("stderr.log"), &stderr)?;
        if !status.success() || written.is_err() || stdout_overflow || stderr_overflow {
            eprintln!(
                "actual native probe exit={status}; evidence={}",
                work.display()
            );
            return Err("native process/real input IO failed or output exceeded bounds".into());
        }
        let probe: Probe = serde_json::from_slice(&stdout)?;
        if probe.schema_version != 1
            || probe.kind != "feature_only_appdir_code_probe"
            || !probe.known_qr_verified
            || !probe.actual_ready_verified
            || !probe.actual_child_tree_io_drained
            || !probe.permit_released
            || probe.production_admitted
            || probe.gui_verified
        {
            return Err("incomplete or incorrectly admitted native receipt".into());
        }
        let stderr_text = std::str::from_utf8(&stderr)?;
        for marker in [
            "package code probe: actual Code Ready version=1 PASS",
            "package code probe: actual Child wait + PID1 owner empty + actual IO join PASS",
        ] {
            if stderr_text.lines().filter(|line| *line == marker).count() != 1 {
                return Err("actual native Ready/wait/IO marker is absent or duplicated".into());
            }
        }
        let current = fs::metadata(&executable)?;
        let after = held.metadata()?;
        for metadata in [&current, &after] {
            if (
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
            ) != (
                before.dev(),
                before.ino(),
                before.len(),
                before.mtime(),
                before.mtime_nsec(),
            ) {
                return Err("actual Mewu ELF changed during probe".into());
            }
        }
        let receipt = serde_json::json!({
            "schemaVersion":1,"kind":"native_feature_appdir_code_receipt",
            "sourceCommit":source_commit,"normalInstallerAssets":staged["assets"],
            "featureRepackedArtifact":normalized["outputArtifact"],
            "nativeLinux":true,"actualMewuElfSha256":elf_hash,
            "actualMewuElfBytes":before.len(),"pngSha256":hash(png.get_ref()),
            "lumaSha256":hash(luma),"elapsedMillis":started.elapsed().as_millis(),
            "externalChildWaited":true,"externalIoJoined":true,"probe":probe,
            "productionAdmitted":false,"guiVerified":false,"full13Gate":false
        });
        exclusive(
            &work.join("receipt.json"),
            &serde_json::to_vec_pretty(&receipt)?,
        )?;
        println!("{receipt}");
        println!("feature-only native evidence: {}", work.display());
        Ok(())
    }
}
