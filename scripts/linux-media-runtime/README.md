# Linux media runtime

Mewu-authored build and staging scripts retain MPL-2.0. The runtime keeps each
component's own license. `sdk/sources.json` fixes official source URLs and
SHA-256 digests for GStreamer 1.28.8, FFmpeg 7.1.5, OpenH264 2.6.0, Orc 0.4.44
and PipeWire 1.4.11. FFmpeg's upstream signature is checked against its published
fingerprint; no installer signature is produced by this SDK.

Build the complete SDK in Docker from the repository root:

```sh
docker build -f scripts/linux-media-runtime/sdk/Dockerfile \
  -t mewu-linux-media-sdk scripts/linux-media-runtime
```

The Ubuntu 22.04 base digest is fixed. Source archives are verified before
extraction. The build uses shared LGPL FFmpeg with GPL, nonfree and version3
features disabled, the official OpenH264 shared library, and the selected Gst
plugins. GstGL and all Gst base libraries are included because WebKit imports
additional libraries that a recording-only plugin closure would miss. The
PTP helper has no setuid/setgid permissions.

The image contains `/opt/mewu-gst-next` for native build headers/libraries,
`/opt/mewu-media-runtime` for packaging, and
`/opt/mewu-media-runtime.tar.gz`. The runtime carries exact upstream license
texts, dependency package copyrights, fixed sources and the actual Rust
standard-library copyright for the independently compiled PTP helper.
`sdk/audit.py` checks imported GLIBC requirements (maximum 2.35), actual
FFmpeg license/configuration and the plugin/shared-library closure. The SDK
build prefix is a build-time location; installed software must use relative
AppDir resources.

`stage_appdir.py` creates a new review AppDir from an actual Tauri bundle and a
verified runtime. First apply `../linux-tiff-no-jbig/stage_closure.py` with its
locked artifact. The media stage replaces the entire old Gst plugin/helper
trees, assigns relative ELF RUNPATHs, verifies every surviving Gst family
provider against the runtime, retains the matched GTK/WebKit resources and
removes library-only leftovers from obsolete plugins. System libc and graphics
dispatch remain platform providers. It never modifies or executes the input
application.

After staging, run `../normalize-linux-appimage.mjs` and its normal AppImage
repack verification. Debug-only native candidates may need standard
`strip --strip-debug` before the bounded package audit; preserve BuildID and
verify every allocated ELF section remains unchanged. Release builds normally
already omit these debugging sections. Save the input source commit, raw ELF
hash, bundle hash and final installer hash separately.

A validated AppDir must retain `usr/bin/MewuAI`, GTK caches/types/modules/schema,
Gst plugins/scanner/PTP helper, PipeWire modules/SPA/config, complete notices and
its original AppRun. The trusted worker derives these paths from its loaded,
owned AppDir. `run-with-media` is a diagnostic SDK launcher, not a replacement
for this AppDir or the trusted worker resource environment. Native Linux
screenshot/recording tests and actual WebKit playback must use the final
AppDir/installer; a host's newer Gst installation does not validate a bundle.

Publishing requires the project's separate platform acceptance checks. This
Docker SDK build proves runtime construction, not desktop or recording GUI
acceptance. Keep downloaded source archives, build logs, SDK images and test
media outside tracked source. Use the fixed upstream source URLs and these
exact build recipes when providing corresponding dependency source.