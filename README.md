# Mewu AI

[简体中文](README.zh-CN.md)

Mewu 1.0 is a desktop remake for Windows x64, macOS ARM64 and Linux x64. A shortcut opens the capture space: screenshots, conversation, files, annotations and interactive HTML/SVG results share the same surface.

Published packages are listed on the [releases page](https://github.com/abnste/mewu_ai/releases): Windows installer and portable ZIP, macOS ARM64 DMG, and Linux x64 AppImage. This branch prepares preview 4; the version becomes available after its release is published. Stable 0.7.x remains on [master](https://github.com/abnste/mewu_ai/tree/master).

## Included

Agent and blackboard are optional plugins. A fresh installation presents capture controls until a model connection or an independent Agent provider is explicitly configured; providers that require no host API key are also supported.

- Region capture, drawing, borderless pinned images, OCR, translation, scrolling capture and barcode recognition; blackboards are available through the optional official plugin.
- With the OCR plugin enabled, stable selections locally recognize links, email addresses and phone numbers. Click a result to copy it, open a webpage, compose a message or hand a number to the dialer. Recognition makes no network requests and does not send messages or place calls automatically.
- Multiple conversations, freeze/resume, Markdown and formulas, collapsible public reasoning, image and text attachments.
- Screen recording with computer audio by default, pause, trimming, video annotations, MP4/MP3/GIF export, and original/75%/50% preview resolution.
- Optional Agent plugin: identity, local SQLite/FTS5 memory, Hindsight connection, run history and resumable answers.
- Chat Completions, OpenAI Responses and Anthropic Messages connections; model templates and declared capabilities can be installed from public GitHub repositories.
- Local MCP tools, compatible API 1 declaration packages, and API 2 JavaScript plugins with GitHub Release packages and a community catalog.
- Installed plugin tools join the current Agent's normal conversation. Reference a capture region and send the task in the conversation bar; the independent region-operation example needs no separate task dialog.

Community plugin details include real screenshots with thumbnails and full-size viewing, plus links to the plugin's source project and release. Project metadata pins the source commit and package hash; publishing checks the source modules against the release ZIP and validates screenshot bytes and dimensions. The optional project feed preserves compatibility with older catalog clients. Independent examples: [region Agent](https://github.com/abnste/mewu-plugin-region-agent), [memory](https://github.com/abnste/mewu-plugin-journal-memory), and [interactive ruler](https://github.com/abnste/mewu-plugin-interactive-widget).

On this branch, opening a blackboard carries existing capture regions, images, HTML/SVG results, recordings and TXT notes into it as independent objects. Other documents stay out of the board. TXT notes have a plain white editing surface and four corner resize handles; edits save a copy without changing the original file. Capture and imported images support hover-wheel resizing and can move beyond any screen edge. The selection tool supports marquee selection and moving drawing objects together; a group move is one undo step. Local extraction repairs follow their source image when it moves or scales. Desktop pinned images use their context menu and have no hover toolbar. Capture images retain saved annotations; HTML remains interactive and videos retain playback, trimming and annotations. Hover over the top center of an object to reveal its move bar; drag it onto Delete to remove it. The eraser only removes ink. Brush width uses a slider; holding the right mouse button temporarily erases with the same diameter, including single-point strokes. Drawing tools use a custom RGB/HEX palette with three recent colors. Toolbar buttons keep the same square dimensions with labels shown or hidden.

Drawing arrows use open heads that adapt to their length. Screenshot and blackboard highlighters tint the background while preserving dark, light and colored text and antialias detail, including committed mosaic, repair and extracted-image content. Display, image copy, saving and image references use the same foreground separation rules. Hold Shift with the highlighter pen to constrain the stroke horizontally, vertically or to 45°; release Shift to return to freehand ink. Highlight region draws a filled rounded rectangle with a thin border; select it to move, resize or recolor it, and hold Shift for a square. Highlights currently follow the saved screenshot document; tracking refreshed desktop content is not yet available.

Drawing and recording remain built in. Agent and blackboard are optional official plugins that can be disabled or removed. Without a ready model connection or an explicitly bound independent Agent, capture is the default interface and AI conversation controls stay hidden. API 1 JSON packages remain compatible. API 2 plugins execute ES Modules in separate QuickJS processes and dynamically register commands, tools, Agent and memory providers, models, channels, renderers and UI services. Plugins use permission-checked host interfaces rather than ambient filesystem or operating-system access. Installation reviews permissions and verifies the actual GitHub Release ZIP against its recorded SHA256 and size. See the [SDK](packages/sdk/README.md), [extension guide](docs/extensions.md), [packaging tools](tools/extension-market/README.md) and [community catalog template](community-market/README.md). Region observation and bounded desktop input are initially available on Windows; other platforms are not advertised as supporting that input capability.

Hermes, Codex, WorkBuddy and MiniMax Code agent integrations, device synchronization and dedicated image/video generation APIs are not included. The Agent plugin is not a complete replacement for Hermes and Hindsight. Actual annotation quality depends on the model.

Only explicitly minimized conversations appear in the floating list and its count. Escape closes ordinary conversations into history; a restored minimized conversation returns to the floating list. Its preview follows the latest public reasoning and answer chunks, then shows the beginning of the final answer with “...” for long content. Adding a reference inserts the corresponding @image or @video name at the input caret while retaining its attachment identity.

Settings → General can disable automatic chat titles. By default, after the first user request receives a complete answer, a separate request asks the same selected model for a short task title. It uses only the first prompt, excludes memory and attachments, and does not add another chat message. Title failures leave the answer intact; manual names are preserved.

HTML/SVG/Canvas results are transparent interactive objects with hover controls for moving, reloading and closing. Video actions appear above the video and quick trim below it by default and avoid nearby controls when space is limited. Long captures show their stitched content in a live side preview.

Automatic window selection enables **Snapshot** on the long-capture button. Supported Windows UI Automation viewports scroll automatically; WGC reads the selected window, the side preview updates as pixels are stitched, and the original scroll position is restored before saving. The result keeps the selection identity and becomes an input reference. With the official pin plugin enabled, it also becomes an independent pinned image that remains available after closing the capture or switching conversations. Hold Shift when clicking to use manual scrolling. Failed continuity checks, a changed source window or unconfirmed restoration discard the incomplete snapshot.

## Installation and updates

Preview 4 uses the following package names. Check the releases page for published availability:

| Platform | Download |
| --- | --- |
| Windows x64 | `MewuAI-Remake-Setup-1.0.0-preview.4-win-x64.exe` · `MewuAI-Remake-Portable-1.0.0-preview.4-win-x64.zip` |
| macOS ARM64 | `MewuAI-Remake-1.0.0-preview.4-mac-arm64.dmg` |
| Linux x64 | `MewuAI-Remake-1.0.0-preview.4-linux-x64.AppImage` |

Windows requires Windows 10 2004 or later and WebView2. macOS uses the system WKWebView. Linux uses WebKitGTK; Wayland capture and recording request permission through the desktop portal.

After granting Wayland recording access, select the recording region again in the approved preview. Move the recording controls outside the region to keep them out of the video; GNOME does not guarantee that the controls remain on top.

Linux requires the system `libnghttp2.so.14` provider (`libnghttp2-14` on Ubuntu). Local OCR uses system Tesseract 4.1 or 5 and English language data (`tesseract-ocr-eng` on Ubuntu).

The application checks for updates in the background. Settings → About provides **Check for updates** and **Update and restart**. Every update requires an installer signature bound to its version; the installer starts after the application saves and drains active work. Updates use a fixed release channel, not a branch URL. Moving this code to master or renaming next does not change the installed updater.

Updating a portable build runs the installer and switches to the installed application; it does not replace the executable in the ZIP extraction directory.

Legacy 0.7.4 can move to preview 4 through the [legacy update entry](https://github.com/abnste/mewu_ai/releases/tag/v1.0.0.4). Its numeric compatibility tag is required by the old updater; the installer is the unchanged `1.0.0-preview.4` installer. After installation, updates use the preview channel. Versions 0.7.3 and earlier should first install [0.7.4](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4). Supported legacy API connections are imported once from local settings. Legacy data is retained, but legacy conversation history is not converted into the new scene database. See [upgrade details](docs/legacy-updates.md).

New installations store data in `~/.mewu`. Existing remake installations retain their configured directory; Settings → Data can move it. Signing updates is separate from Windows Authenticode signing; this preview installer has no Windows signing certificate.

Settings → Data also shows stored file, image and conversation sizes, with separate cleanup actions. Cleanup protects open conversations, memory sources and referenced media. Unreferenced app-owned copies become eligible after one day. Conversation sizes are logical payload sizes; the database row and cleanup result show actual disk usage.

## Development

Use Node.js 24, Rust stable, Windows C++ build tools and WebView2.

```powershell
npm ci
npm run desktop:dev
```

```powershell
node --experimental-vm-modules --test apps/desktop/src/*.test.mjs
cargo test --workspace --locked
npm run build
```

The [release workflow](.github/workflows/preview-release.yml) builds unsigned candidates for native testing. Publishing requires matching acceptance records for all three platforms and updater signatures from a protected private key. Signatures are embedded in the fixed update manifest; standalone `.sig`, checksum and private metadata files are not added to the version downloads. Browser development mode previews the interface and does not provide native capture, recording or storage.

| Directory | Purpose |
| --- | --- |
| [crates/mewu-core](crates/mewu-core) | Scenes, conversations, agent identity, memory and run records |
| [apps/desktop/src-tauri](apps/desktop/src-tauri) | Native windows, capture, media, connections, updater and plugin host |
| [apps/desktop/src](apps/desktop/src) | Solid/TypeScript interface |
| [apps/desktop/plugins](apps/desktop/plugins/README.md) | Official declarative plugins and API contract |
| [tools/plugin-catalog](tools/plugin-catalog/README.md) | Community catalog validation |

## License

Mewu source remains [MPL-2.0](LICENSE). Full third-party notices are provided in [THIRD-PARTY-NOTICES](apps/desktop/THIRD-PARTY-NOTICES) and included with downloads. Source for each released build is linked from its version tag. [Report an issue](https://github.com/abnste/mewu_ai/issues).
