// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::{Client, Endpoint, ErrorCode, Request, Response, Server, ServerOptions};
use serde_json::json;
use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(10);
const IMAGE_BYTES: &[u8] = b"\x7fELF\x02\x01\x01\0AI\x02source-appimage-fixture";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.private/remake-review/cli-appimage-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o700)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let target = self.0.canonicalize().expect("fixture path exists");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.private/remake-review/cli-appimage-tests")
            .canonicalize()
            .expect("fixture parent exists");
        assert!(target.starts_with(&root) && target != root);
        fs::remove_dir_all(target).unwrap();
    }
}
fn open(path: &Path) -> File {
    File::open(path).unwrap()
}

#[test]
fn environment_only_retains_one_bounded_absolute_appimage_path() {
    assert_eq!(
        appimage_environment(
            &b"UNRELATED=discarded\0APPIMAGE=/tmp/source.AppImage\0X=discarded\0"[..]
        )
        .unwrap(),
        Path::new("/tmp/source.AppImage")
    );
    for bytes in [
        &b"UNRELATED=x\0"[..],
        &b"APPIMAGE=relative\0"[..],
        &b"APPIMAGE=\0"[..],
        &b"APPIMAGE=/one\0APPIMAGE=/two\0"[..],
        &b"APPIMAGE=/unterminated"[..],
    ] {
        assert!(appimage_environment(bytes).is_err());
    }
    let mut oversized = b"APPIMAGE=/".to_vec();
    oversized.extend(vec![b'x'; MAX_IMAGE_PATH]);
    oversized.push(0);
    assert!(appimage_environment(oversized.as_slice()).is_err());
    assert!(appimage_environment(vec![b'x'; MAX_ENVIRONMENT + 1].as_slice()).is_err());
}

#[test]
fn foreign_uid_missing_credentials_and_nonpositive_pid_are_rejected() {
    let uid = unsafe { libc::geteuid() };
    assert_eq!(peer_identity(Some(uid), Some(7), uid).unwrap(), 7);
    assert!(peer_identity(Some(uid.wrapping_add(1)), Some(7), uid).is_err());
    assert!(peer_identity(None, Some(7), uid).is_err());
    assert!(peer_identity(Some(uid), None, uid).is_err());
    assert!(peer_identity(Some(uid), Some(0), uid).is_err());
    assert!(peer_identity(Some(uid), Some(-1), uid).is_err());
}

#[test]
fn identical_executable_bytes_at_distinct_mount_like_paths_are_accepted() {
    // Ordinary fixtures model distinct FUSE inode/path identities. Actual FUSE
    // mounting remains a packaging integration check, not this helper test.
    let f = Fixture::new();
    let own = f.file("mount-a/usr/bin/MewuAI", b"exact-same-executable-bytes");
    let peer = f.file("mount-b/usr/bin/MewuAI", b"exact-same-executable-bytes");
    assert_ne!(
        Stamp::file(&open(&own)).unwrap(),
        Stamp::file(&open(&peer)).unwrap()
    );
    same_executables(
        &mut open(&own),
        &mut open(&peer),
        Instant::now() + TIMEOUT,
        || {},
    )
    .unwrap();
}

#[test]
fn same_name_size_but_different_version_or_bytes_is_rejected() {
    let f = Fixture::new();
    let own = f.file("a/MewuAI", b"version-one");
    for bytes in [b"version-two".as_slice(), b"version-one-plus".as_slice()] {
        let peer = f.file("b/MewuAI", bytes);
        assert!(same_executables(
            &mut open(&own),
            &mut open(&peer),
            Instant::now() + TIMEOUT,
            || {}
        )
        .is_err());
    }
}

#[test]
fn post_hash_executable_mutation_and_expired_deadline_fail_closed() {
    let f = Fixture::new();
    let own = f.file("a", b"same");
    let peer = f.file("b", b"same");
    assert!(same_executables(
        &mut open(&own),
        &mut open(&peer),
        Instant::now() + TIMEOUT,
        || {
            OpenOptions::new()
                .write(true)
                .open(&peer)
                .unwrap()
                .set_len(1)
                .unwrap();
        }
    )
    .is_err());
    assert!(same_executables(&mut open(&own), &mut open(&own), Instant::now(), || {}).is_err());
}

