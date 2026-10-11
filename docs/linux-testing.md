# Linux candidate testing

Linux support is under development. A successful Windows build or an X11 decoder unit test does not establish that the Linux application builds, starts, captures, saves or updates successfully.

The current capture adapter requires an X11 desktop with RandR 1.5 monitor support. It reads the actual monitor pixel rectangles, image byte order, visual color masks and scanline padding from the X server. Font DPI does not change the stored capture coordinates. Wayland, cursor inclusion and window snapping need separate adapters and are not accepted as passed by this implementation.

X11 transport uses the actual local desktop `DISPLAY`, Unix sockets and normal Xauthority lookup. Remote TCP/SSH displays are unsupported. One ten-second socket deadline covers nonblocking connection, setup, writes and replies; it does not restart for each reply. Socket timeout tests must run on Linux, including a silent setup server, write backpressure and actual socket closure. This bounds socket waits, not filesystem access or PNG encoding.

The capture command takes the space and frozen widget's actual GTK-owned XIDs and display name on the UI thread. It does not identify windows through frontend input, PID or title. GTK unrealize/Tauri destruction revokes their lifecycle generations; GTK remapping changes their visibility revisions. After the main-loop hide acknowledgement, the capture worker subscribes to X11 StructureNotify and requires each GetWindowAttributes reply to report UNMAPPED, both before and after acquiring pixels. A map/unmap pair or destruction invalidates the result even if the final state looks hidden. The same bounded X11 connection and event queue survive UI layout and are checked again on a blocking worker before adoption. Cancellation and owner revisions are rechecked before engine adoption. An unadopted PNG is removed when its owning invocation is dropped. These checks do not prove a compositor has removed every visual shadow/fade: actual desktop captures still must verify that separately.

