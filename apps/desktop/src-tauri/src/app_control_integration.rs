// SPDX-License-Identifier: MPL-2.0
//! One compiled-in official Unix integration script; call only from spawn_blocking.
//! This is not an arbitrary worker entry or a relaxation of owned_process.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const ERROR: &str = "无法完成 Mewu 命令行初始化";
const OUTPUT_LIMIT: usize = 16 * 1024;
const SCRIPT_TEXT: &str = include_str!("../resources/cli/integrate-unix.sh");
const SCRIPT: &[u8] = SCRIPT_TEXT.as_bytes();

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IntegrationResult {
    pub success: bool,
    pub changed: bool,
    pub path_configured: bool,
    pub shell_setup_required: bool,
    pub new_shell_required: bool,
}

fn parse_output(
    stdout: &[u8],
    stderr_len: usize,
    success: bool,
) -> Result<IntegrationResult, String> {
    if !success || stdout.len() > OUTPUT_LIMIT || stderr_len > OUTPUT_LIMIT {
        return Err(ERROR.into());
    }
    let result: IntegrationResult = serde_json::from_slice(stdout).map_err(|_| ERROR)?;
    if !result.success {
        return Err(ERROR.into());
    }
    // These are separate facts. A created shim may still need manual shell
    // setup or a new shell; neither is inferred from success/changed.
    Ok(result)
}

