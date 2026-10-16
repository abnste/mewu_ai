# Desktop development and upgrade compatibility

The desktop uses Rust/Tauri 2, Solid/TypeScript and SQLite with the system webview: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux. The 1.0 stable release targets Windows x64, macOS ARM64 and Linux x64. See the [1.0.0 release notes](../../docs/release-notes-1.0.0.md).

## Build

The default `master` branch contains the remake; `dotnet` retains the legacy .NET/WPF source. Build and release new desktop versions from a frozen full commit on `master`.

From the repository root, run `npm ci`, `npm run build` and `cargo test --workspace --locked`. Run the frontend tests with `node --experimental-vm-modules --test apps/desktop/src/*.test.mjs`.

Build native installers on their matching platform: Windows NSIS and portable ZIP, macOS application/DMG, and Linux AppImage with the audited media runtime and package permission normalization. Linux development for this release uses the complete Docker build. Agent and blackboard are optional official plugins; without a ready model connection, capture controls are shown by default.

The release workflow creates unsigned candidates. A coordinator keeps the updater private key outside source and artifacts, signs the exact update payloads, stages each platform with `scripts/prepare-release.mjs`, combines matching native acceptance records with `scripts/assemble-release.mjs`, then publishes with `scripts/publish-release.mjs`. Signing can use `TAURI_SIGNING_PRIVATE_KEY` and its optional password. Existing clients trust the repository's public key, so retain the same protected signing key.

## Updates from stable releases

Stable 1.0 builds use this endpoint:

https://github.com/abnste/mewu_ai/releases/latest/download/latest.json

In the revised 1.0.0 build, the manifest and macOS updater TAR belong to the same immutable versioned release as the installers. The normal publisher neither reads nor creates the retired update-channel release. The stable endpoint follows GitHub Latest and does not depend on the default branch name.

Earlier 1.0.0 and preview builds have their original endpoint compiled in. Old previews can receive the revised 1.0.0 through that endpoint; the old Mac validator still requires its original channel TAR URL, so the compatibility feed mirrors the exact signed revised TAR bytes. It must not be replaced by the new manifest. An existing 1.0.0 installation does not see a same-version update and must reinstall once. Revised builds use only the new stable endpoint.

Versioned releases attach four software downloads (Windows installer and portable ZIP, macOS DMG and Linux AppImage), plus latest.json and the macOS .app.tar.gz for automatic updates. Windows and Linux update from their exact installers. The manifest contains all three targets and version-bound inline signatures. License texts and source attribution remain inside packages; standalone signatures, private metadata and test files are not attached. GitHub provides asset SHA-256 digests. Notes come from docs/release-notes-<version>.md.

Use monotonically increasing SemVer versions. Signature verification binds the installer bytes and version, rejects unsigned updates, rejects downgrade offers, and restricts installer URLs to this repository. Only the first-party settings window has updater permissions. Generated HTML/SVG cannot invoke the updater.

Update checks run after startup and every six hours, using the user's proxy setting. Installation is started explicitly in About. The updater downloads first, verifies the signature, then uses the ordinary save/drain shutdown protocol. Failure before shutdown keeps the running application; failure to launch an installer after final shutdown restarts the current application. Updates do not rename or delete the user data root.

## Legacy migration

The 0.7.4 updater reads GitHub's latest non-prerelease entry and accepts only numeric three- or four-component version tags. It recognizes `MewuAI-Remake-Setup-{version}-win-x64.exe` for 1.x releases and invokes NSIS with `/S`. Publishing stable `v1.0.0` as GitHub latest lets 0.7.4 discover the regular 1.0.0 installer directly. No new compatibility alias is required.

The historical `v1.0.0.4` compatibility entry supplied the exact preview 4 installer under the old updater's required filename. Its installed application identifies itself as `1.0.0-preview.4`; that numeric alias did not advance `latest.json` or change the product version. The preview and compatibility releases and their packages were removed after stable 1.0.0 took over; their source tags remain. See [legacy compatibility history](../../docs/legacy-updates.md).

Earlier 0.7.x versions should first install 0.7.4 from its versioned release page. Their updater does not recognize the remake installer.

On the first installed transition, the NSIS hook reuses the legacy current-user installation directory when it contains MewuAI.exe. The main executable keeps that filename. Successful installation replaces the obsolete Inno uninstall registration to prevent it deleting the new executable; it never deletes legacy local application data. Subsequent remake updates use the registered remake directory.

Supported Chat Completions, Responses and Messages API profiles are imported once from legacy local settings. Credentials are decoded only by the native host and saved through the normal credential store. Unsupported or interrupted profiles are recorded without repeatedly recreating deleted entries. No model request occurs during import.

Legacy conversation files remain in the legacy data directory and are not converted into the new history UI. Preferences and integrations are not fully migrated; the stable release does not promise full legacy feature or data parity. The installer and legacy updater compatibility are independent of that migration scope.

## Data and rollback

New data defaults to `~/.mewu`; existing remake installations retain the existing location. Settings → Data can relocate the complete data directory after saving and shutting down. Preserve both the data directory and the storage-location record before a manual downgrade. Older executables may not understand newer scene fields; never reset a database version number to force a rollback.

Online installer updates do not automatically make a full data backup. Back up important data before switching between legacy and remake applications. MPL-2.0 and all collected dependency/font notices are included in the downloads.
