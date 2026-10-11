# Build-time AppImage permissions

`scripts/normalize-linux-appimage.mjs` is a native Linux x86_64 build step for a
newly generated AppDir. Run it after Tauri's GTK/GStreamer plugins and before
release staging/signing. It does not run Mewu/AppRun or modify an installed or
previously transferred candidate. Application startup never chmods resources.

The script accepts exactly the workspace's newly built `MewuAI.AppDir`, its
preview AppImage output and this build's cached AppImage output plugin. It checks
bounded entry/depth/byte counts, ownership, ordinary file types, internal symlinks,
required resources and executable bits. It removes group/other writable and
setuid/setgid bits from ordinary files/directories while preserving content,
read/execute permissions and internal symlinks. Symlinks are never chmoded or
traversed as directories. Same-user changes during later external tool reads
are outside this build validation's protection.

The already fetched plugin is hash-checked and copied into private build scratch;
no plugin code/binary is vendored or shipped. A sanitized environment excludes
signing options and user loader paths. The standalone output plugin repacks the
completed AppDir without rerunning GTK/GStreamer input plugins. The system
`/usr/bin/unsquashfs` then reads the new filesystem into private scratch; this
external build tool must be installed in CI and is not bundled in the product.
An independent ELF64 header reader locates the filesystem, rather than executing
the new application to query its offset.

Only after complete file SHA256, tree, permissions, resource and source checks
does the script replace this build's unpublished output. Release staging must
then recompute artifact hashes/signatures. A failure before final replacement
preserves the original output and leaves scratch/partially normalized AppDir for
diagnosis. A receipt written before a failed replacement cannot authorize the
old output: staging must verify its artifact hash and successful command status.
Deferred cleanup after successful replacement is reported explicitly. This is not
a successful build, installer, worker Ready or GUI receipt.

Preparation checks on Windows cover syntax, malformed ELF rejection and a
read-only actual AppImage header; POSIX permission tests are skipped there.
Native permission fixtures and a real cached-plugin repack plus extracted-tree
comparison must pass on Linux before this step is accepted. No native repack
result is claimed by this preparation packet.

References: [pinned Tauri 2.12.1 bundler](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.1/crates/tauri-bundler/src/bundle/linux/appimage/linuxdeploy.rs),
[standalone output plugin](https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/blob/master/src/main.cpp),
[system SquashFS tools](https://github.com/plougher/squashfs-tools).
