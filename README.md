# Mewu AI

[简体中文](README.zh-CN.md)

Mewu 1.0 is a Windows desktop remake. A shortcut opens the capture space: screenshots, conversation, files, annotations and interactive HTML/SVG results share the same surface.

**This branch contains the 1.0 preview.** The stable 0.7.x application remains on [master](https://github.com/abnste/mewu_ai/tree/master). Download a preview from [Releases](https://github.com/abnste/mewu_ai/releases); prereleases are marked separately.

## Included

- Region capture, drawing, blackboards, borderless pinned images, OCR, translation, scrolling capture and barcode recognition.
- Multiple conversations, freeze/resume, Markdown and formulas, collapsible public reasoning, image and text attachments.
- Screen recording with computer audio by default, pause, trimming, video annotations, MP4 and GIF export.
- Agent identity, local SQLite/FTS5 memory, optional Hindsight connection, run history and resumable answers.
- Chat Completions, OpenAI Responses and Anthropic Messages connections; model templates and declared capabilities can be installed from public GitHub repositories.
- Local MCP tools, installed plugin management and a configurable GitHub plugin catalog.

On this branch, opening a blackboard carries existing capture regions, images, HTML/SVG results, recordings and TXT notes into it as independent objects. Other documents stay out of the board. TXT notes have a plain white editing surface and four corner resize handles; edits save a copy without changing the original file. Capture and imported images support hover-wheel resizing and can move beyond any screen edge. The selection tool supports marquee selection and moving drawing objects together; a group move is one undo step. Local extraction repairs follow their source image when it moves or scales. Desktop pinned images use their context menu and have no hover toolbar. Capture images retain saved annotations; HTML remains interactive and videos retain playback, trimming and annotations. Use the upper corner handle to move objects and drop them onto Delete to remove them. The eraser only removes ink. Brush width uses a slider; holding the right mouse button temporarily erases with the same diameter, including single-point strokes. Drawing tools use a custom RGB/HEX palette with three recent colors. Toolbar buttons keep the same square dimensions with labels shown or hidden.

Drawing, recording and AI annotations are built in. Plugins can be disabled or removed. Current plugins are declarative JSON packages: they cannot execute arbitrary downloaded code. Hermes, Codex, WorkBuddy and MiniMax Code agent integrations, macOS delivery, device synchronization and dedicated image/video generation APIs are not included in this preview. The current agent is not a complete replacement for Hermes and Hindsight. Actual annotation quality depends on the model.

Only explicitly minimized conversations appear in the floating list and its count. Escape closes ordinary conversations into history; a restored minimized conversation returns to the floating list. Its preview follows the latest public reasoning and answer chunks, then shows the beginning of the final answer with “...” for long content. Adding a reference inserts the corresponding @image or @video name at the input caret while retaining its attachment identity.

HTML/SVG/Canvas results are transparent interactive objects with hover controls for moving, reloading and closing. Video actions and quick trim appear below the video by default and avoid nearby controls when space is limited. Long captures show their stitched content in a live side preview.

## Installation and updates

Windows x64, Windows 10 2004 or later, and Microsoft WebView2 are required. Use the `MewuAI-Remake-Setup-…-win-x64.exe` installer; a portable ZIP is also provided.

The application checks for updates in the background. Settings → About provides **Check for updates** and **Update and restart**. Every update requires an installer signature bound to its version; the installer starts after the application saves and drains active work. Updates use a fixed release channel, not a branch URL. Moving this code to master or renaming next does not change the installed updater.

Legacy 0.7.4 already recognizes the future stable 1.x installer name. It intentionally ignores prereleases; 0.7.3 should first update to 0.7.4. Supported legacy API connections are imported once from local settings. Legacy data is retained, but legacy conversation history is not converted into the new scene database. See [upgrade details](apps/desktop/README.md).

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

Signed releases are built by the [release workflow](.github/workflows/preview-release.yml). Local builds require a protected updater signing key via `TAURI_SIGNING_PRIVATE_KEY`. Never put private keys in source or distributable folders. Browser development mode previews the interface and does not provide native capture, recording or storage.

| Directory | Purpose |
| --- | --- |
| [crates/mewu-core](crates/mewu-core) | Scenes, conversations, agent identity, memory and run records |
| [apps/desktop/src-tauri](apps/desktop/src-tauri) | Native windows, capture, media, connections, updater and plugin host |
| [apps/desktop/src](apps/desktop/src) | Solid/TypeScript interface |
| [apps/desktop/plugins](apps/desktop/plugins/README.md) | Official declarative plugins and API contract |
| [tools/plugin-catalog](tools/plugin-catalog/README.md) | Community catalog validation |

## License

Mewu source remains [MPL-2.0](LICENSE). Full third-party notices are provided in [THIRD-PARTY-NOTICES](apps/desktop/THIRD-PARTY-NOTICES) and included with downloads. Source for each released build is linked from its version tag. [Report an issue](https://github.com/abnste/mewu_ai/issues).
