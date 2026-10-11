# Linux candidate testing

Linux support is under development. A successful Windows build or an X11 decoder unit test does not establish that the Linux application builds, starts, captures, saves or updates successfully.

The current capture adapter requires an X11 desktop with RandR 1.5 monitor support. It reads the actual monitor pixel rectangles, image byte order, visual color masks and scanline padding from the X server. Font DPI does not change the stored capture coordinates. Wayland, cursor inclusion and window snapping need separate adapters and are not accepted as passed by this implementation.

X11 transport uses the actual local desktop `DISPLAY`, Unix sockets and normal Xauthority lookup. Remote TCP/SSH displays are unsupported. One ten-second socket deadline covers nonblocking connection, setup, writes and replies; it does not restart for each reply. Socket timeout tests must run on Linux, including a silent setup server, write backpressure and actual socket closure. This bounds socket waits, not filesystem access or PNG encoding.

The capture command takes the space and frozen widget's actual GTK-owned XIDs and display name on the UI thread. It does not identify windows through frontend input, PID or title. GTK unrealize/Tauri destruction revokes their lifecycle generations; GTK remapping changes their visibility revisions. After the main-loop hide acknowledgement, the capture worker subscribes to X11 StructureNotify and requires each GetWindowAttributes reply to report UNMAPPED, both before and after acquiring pixels. A map/unmap pair or destruction invalidates the result even if the final state looks hidden. The same bounded X11 connection and event queue survive UI layout and are checked again on a blocking worker before adoption. Cancellation and owner revisions are rechecked before engine adoption. An unadopted PNG is removed when its owning invocation is dropped. These checks do not prove a compositor has removed every visual shadow/fade: actual desktop captures still must verify that separately.

GTK/GDK source packages already present in the pinned Tauri graph are direct Linux dependencies for these native getters. The getters and unrealize lifecycle follow the official [GDK X11 window API](https://docs.gtk.org/gdk3-x11/method.X11Window.get_xid.html), [display name API](https://docs.gtk.org/gdk3/method.Display.get_name.html) and [GTK unrealize signal](https://docs.gtk.org/gtk3/signal.Widget.unrealize.html).

## Prepare an isolated test environment

Use a separate checkout and empty test data directory. Record the source commit, dirty status, package version, distribution, architecture, session type, desktop and installed dependency versions before testing. For a source archive, record its SHA256 and supplied commit instead of claiming a Git HEAD.

Install Node.js 24, Rust stable and the [official Tauri Linux development dependencies](https://v2.tauri.app/start/prerequisites/#linux). Check WebKitGTK 4.1, GTK3 and appindicator development packages with `pkg-config`; runtime libraries alone do not prove a build environment is ready. Verify that a marker created in the build directory is visible through the desktop file manager, then verify an actual local executable can be launched through the supported desktop tools. Opening a terminal alone does not prove this.

Chinese drawing, table, video annotation and translation raster tests require usable Chinese fonts, such as the distribution's `fonts-noto-cjk` package. The bounded loader reads DejaVu Sans and Noto Sans CJK font data and chooses its generic family from the parsed faces. Preserve the actual glyph and raster assertions; missing glyphs are failures. The bundled OCR plugin still rejects Linux explicitly until its native adapter is implemented. Platform rejection tests verify this boundary and do not count as working OCR.

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

The Linux override selects AppImage, the PNG icon, a non-fullscreen initial window and the media framework bundle. Local candidate packaging does not require an updater private key. The release coordinator signs the final distributable bytes separately. Never transfer signing keys or private application data to a test device.

Linux settings information reports `platform: linux` and `updateSupport: unconfigured`. About and data information remain readable, while the update panel reports unavailable support and sends no updater status, check or install commands. Linux receipts claiming Windows signed installer or Mac bundle support are rejected. This boundary is not a completed Linux updater; signed AppImage replacement and recovery still require a native implementation and acceptance tests.

Follow the [official AppImage packaging guidance](https://v2.tauri.app/distribute/appimage/) when choosing a build distribution. A build made on a newer distribution does not establish compatibility with older systems. Test normal AppImage execution and extracted execution separately; missing `/dev/fuse` blocks normal mounting even if extraction works.

## Native acceptance evidence

Keep each command, exit code, log, screenshot and exported file hash in a private evidence directory. Record PASS, FAIL, BLOCKED or NOT RUN for each case; only observed expected behavior is PASS.

1. Start the exact candidate from the real desktop. Record its process identity and startup log, visible capture space, settings and clean exit/restart. Browser development mode is a separate check.
2. Enumerate monitors; capture a known color/text fixture and verify PNG dimensions, RGB colors, region selection, preview position and display coordinates. Repeat on available scaled and multiple-monitor configurations, including monitor selection and topology change. Record unavailable hardware cases explicitly.
3. Exercise annotation, copy, pinning, save/cancel, OCR and language handling, scrolling capture, recording/audio, video/export, model connections, local memory and plugin actions. Preserve expected and actual results per feature. Unsupported platform paths are blockers, not skipped successes.
4. Install/run the final AppImage normally and test restart and data persistence in the isolated test profile. Test a signed update, invalid signature/version rejection and rollback/recovery only after the native update adapter is implemented and the final candidate is supplied.

All three platforms must accept the same final source commit and version before publication. A Linux decoder test or a CI build cannot substitute for Linux desktop and installation evidence.