GTK/GDK source packages already present in the pinned Tauri graph are direct Linux dependencies for these native getters. The getters and unrealize lifecycle follow the official [GDK X11 window API](https://docs.gtk.org/gdk3-x11/method.X11Window.get_xid.html), [display name API](https://docs.gtk.org/gdk3/method.Display.get_name.html) and [GTK unrealize signal](https://docs.gtk.org/gtk3/signal.Widget.unrealize.html).

## Prepare an isolated test environment

Use a separate checkout and empty test data directory. Record the source commit, dirty status, package version, distribution, architecture, session type, desktop and installed dependency versions before testing. For a source archive, record its SHA256 and supplied commit instead of claiming a Git HEAD.

Install Node.js 24, Rust stable and the [official Tauri Linux development dependencies](https://v2.tauri.app/start/prerequisites/#linux). Check WebKitGTK 4.1, GTK3 and appindicator development packages with `pkg-config`; runtime libraries alone do not prove a build environment is ready. Verify that a marker created in the build directory is visible through the desktop file manager, then verify an actual local executable can be launched through the supported desktop tools. Opening a terminal alone does not prove this.

Chinese drawing, table, video annotation and translation raster tests require usable Chinese fonts, such as the distribution's `fonts-noto-cjk` package. The bounded loader reads DejaVu Sans and Noto Sans CJK font data and chooses its generic family from the parsed faces. Preserve the actual glyph and raster assertions; missing glyphs are failures. The Linux OCR adapter uses protected system Tesseract 4.1/5 and installed language models; its ordinary host native test is described in [Linux native OCR](linux-native-ocr.md). Plugin registration and Windows compilation do not count as Linux recognition. Chinese recognition, isolated Code-worker resource trust and desktop adoption remain separate acceptance cases.

From the repository root:

```sh
npm ci
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
xvfb-run -a cargo test --locked --features linux-x11-tests x11_visibility_native
node --experimental-vm-modules --test apps/desktop/src/*.test.mjs
npm run desktop:build -- --config src-tauri/tauri.linux.conf.json
```

The separate Xvfb run exercises real X11 map state, transient map/unmap events, destruction and mapped children under hidden parents. Enabling `linux-x11-tests` requires an actual test X server; absence fails rather than skips these tests. This protocol evidence still does not replace a GTK/Mewu desktop screenshot.

Region image export uses a GTK-owned action token captured before Engine locks. GTK unmap, iconify, unrealize and host destruction invalidate the token; subsequent display cannot revive an old request even if native addresses are reused. A separate native GTK fixture in the Xvfb gate checks real hide/remap/destruction. Cheap publication checks read scalar lifecycle state only; they do not dispatch UI work while holding Engine locks.

The Linux region save path uses the host window's [GTK3 file chooser](https://docs.gtk.org/gtk3/class.FileChooserDialog.html), without relying on a portal service. The explicit PNG/JPEG filter determines the encoder, and the final extension must agree. Switching a filter updates a previously known suffix before GTK's overwrite confirmation; unknown suffixes remain a conflict. Native dialogs use asynchronous GTK signals without a nested modal loop. Source/cancellation checks remain on the original blocking worker, and its Work/source leases survive until UI destruction and the display synchronization acknowledgement. Image copy uses [GTK's clipboard image ownership](https://docs.gtk.org/gtk3/method.Clipboard.set_image.html), so its main loop serves later paste requests after the command returns. The native fixture reads exact RGBA pixels through a separate X11 client after the publishing wrapper is dropped. Actual PNG/JPEG contents, cancellation, overwrite behavior and desktop clipboard interoperability still require acceptance; the fixture does not prove them. Clipboard persistence after application exit depends on a desktop clipboard manager and must be tested separately.

The Linux override selects AppImage, the PNG icon, a non-fullscreen initial window and the media framework bundle. Local candidate packaging does not require an updater private key. The release coordinator signs the final distributable bytes separately. Never transfer signing keys or private application data to a test device.

Linux settings information reports `platform: linux` and `updateSupport: signed_appimage` only for an admitted, writable installation; other installations report `manual_only`. About and data remain readable. The native updater binds the original AppImage and mounted application directory to Tauri's cached relaunch identity, checks the x86-64 Type 2 ELF/SquashFS format, and uses the official updater's signature verification and raw AppImage replacement. Manual-only installations send no update check or install commands. Linux receipts claiming Windows installer or Mac bundle support are rejected. Source validation does not establish successful native replacement, rollback or relaunch; those require real-device acceptance below.

Linux connection and memory credentials use the current user's existing [Secret Service](https://specifications.freedesktop.org/secret-service/latest-single/) and its unlocked default collection. The standard local-bus plain transfer session is used; the desktop keyring owns storage protection. Immutable UUID attributes contain no key. The adapter pins the unique service owner and checks its user identity, rejects duplicate or changed lookup attributes, creates without replacement and verifies the stored value before returning its new reference. There is no plaintext-file fallback, automatic service activation, collection creation or unlock prompt. A missing service or locked keyring is a failure requiring desktop setup, not a successful credential save.

A single five-second deadline covers authentication and all D-Bus methods; the owned connection has no detached executor and is explicitly closed after success or failure. An additional 100 ms bounds socket close. Protocol fixtures must use an isolated bus and only synthetic keys:

```sh
dbus-run-session -- env MEWU_SECRET_SERVICE_TEST_ISOLATED=1 cargo test -p mewu-desktop --locked --features linux-secret-service-tests native_secret_service -- --nocapture --test-threads=1
```

These fixtures require the isolation marker, refuse any existing Secret Service owner, and test actual D-Bus round trips, scope and immutable-reference rejection, locked/duplicate/tampered receipts, a stalled method and silent authentication socket closure. They do not prove GNOME Keyring/KWallet persistence, desktop unlock behavior or at-rest protection. Verify real desktop create/read/restart, connection and memory separation, failed commit cleanup and unavailable/locked keyring behavior separately, using test-only credentials.

Follow the [official AppImage packaging guidance](https://v2.tauri.app/distribute/appimage/) when choosing a build distribution. A build made on a newer distribution does not establish compatibility with older systems. Test normal AppImage execution and extracted execution separately; missing `/dev/fuse` blocks normal mounting even if extraction works.

## Linux pin export

Pin export retains its immutable image source, pixel reservation, actual pin work and export operation through the picker, encoding and publication. Linux uses the same typed PNG/JPEG choice and destination validation as region export, with an asynchronous GTK dialog parented to the original pin and a token invalidated by unmap or destruction. Clipboard publication waits for the original GTK callback and retains GTK ownership for later paste requests. The registry accepts only the space label and canonical UUID pin labels, with bounded identity tombstones. The native X11 fixture exercises a real pin hide/remap/destruction in addition to space lifecycle and clipboard pixels. Portable registry tests do not establish GTK/native success; actual rotated PNG/JPEG export, overwrite/cancel and later desktop paste require separate native acceptance.

## Linux table export

Linux history-table export retains its actual export work through GTK dialog destruction and clipboard UI acknowledgements, including when the invoking future is dropped. Message identity is checked on its blocking worker, with Engine acquired before the export registry at final admission; GTK callbacks never acquire Engine. The PNG chooser is parented to the original space token, uses an explicit PNG suffix and validates the final canonical destination outside the application data directory. Text copy uses a GTK-owned UTF-8 selection. Full table copy prepares escaped HTML, CSV, Markdown/plain UTF-8, PNG and a percent-encoded file URI, then registers all eight MIME targets together. The immutable staging PNG remains available after publication. The GTK owner serves already prepared bytes without reading files or holding application locks in selection callbacks; ownership remains until the clipboard is replaced or the application exits.

The same isolated single-thread GTK/X11 fixture opens a second native GDK/X11 connection and requests each of the eight MIME payloads after the publishing wrapper is dropped. This validates selection ownership and exact bytes when actually executed; portable payload tests validate PNG pixels and URI encoding only. Actual office-application rich-table paste, clipboard-manager persistence after application exit, save/cancel/overwrite, source changes and application-exit drain remain native acceptance cases. Linux video-file clipboard publication remains unsupported pending its separate host integration.

## Native worker lifetime prerequisite

The Linux production `ManagedProcess` adapter remains unsupported and rejects plain commands before spawning them. Its prerequisite fixture uses only the exact current test executable and installed `/usr/bin/bwrap`, with a new user/PID namespace and the fixture worker as PID 1. The writable root bind preserves the helper's normal filesystem view; this is a lifetime prerequisite, not filesystem isolation or support for arbitrary commands/MCP plugins. No bubblewrap source or binary is bundled.

```sh
MEWU_LINUX_WORKER_TEST_ISOLATED=1 cargo test -p mewu-desktop --locked --features linux-worker-tests native_linux_ -- --nocapture --test-threads=1
```

The explicit marker and feature are required. A missing helper, denied namespace or failed pidfd transfer fails the test. Each fixture process sends its own kernel pidfd with SCM_RIGHTS through an exclusively created private test socket. The original helper's child creates a new session and spawns a grandchild with all stdio closed. The controller checks that all owners and the actual output reader remain alive before normal exit, owner cancellation, an actual 80 ms timeout, wrapper cancellation and application-parent death. Completion requires every owned pidfd to be signaled, the actual original child to be reaped, and the actual reader thread to be joined. A numeric PID, an empty stdout notification or wrapper exit alone never establishes completion.

The root bind is followed by `--proc /proc` and `--dev /dev`. Bubblewrap's ordinary recursive bind applies `nodev` to submounts, so inheriting the host `/dev` through that bind does not provide usable null devices. The fixed PID 1 verifies `/dev/null` can actually be opened, read as EOF and written before spawning helpers that use `Stdio::null`. A device or spawn error remains a failed prerequisite; it never extends the deadline or enables production supervision. See the official [bind-mount implementation](https://github.com/containers/bubblewrap/blob/v0.6.1/bind-mount.c).

This fixture follows the installed helper's [PID namespace options](https://github.com/containers/bubblewrap/blob/main/bwrap.xml) and the kernel's [PID namespace teardown implementation](https://github.com/torvalds/linux/blob/master/kernel/pid_namespace.c). It does not yet verify production operation-permit retention, the production bootstrap, configured media backends, or arbitrary descendant containment outside a dedicated namespace. Those remain separate implementation and acceptance gates.

Startup receipt reads and each completion phase share their respective single five-second deadline; a silent receipt has a separate 60 ms deadline regression. Failure cleanup closes original stdin and retained connections, requests cancellation, and waits within one five-second budget for the actual child and reader. If cleanup cannot complete, it explicitly reports failure and transfers the Child, reader JoinHandle, pidfds and listener to an owning cleanup thread until actual wait/join; it cannot establish drain or successful supervision. A separate native test verifies plain commands are refused before execution. These tests remain prerequisites rather than a production adapter.

## Native acceptance evidence

Keep each command, exit code, log, screenshot and exported file hash in a private evidence directory. Record PASS, FAIL, BLOCKED or NOT RUN for each case; only observed expected behavior is PASS.

1. Start the exact candidate from the real desktop. Record its process identity and startup log, visible capture space, settings and clean exit/restart. Browser development mode is a separate check.
2. Enumerate monitors; capture a known color/text fixture and verify PNG dimensions, RGB colors, region selection, preview position and display coordinates. Repeat on available scaled and multiple-monitor configurations, including monitor selection and topology change. Record unavailable hardware cases explicitly.
3. Exercise annotation, copy, pinning, save/cancel, OCR and language handling, scrolling capture, recording/audio, video/export, model connections, local memory and plugin actions. Preserve expected and actual results per feature. Unsupported platform paths are blockers, not skipped successes.
4. Install/run the final AppImage normally and test restart and data persistence in the isolated test profile. The native updater admits only a writable, user-owned x86-64 Type 2 AppImage whose original APPIMAGE path matches Tauri's cached relaunch path and whose current executable belongs to APPDIR. Raw development binaries, read-only installations and redirected paths remain manual-only. Before marking updates accepted, test the final signed candidate, invalid signature/version rejection, file/inode replacement refusal, normal save/drain, restart and data persistence on the real device. Source tests alone do not establish those results.

All three platforms must accept the same final source commit and version before publication. A Linux decoder test or a CI build cannot substitute for Linux desktop and installation evidence.