#[test]
fn source_appimage_identity_rejects_another_identical_file_and_replacement() {
    let f = Fixture::new();
    let first = f.file("one.AppImage", IMAGE_BYTES);
    let second = f.file("two.AppImage", IMAGE_BYTES);
    let image = Image::open(&first).unwrap();
    same_images(&image, &Image::open(&first).unwrap()).unwrap();
    assert!(same_images(&image, &Image::open(&second).unwrap()).is_err());
    let link = f.0.join("alias.AppImage");
    std::os::unix::fs::symlink(&first, &link).unwrap();
    same_images(&image, &Image::open(&link).unwrap()).unwrap();
    fs::rename(&first, f.0.join("retained.AppImage")).unwrap();
    f.file("one.AppImage", IMAGE_BYTES);
    assert!(image.unchanged().is_err());
    let invalid = f.file("not-image", b"not an AppImage, despite its environment");
    assert!(Image::open(&invalid).is_err());
}

#[test]
fn source_appimage_hardlink_alias_is_same_inode_but_late_changes_are_rejected() {
    let f = Fixture::new();
    let first = f.file("one.AppImage", IMAGE_BYTES);
    let alias = f.0.join("same-inode.AppImage");
    fs::hard_link(&first, &alias).unwrap();
    let image = Image::open(&first).unwrap();
    same_images(&image, &Image::open(&alias).unwrap()).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&first)
        .unwrap()
        .set_len(11)
        .unwrap();
    assert!(image.unchanged().is_err());
}

// The following tests run real Linux processes, SO_PEERCRED, /proc/PID/exe
// handles and native local IPC. They do not mount an AppImage or open any user
// endpoint. Child processes get only fixture paths and a unique test namespace.
const ROLE: &str = "MEWU_CONTROL_APPIMAGE_TEST_ROLE";
const DIRECTORY: &str = "MEWU_CONTROL_APPIMAGE_TEST_DIR";
const NAMESPACE: &str = "MEWU_CONTROL_APPIMAGE_TEST_NAMESPACE";
const EXPECT_SUCCESS: &str = "MEWU_CONTROL_APPIMAGE_TEST_SUCCESS";