fn drain(
    reader: &mut impl std::io::Read,
    bytes: &mut Vec<u8>,
    eof: &mut bool,
) -> Result<(), String> {
    let mut buffer = [0u8; 1024];
    // Even a stream interrupted repeatedly must return to the outer deadline.
    for _ in 0..32 {
        if *eof {
            break;
        }
        match reader.read(&mut buffer) {
            Ok(0) => *eof = true,
            Ok(count) => {
                if count > OUTPUT_LIMIT.saturating_sub(bytes.len()) {
                    return Err(ERROR.into());
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => return Err(ERROR.into()),
        }
    }
    Ok(())
}

fn stable_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}
fn stable_appimage(path: &str) -> bool {
    stable_path(path)
        && ![
            "/tmp/",
            "/var/tmp/",
            "/run/",
            "/private/tmp/",
            "/private/var/tmp/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && !path
            .split('/')
            .any(|part| part.starts_with(".mount_") || part == "AppImageLauncherFS")
}
fn macos_bundle(executable: &str, home: &str) -> Result<PathBuf, String> {
    if !stable_path(executable) || !stable_path(home) {
        return Err(ERROR.into());
    }
    let bundle = executable
        .strip_suffix("/Contents/MacOS/MewuAI")
        .ok_or(ERROR)?;
    let (parent, name) = bundle.rsplit_once('/').ok_or(ERROR)?;
    if !name.ends_with(".app")
        || name == ".app"
        || (parent != "/Applications" && parent != format!("{home}/Applications"))
    {
        return Err(ERROR.into());
    }
    Ok(PathBuf::from(bundle))
}
fn arguments(script: &Path, source: &Path, macos: bool) -> Vec<OsString> {
    vec![
        "-c".into(),
        SCRIPT_TEXT.into(),
        // Only the compiled-in text is executed. The verified resource path is
        // argv[0], so renaming or rewriting that file cannot change the program.
        script.as_os_str().to_owned(),
        "--configure-shell".into(),
        if macos {
            "--macos-app".into()
        } else {
            "--appimage".into()
        },
        source.as_os_str().to_owned(),
    ]
}

/// The host owns official-plugin admission, once-only state and spawn_blocking.
/// Success does not claim that an already running external shell changed PATH.
pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<IntegrationResult, String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        unix::initialize(app)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = app;
        Err("此平台不使用 Unix 命令行初始化".into())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::*;
    use std::{
        fs::{self, File, OpenOptions},
        io::{self, Read},
        os::{
            fd::AsRawFd,
            unix::{
                fs::{MetadataExt, OpenOptionsExt},
                process::CommandExt,
            },
        },
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    use tauri::Manager;

    fn resource_script(root: &Path) -> Result<(PathBuf, File), String> {
        let root = root.canonicalize().map_err(|_| ERROR)?;
        let path = root.join("cli/integrate-unix.sh");
        if path.canonicalize().map_err(|_| ERROR)? != path || !path.starts_with(&root) {
            return Err(ERROR.into());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| ERROR)?;
        let before = file.metadata().map_err(|_| ERROR)?;
        if !before.is_file() || before.len() != SCRIPT.len() as u64 || SCRIPT.len() > OUTPUT_LIMIT {
            return Err(ERROR.into());
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ERROR)?;
        let after = file.metadata().map_err(|_| ERROR)?;
        let current = fs::symlink_metadata(&path).map_err(|_| ERROR)?;
        if bytes != SCRIPT
            || !current.is_file()
            || before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.dev() != current.dev()
            || before.ino() != current.ino()
        {
            return Err(ERROR.into());
        }
        // Installation-integrity evidence only; /bin/sh never opens this path.
        Ok((path, file))
    }
    fn nonblocking(file: &impl AsRawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                != 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    // WNOWAIT retains the actual child as the PID/PGID anchor until every group
    // signal is finished. No Child::try_wait may reap it before killpg.
    fn exited(child: &Child) -> io::Result<bool> {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        #[cfg(target_os = "linux")]
        let pid = unsafe { info.si_pid() };
        #[cfg(target_os = "macos")]
        let pid = info.si_pid;
        if pid == 0 {
            Ok(false)
        } else if pid as u32 == child.id() {
            Ok(true)
        } else {
            Err(io::ErrorKind::InvalidData.into())
        }
    }
    fn signal_group(child: &Child, force: bool) -> io::Result<()> {
        exited(child)?; // ECHILD/ownership loss forbids signaling any numeric PGID.
        let group = libc::pid_t::try_from(child.id()).map_err(|_| io::ErrorKind::InvalidData)?;
        if group <= 1 || group == unsafe { libc::getpgrp() } {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        if unsafe { libc::kill(-group, signal) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
    struct OwnedScript(Option<Child>);
    impl OwnedScript {
        fn child(&self) -> &Child {
            self.0.as_ref().expect("owned fixed script")
        }
        fn finish(&mut self) -> Result<bool, String> {
            signal_group(self.child(), true).map_err(|_| ERROR)?;
            if !exited(self.child()).map_err(|_| ERROR)? {
                return Err(ERROR.into());
            }
            let mut child = self.0.take().expect("owned fixed script");
            child
                .wait()
                .map(|status| status.success())
                .map_err(|_| ERROR.into())
        }
    }
    impl Drop for OwnedScript {
        fn drop(&mut self) {
            let Some(child) = self.0.take() else {
                return;
            };
            // Same lifetime principle as owned_process::Drop: cleanup cannot
            // retain a Tokio blocking worker or claim timeout as success.
            // The official TERM handler exits through its own lock/temp cleanup.
            // Give that fixed cleanup 250ms, then enforce SIGKILL and real wait.
            let _ = signal_group(&child, false);
            let cleanup_deadline = Instant::now() + Duration::from_millis(250);
            let held = Arc::new(Mutex::new(Some(child)));
            let worker = held.clone();
            if thread::Builder::new()
                .name("mewu-cli-integration-reaper".into())
                .spawn(move || {
                    let mut child = worker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .expect("owned fixed script");
                    loop {
                        match exited(&child) {
                            Ok(true) => {
                                let _ = signal_group(&child, true);
                                let _ = child.wait();
                                return;
                            }
                            Ok(false) => {
                                if Instant::now() >= cleanup_deadline {
                                    let _ = signal_group(&child, true);
                                }
                                thread::sleep(Duration::from_millis(20));
                            }
                            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return,
                            // An interrupted/failed probe is not proof of exit
                            // or ownership loss; retain the Child and retry.
                            Err(_) => thread::sleep(Duration::from_millis(20)),
                        }
                    }
                })
                .is_err()
            {
                if let Some(child) = held.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    let _ = signal_group(child, true);
                }
                std::mem::forget(held);
            }
        }
    }

    pub(super) fn initialize(app: &tauri::AppHandle) -> Result<IntegrationResult, String> {
        let revision = crate::app_control_host::require_enabled(app)?;
        let resource = app.path().resource_dir().map_err(|_| ERROR)?;
        let (script, _resource_inode) = resource_script(&resource)?;
        #[cfg(target_os = "macos")]
        let source = {
            let executable = std::env::current_exe()
                .and_then(fs::canonicalize)
                .map_err(|_| ERROR)?;
            let home = dirs::home_dir()
                .ok_or(ERROR)?
                .canonicalize()
                .map_err(|_| ERROR)?;
            let bundle = macos_bundle(
                executable.to_str().ok_or(ERROR)?,
                home.to_str().ok_or(ERROR)?,
            )?;
            let console = bundle.join("Contents/MacOS/mewu");
            let metadata = fs::symlink_metadata(&console).map_err(|_| ERROR)?;
            if !metadata.is_file()
                || metadata.mode() & 0o111 == 0
                || console.canonicalize().map_err(|_| ERROR)? != console
            {
                return Err(ERROR.into());
            }
            bundle
        };
        #[cfg(target_os = "linux")]
        let source = {
            let image = crate::app_update::linux::appimage_path(app).map_err(|_| ERROR)?;
            if !stable_appimage(image.to_str().ok_or(ERROR)?) {
                return Err(ERROR.into());
            }
            image
        };
        let mut command = Command::new("/bin/sh");
        command
            .args(arguments(&script, &source, cfg!(target_os = "macos")))
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .env(
                "MEWU_CLI_ORIGINAL_PATH",
                std::env::var_os("PATH").unwrap_or_default(),
            )
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LC_ALL", "C");
        // Preserve only the nonsecret per-user shell locations the official
        // helper needs. Never inherit startup hooks or dynamic-loader overrides.
        for name in [
            "HOME",
            "SHELL",
            "XDG_CONFIG_HOME",
            "ZDOTDIR",
            "USER",
            "LOGNAME",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if crate::app_control_host::require_enabled(app)? != revision {
            return Err(ERROR.into());
        }
        let mut process = OwnedScript(Some(command.spawn().map_err(|_| ERROR)?));
        let mut stdout = process.0.as_mut().unwrap().stdout.take().ok_or(ERROR)?;
        let mut stderr = process.0.as_mut().unwrap().stderr.take().ok_or(ERROR)?;
        nonblocking(&stdout)
            .and_then(|_| nonblocking(&stderr))
            .map_err(|_| ERROR)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut out_eof, mut err_eof) = (false, false);
        loop {
            if Instant::now() >= deadline {
                return Err(ERROR.into());
            }
            drain(&mut stdout, &mut out, &mut out_eof)?;
            drain(&mut stderr, &mut err, &mut err_eof)?;
            if Instant::now() >= deadline
                || crate::app_control_host::require_enabled(app)? != revision
            {
                return Err(ERROR.into());
            }
            if exited(process.child()).map_err(|_| ERROR)? && out_eof && err_eof {
                let success = process.finish()?;
                // Reaping does not revoke already published shell changes, but
                // a disabled plugin must not report a late successful setup.
                if Instant::now() >= deadline
                    || crate::app_control_host::require_enabled(app)? != revision
                {
                    return Err(ERROR.into());
                }
                return parse_output(&out, err.len(), success);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_capture_reader_bounds_bytes_and_does_not_treat_would_block_as_eof() {
        let (mut bytes, mut eof) = (Vec::new(), false);
        drain(
            &mut vec![b'x'; OUTPUT_LIMIT].as_slice(),
            &mut bytes,
            &mut eof,
        )
        .unwrap();
        assert!(eof);
        assert_eq!(bytes.len(), OUTPUT_LIMIT);
        let (mut bytes, mut eof) = (Vec::new(), false);
        assert!(drain(
            &mut vec![b'x'; OUTPUT_LIMIT + 1].as_slice(),
            &mut bytes,
            &mut eof
        )
        .is_err());
        assert!(bytes.len() <= OUTPUT_LIMIT && !eof);
        struct NotReady;
        impl std::io::Read for NotReady {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        let (mut bytes, mut eof) = (Vec::new(), false);
        drain(&mut NotReady, &mut bytes, &mut eof).unwrap();
        assert!(!eof && bytes.is_empty());
        struct Interrupted(usize);
        impl std::io::Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                self.0 += 1;
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
        let mut interrupted = Interrupted(0);
        drain(&mut interrupted, &mut bytes, &mut eof).unwrap();
        assert_eq!(interrupted.0, 32);
        assert!(!eof && bytes.is_empty());
    }
    #[test]
    fn result_keeps_created_shim_separate_from_shell_setup_and_new_shell() {
        let value = parse_output(br#"{"success":true,"changed":true,"pathConfigured":false,"shellSetupRequired":true,"newShellRequired":true}"#, 0, true).unwrap();
        assert!(
            value.success
                && value.changed
                && !value.path_configured
                && value.shell_setup_required
                && value.new_shell_required
        );
        let configured = parse_output(br#"{"success":true,"changed":false,"pathConfigured":false,"shellSetupRequired":false,"newShellRequired":true}"#, 0, true).unwrap();
        assert!(!configured.shell_setup_required && configured.new_shell_required);
    }
    #[test]
    fn output_rejects_wrong_types_unknown_duplicate_missing_extra_json_and_bad_exit() {
        let good = br#"{"success":true,"changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false}"#;
        assert!(parse_output(good, 0, false).is_err());
        for value in [br#"{"success":true}"#.as_slice(), br#"{"success":"true","changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false}"#,
            br#"{"success":false,"changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false}"#,
            br#"{"success":true,"success":true,"changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false}"#,
            br#"{"success":true,"changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false,"extra":1}"#,
            br#"{"success":true,"changed":false,"pathConfigured":true,"shellSetupRequired":false,"newShellRequired":false} {}"#] {
            assert!(parse_output(value, 0, true).is_err());
        }
        assert!(parse_output(good, OUTPUT_LIMIT + 1, true).is_err());
        let mut large = vec![b' '; OUTPUT_LIMIT + 1];
        large.extend_from_slice(good);
        assert!(parse_output(&large, 0, true).is_err());
        assert!(parse_output(&[0xff], 0, true).is_err());
    }
    #[test]
    fn stable_sources_reject_fuse_temporary_relative_and_translocated_paths() {
        assert!(stable_appimage("/home/u/Applications/Mewu AI.AppImage"));
        for bad in [
            "relative.AppImage",
            "/tmp/Mewu.AppImage",
            "/run/user/1/Mewu.AppImage",
            "/opt/.mount_mewu123/Mewu.AppImage",
            "/home/u/AppImageLauncherFS/Mewu.AppImage",
            "/home/u/../Mewu.AppImage",
            "/home/u/a\n.AppImage",
        ] {
            assert!(!stable_appimage(bad));
        }
        assert_eq!(
            macos_bundle("/Applications/MewuAI.app/Contents/MacOS/MewuAI", "/Users/u")
                .unwrap()
                .to_string_lossy(),
            "/Applications/MewuAI.app"
        );
        assert!(macos_bundle(
            "/Users/u/Applications/Mewu AI.app/Contents/MacOS/MewuAI",
            "/Users/u"
        )
        .is_ok());
        for bad in [
            "/Volumes/Mewu/MewuAI.app/Contents/MacOS/MewuAI",
            "/private/var/folders/x/AppTranslocation/y/MewuAI.app/Contents/MacOS/MewuAI",
            "/Applications/MewuAI.app/Contents/MacOS/other",
        ] {
            assert!(macos_bundle(bad, "/Users/u").is_err());
        }
    }
    #[test]
    fn fixed_shell_body_is_compiled_in_and_paths_are_only_literal_arguments() {
        let script = Path::new("/resources/' $x;$(false)/cli/integrate-unix.sh");
        assert_eq!(SCRIPT_TEXT.as_bytes(), SCRIPT);
        for (source, macos, kind) in [
            (
                "/Applications/Mewu ' \" $x;$(false)`false`*?.app",
                true,
                "--macos-app",
            ),
            ("/home/u/Mewu\n'\";$x.AppImage", false, "--appimage"),
        ] {
            let source = Path::new(source);
            let args = arguments(script, source, macos);
            assert_eq!(args.len(), 6);
            assert_eq!(args[0], "-c");
            assert_eq!(args[1], SCRIPT_TEXT);
            assert_eq!(args[2], script.as_os_str());
            assert_eq!(args[3], "--configure-shell");
            assert_eq!(args[4], kind);
            assert_eq!(args[5], source.as_os_str());
            assert!(!SCRIPT_TEXT.contains(source.to_str().unwrap()));
            assert!(!SCRIPT_TEXT.contains(script.to_str().unwrap()));
        }
    }
}