struct OwnedChild(Child);
impl OwnedChild {
    fn wait(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "AppImage fixture child failed");
                return;
            }
            assert!(Instant::now() < deadline, "AppImage fixture child timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // This guard owns only its explicitly spawned test-harness child.
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn child(
    executable: &Path,
    role: &str,
    fixture: &Fixture,
    namespace: &str,
    appimage: Option<&Path>,
    success: bool,
) -> OwnedChild {
    let mut command = Command::new(executable);
    command
        .args([
            "--exact",
            "linux_appimage::tests::native_process_fixture_entry",
            "--nocapture",
        ])
        .env(ROLE, role)
        .env(DIRECTORY, &fixture.0)
        .env(NAMESPACE, namespace)
        .env(EXPECT_SUCCESS, if success { "1" } else { "0" })
        .env_remove("XDG_RUNTIME_DIR")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(image) = appimage {
        command.env("APPIMAGE", image);
    } else {
        command.env_remove("APPIMAGE");
    }
    OwnedChild(command.spawn().unwrap())
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "AppImage fixture handshake timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn native_roundtrip(
    peer_image_present: bool,
    different_image: bool,
    altered_binary: bool,
    accepted: bool,
) {
    let f = Fixture::new();
    let own_exe = f.0.join("mount-a/MewuAI");
    let peer_exe = f.0.join("mount-b/MewuAI");
    fs::create_dir_all(own_exe.parent().unwrap()).unwrap();
    fs::create_dir_all(peer_exe.parent().unwrap()).unwrap();
    fs::copy("/proc/self/exe", &own_exe).unwrap();
    fs::copy("/proc/self/exe", &peer_exe).unwrap();
    fs::set_permissions(&own_exe, Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&peer_exe, Permissions::from_mode(0o700)).unwrap();
    if altered_binary {
        use std::io::Write;
        OpenOptions::new()
            .append(true)
            .open(&peer_exe)
            .unwrap()
            .write_all(b"different-build-bytes")
            .unwrap();
    }
    let own_image = f.file("original.AppImage", IMAGE_BYTES);
    let other_image = f.file("other.AppImage", IMAGE_BYTES);
    let peer_image = if different_image {
        &other_image
    } else {
        &own_image
    };
    let namespace = format!(
        "appimage-{}-{}",
        std::process::id(),
        f.0.file_name().unwrap().to_str().unwrap()
    );
    let mut server = child(
        &peer_exe,
        "server",
        &f,
        &namespace,
        peer_image_present.then_some(peer_image.as_path()),
        accepted,
    );
    wait_file(&f.0.join("ready"));
    let mut client = child(
        &own_exe,
        "client",
        &f,
        &namespace,
        Some(&own_image),
        accepted,
    );
    client.wait();
    fs::write(f.0.join("stop"), b"stop").unwrap();
    server.wait();
    assert_eq!(
        f.0.join("dispatched").exists(),
        accepted,
        "rejected verification must send zero request bodies"
    );
}

#[test]
fn actual_linux_ipc_accepts_same_image_and_equal_running_bytes_across_paths() {
    native_roundtrip(true, false, false, true);
}
#[test]
fn actual_linux_ipc_rejects_missing_env_other_source_and_forged_env_before_body() {
    native_roundtrip(false, false, false, false);
    native_roundtrip(true, true, false, false);
    native_roundtrip(true, false, true, false);
}

#[test]
fn socket_pidfd_pins_actual_peer_and_rejects_its_exit_before_any_body() {
    use interprocess::{local_socket::ConnectOptions, ConnectWaitMode};
    let f = Fixture::new();
    let namespace = format!(
        "lifetime-{}-{}",
        std::process::id(),
        f.0.file_name().unwrap().to_str().unwrap()
    );
    let mut server = child(
        &std::env::current_exe().unwrap(),
        "server",
        &f,
        &namespace,
        None,
        false,
    );
    wait_file(&f.0.join("ready"));
    let endpoint = Endpoint::for_test_namespace(&namespace).unwrap();
    let stream = ConnectOptions::new()
        .name(endpoint.name().unwrap())
        .nonblocking_stream(true)
        .wait_mode(ConnectWaitMode::Timeout(TIMEOUT))
        .connect_sync()
        .unwrap();
    let Stream::UdSocket(socket) = &stream;
    let lifetime = PeerLifetime::socket(socket.as_fd().as_raw_fd()).unwrap();
    lifetime.alive(Instant::now() + TIMEOUT).unwrap();
    fs::write(f.0.join("stop"), b"stop").unwrap();
    server.wait();
    assert!(lifetime.alive(Instant::now() + TIMEOUT).is_err());
    assert!(!f.0.join("dispatched").exists());
}

#[test]
fn native_process_fixture_entry() {
    let Some(role) = std::env::var_os(ROLE) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(DIRECTORY).expect("fixture directory"));
    let namespace = std::env::var(NAMESPACE).unwrap();
    let endpoint = Endpoint::for_test_namespace(&namespace).unwrap();
    match role.to_str().unwrap() {
        "server" => {
            let marker = root.join("dispatched");
            let mut server = Server::start(
                endpoint,
                Arc::new(move |request| {
                    fs::write(&marker, b"actual-dispatch").unwrap();
                    Response::success(request.id, json!({"ok":true}))
                }),
                ServerOptions {
                    io_timeout: TIMEOUT,
                    ..Default::default()
                },
            )
            .unwrap();
            fs::write(root.join("ready"), b"ready").unwrap();
            wait_file(&root.join("stop"));
            server.shutdown(TIMEOUT).unwrap();
        }
        "client" => {
            let result = Client::connect_appimage_verified(&endpoint, TIMEOUT);
            if std::env::var(EXPECT_SUCCESS).unwrap() == "1" {
                let mut client = result.unwrap();
                let response = client
                    .call(&Request::new("proof", "app.status", json!({})), TIMEOUT)
                    .unwrap();
                assert_eq!(response.result().unwrap()["ok"], true);
            } else {
                let error = result
                    .err()
                    .expect("verification must reject before sending a body");
                assert_eq!(error.code, ErrorCode::PermissionDenied);
            }
        }
        _ => panic!("invalid fixture role"),
    }
}
