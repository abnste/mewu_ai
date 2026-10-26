<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI" />
  <h1>MewuAI · 喵呜AI</h1>
  <p><strong>An open-source, LLM-powered desktop Agent application</strong></p>
  <p>Personal Agent · AI creative canvas · Visual annotations · Desktop tools and automation</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/latest"><img src="https://img.shields.io/github/v/release/abnste/mewu_ai?style=flat-square&color=7C6CF0" alt="Latest stable release" /></a>
    <img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat-square" alt="Windows x64" />
    <img src="https://img.shields.io/badge/macOS-Apple_Silicon-555555?style=flat-square" alt="macOS Apple Silicon" />
    <img src="https://img.shields.io/badge/Linux-x64-E95420?style=flat-square" alt="Linux x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="MPL-2.0" /></a>
  </p>
  <p>
    <a href="https://www.mewu.ai/"><strong>Website & demos</strong></a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu_ai/releases/latest"><strong>Download MewuAI</strong></a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu-plugins">Plugin marketplace</a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu_ai/issues">Issues & ideas</a>
  </p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong></p>
</div>

**MewuAI is a personal Agent application for Windows, macOS and Linux.** Talk to AI with screenshots, videos, files and hand-drawn annotations as context. Generate interactive HTML tools, use the results on your canvas, or add plugins to operate an authorized screen region, retrieve memories and automate tasks.

**Mewu with you** means working together. Point to a detail, draw a connection, or reference several images to explain exactly what you mean. Keep the resulting content in the same workspace to revise and use.

[Features](#features) · [Use cases](#use-cases) · [Get started](#get-started) · [AI connections](#ai-connections-and-personal-agents) · [CLI](#cli-share-the-desktop-space-with-other-agents) · [Hooks](#hooks-event-automation) · [Plugins](#plugin-marketplace) · [Development](#development-and-contributing)

## Features

### Personal Agents and conversations

Create Agents for different tasks, with a name, instructions, default model and memory provider. Select an Agent and model in the conversation bar, reference regions, images, videos or files, and keep the conversation going. Multiple conversations, history, streaming replies, Markdown, math and tool execution records are supported.

Configure the Agent, model connection and memory separately. Use Mewu's Agent or connect an existing Agent runtime through a plugin. MCP and plugin tools can participate in the same task.

### AI creative canvas and interactive tools

Describe an **HTML / SVG / Canvas interactive object** you want: a small game, calculator, adjustable chart or teaching aid. Move and resize the result in the workspace, interact with it, and ask for further changes.

Screenshots, images, videos, blackboards and generated objects can share one space. The blackboard provides freehand drawing, text, shapes, undo and redo. With the Toolbox plugin, save materials and tools and drag them into another conversation for reuse.

### Selections, annotations and visual communication

- **Single and multiple regions:** Select several areas in succession. Completed selections automatically become conversation references; the surrounding screen is dimmed to make the task's scope clear.
- **Manual markup:** Use pens, arrows, shapes, text, numbered markers, highlights and mosaic, with selection, erasers, undo and redo to explain locations, changes or relationships.
- **In-place AI answers:** Ask AI to mark important areas, add explanations or draw diagrams on screenshots and images, or answer using several references. Annotations are stored separately from the original image.
- **Capture tools:** Copy, save, pin and capture scrolling content. Automatic application snapshots depend on Windows UI Automation and the target application's support.

### Video annotations and screen recording

Import a video or record a screen region, then play, trim, reference and discuss it in the same workspace. AI can add rectangles and text for specific time intervals. Click a time segment in its answer to navigate to the relevant footage.

Recording supports computer audio, microphone input and pause/resume. Trimming preserves the original source, with **MP4, MP3 and GIF** export. Video understanding and AI annotations depend on the selected model's visual capabilities.

### Text, tables and charts

- **Offline OCR:** Recognize text in screenshots, select the portion you need and copy it.
- **In-place translation:** Read translated text where the source text appears.
- **Table extraction:** Turn a screenshot table into structured rows and columns you can copy into a spreadsheet.
- **Chart understanding:** Reference a chart and ask about values, trends and differences, or ask AI to mark areas of interest.
- **QR codes and barcodes:** Recognize codes in a selection and use copy/open actions.

Capture, manual drawing, recording and local OCR do not need a model account. Translation, AI table extraction, chart understanding and content generation require a suitable model or Agent.

### Region AI operations

With the [region-operation plugin](https://github.com/abnste/mewu-plugin-region-agent), reference and authorize a region in the existing conversation bar, then describe the task. The Agent observes the screenshot, clicks or types, and uses a fresh screenshot to decide what to do next. You can stop it during the task.

Native input supports Windows, macOS, and **GNOME 50 / Wayland / x64** with [Mewu Desktop Witness](./tools/linux-desktop-witness/README.md). macOS needs screen recording and Accessibility permissions; Linux needs Portal authorization. Other Linux desktops are not yet supported for region input.

## Use cases

These are example requests to use with your own content. Answers and generated results depend on the model you choose.

| Scenario | What to do or ask |
| --- | --- |
| Interface review | Select a button and its explanatory text, draw an arrow between them, and ask: “These repeat the same information. Suggest a clearer layout and annotate the changes.” |
| Code and error explanations | Reference code and an error message together: “Explain how these relate and mark the places I should check.” |
| Papers and learning | Reference a question and the working: “Find the incorrect step, explain it on the image, and give me a similar practice problem.” |
| Video review | Reference meaningful footage: “Mark key moments and explain what happens in each interval.” Jump from the answer back to the video to check. |
| Research | Select text, tables and charts to extract data, translate passages or compare trends. Save conclusions and references in a note. |
| Interactive tools | Ask for an adjustable demonstration or turn a calculation into a calculator. Use it on the canvas, then request revisions. |
| Desktop tasks | Authorize the target region and ask the Agent to click, fill in fields and check results, continuing from fresh screenshot feedback. |
| Repeated workflows | Use Hooks schedules, keyword rules or shortcuts to connect prompts, MCP tools and local operations. |

See interface and workflow demonstrations at [mewu.ai](https://www.mewu.ai/). Check teaching, recognition and analysis results against the original material.

## Get started

### Download and install

Installers and release notes are published on the [latest stable GitHub Release](https://github.com/abnste/mewu_ai/releases/latest).

| Platform | Package | Requirements |
| --- | --- | --- |
| [Windows x64](https://github.com/abnste/mewu_ai/releases/latest) | `Setup…exe` installer / `Portable…zip` | Windows 10 2004 or later, using WebView2 |
| [macOS Apple Silicon](https://github.com/abnste/mewu_ai/releases/latest) | `mac-arm64.dmg` | Move the app into Applications and grant the permissions requested by macOS |
| [Linux x64](https://github.com/abnste/mewu_ai/releases/latest) | `linux-x64.AppImage` | Make it executable, then launch; uses WebKitGTK and desktop portals |

1. **Open a space.** Launch MewuAI and click its tray icon or use the capture shortcut shown in Settings. Select an area to annotate, copy or save; you can also open a blackboard or import materials.
2. **Connect AI.** Configure a model under **Settings → Agent and connections → Connections**, or install an external Agent plugin, configure its connection and bind it to an Agent.
3. **Explain the task.** Select an Agent, capture the relevant regions or add files, and type your request. You can draw annotations before sending.
4. **Add tools.** Install extensions under **Settings → Plugins**, and create event rules in **Hooks**.

Linux needs system `libnghttp2.so.14`. Local OCR requires Tesseract 4.1/5 and English language data: `libnghttp2-14` and `tesseract-ocr-eng` on Ubuntu. Wayland capture and recording require desktop authorization; after recording permission is granted, confirm the region again in the refreshed preview. See [Linux OCR](./docs/linux-native-ocr.md), [recording](./docs/linux-native-recording.md) and [package permissions](./docs/linux-appimage-permissions.md).

## AI connections and personal Agents

| Connection | Purpose |
| --- | --- |
| Model APIs | Chat Completions, OpenAI Responses and Anthropic Messages. Save multiple connections, use provider presets or custom URLs, discover models and switch between them. |
| External Agents | Connect **Codex, Hermes, MiniMax Code, WorkBuddy / CodeBuddy** through the [External Agents plugin](https://github.com/abnste/mewu-plugin-agent-connectors) and use them in the existing conversation bar. |
| MCP | Configure services under **Agent and connections → MCP** to make their tools available to the current Agent. |
| Long-term memory | Choose local SQLite/FTS5 memory or an independent provider such as Hindsight or Journal Memory for each Agent. |

The external Agent plugin uses Codex App Server or ACP and relies on a compatible installed client's login and runtime configuration. Conversation, image input and session restoration are provided according to each adapter's support. Provider-specific video upload and a complete tool-approval interface are not yet integrated. See the [plugin documentation](https://github.com/abnste/mewu-plugin-agent-connectors#平台接口) for individual client requirements.

Select the model, Agent and memory provider independently. Save distinct instructions for different roles, keep each role's memory, and switch models to suit the task.

## CLI: share the desktop space with other Agents

The official CLI plugin ships with MewuAI and is enabled by default. Start MewuAI, then open a terminal:

```sh
mewu
mewu chat "Help me break this task into steps"
mewu status
mewu schema
mewu call capture.open --json '{}' --wait
mewu scene list
```

Run `mewu` for interactive Agent chat. Local Agents can also use the CLI to **open capture, create regions, set references, draw annotations, export images, run OCR and control recording**. The CLI and desktop share conversations, models, memory and tools.

Automation clients should read `mewu schema`, then use the actual scene, region and revision data returned by the host. Writes return operation receipts; `--wait` waits for that operation, not necessarily an entire AI response. Terminal chat sends text by default; image references must be set explicitly.

Open a new terminal after installing on Windows. On macOS/Linux, starting an installed app attempts user-level CLI integration. The Linux AppImage's local CLI connection requires Linux 6.5+. See the [bilingual CLI guide](./docs/cli.md) and [interface catalog](./apps/desktop/control/catalog.json).

## Hooks: event automation

Hooks is an official native plugin shipped and enabled by default. Manage rules in the settings sidebar, with independent enable/disable controls, editing and execution history.

| Trigger | Example use |
| --- | --- |
| Schedule / Cron | Send a task to an Agent or run a local workflow at a specified time, with time-zone support. |
| Prompt | Add instructions to the current request using keyword, exact or regular-expression matching. |
| AI reply | Match a completed reply and perform follow-up actions. |
| Global shortcut | Open capture, create a blackboard or start a custom workflow. |
| Lifecycle / custom event | Respond to application, scene, Agent, tool or plugin events with conditional actions. |

Combine **prompt injection, messages, MCP, commands and programs, websites, keyboard/mouse input, JavaScript and Mewu operations**. Capture shortcuts are ordinary Hooks rules too. Rules include cooldown, timeout and stop controls; operating-system permissions still apply.

See the [Hooks guide](./docs/hooks.md) for configuration and examples.

## Plugin marketplace

Use **Settings → Plugins** to browse the marketplace, manage installed extensions and toggle native features. Community plugins are installed from their own GitHub Releases, with source, permissions and version information available for review.

| Plugin | Purpose |
| --- | --- |
| [External Agents](https://github.com/abnste/mewu-plugin-agent-connectors) | Connect Codex, Hermes, MiniMax Code and WorkBuddy / CodeBuddy. |
| [Region AI operations](https://github.com/abnste/mewu-plugin-region-agent) | Observe, click, type and verify results within an authorized region; listed as “区域操作 Agent” in the marketplace. |
| [Hindsight](https://github.com/abnste/mewu-plugin-hindsight) | Connect a long-term memory service, select existing banks, and save or retrieve memories. |
| [Journal Memory](https://github.com/abnste/mewu-plugin-journal-memory) | Persist and search memories separately for each Agent. |
| [Toolbox](https://github.com/abnste/mewu-plugin-object-toolbox) | Save images, videos, generated objects and blackboards for reuse across conversations. |
| [Research Notes](https://github.com/abnste/mewu-plugin-research-notes) | Edit floating notes and retain their text and attached object references. |
| [Interactive ruler](https://github.com/abnste/mewu-plugin-interactive-widget) | A movable, interactive transparent SVG tool and an example of an interactive-object plugin. |

[Community marketplace](https://github.com/abnste/mewu-plugins) · [Mewu API / SDK](./packages/sdk/README.md) · [Extension guide](./docs/extensions.md) · [Packaging and publishing](./tools/extension-market/README.md#from-an-independent-project-to-a-market-pr)

Plugins can provide tools, Agents, memory, settings pages and workspace objects. JavaScript plugins run in separate QuickJS processes through authorized host interfaces; users do not need Node.js to install them. Plugins that launch external clients still require those clients. Disabling a plugin revokes its interfaces and execution capabilities while retaining its saved data.

Planned directions include reference-image generation and broader image/video workflows, desktop pets, circle-to-search, more plugins and additional Linux desktop support. These are roadmap items; release notes and published marketplace packages describe what is available.

## Data, permissions and updates

- **Local data:** New installations default to `~/.mewu`. Use **Settings → Data** to inspect storage, move the data directory or clean up content.
- **AI requests:** Capture and manual drawing happen locally. Sending an AI request passes its references and relevant context to the selected model or Agent service. Plugins access the capabilities you authorize.
- **Platform differences:** Region input, recording and OCR depend on system capabilities and permissions. QR/barcodes and actionable link, email and phone recognition currently support Windows/macOS. Native formula rendering on images currently supports Windows; chat formulas use the shared renderer on all three platforms.
- **Screen sharing:** Adjust **Settings → General → Allow screen sharing to see selections and annotations**. Visibility also depends on the operating system and sharing application.
- **Updates:** Check **Settings → About** for updates from stable GitHub Releases. Automatic updates of Windows portable builds run the installer and switch to an installed build; download a new portable ZIP to remain portable.
- **Installation help:** Windows packages do not yet have an Authenticode certificate. Windows N/KN recording may require the Media Feature Pack. Include the system, version and reproduction steps when reporting an [issue](https://github.com/abnste/mewu_ai/issues).

## Development and contributing

MewuAI uses **Rust + Tauri 2 + Solid + TypeScript + SQLite**, with the system WebView. Current source is on `master`.

| Directory | Contents |
| --- | --- |
| [apps/desktop](./apps/desktop) | Desktop frontend and native Tauri host |
| [crates](./crates) | Core data, control protocol, Hooks and extension runtime |
| [tools/mewu-cli](./tools/mewu-cli) | Local command-line client |
| [packages/sdk](./packages/sdk) | Extension development SDK |
| [extensions](./extensions) | Plugin source and examples |
| [docs](./docs) | Feature, platform and release documentation |

Use `npm ci` and `npm run dev` for frontend development, and `npm run desktop:dev` for native development. Browser previews do not provide system capture, native recording or real desktop input. Native development requires the platform's Rust/Tauri toolchain; Linux builds use the project's complete Docker workflow.

[Desktop development guide](./apps/desktop/README.md) · [Linux builds and testing](./docs/linux-testing.md) · [Release process](./docs/releasing.md)

Issues, use cases, translations, documentation, code and plugins are welcome. Thank you to [all project contributors](https://github.com/abnste/mewu_ai/graphs/contributors).

## License and acknowledgments

MewuAI is licensed under [MPL-2.0](./LICENSE). Dependency and font licenses are included in the [third-party notices](./apps/desktop/THIRD-PARTY-NOTICES).

Thanks to teachers **Lin Wenjie** for subject-matter guidance and **Zhang Wenbiao** for feature development suggestions, and to [**blueling0**](https://github.com/blueling0) for testing assistance.

Related project: [**kangarooking/Ta (拓)**](https://github.com/kangarooking/Ta), an AI screenshot tool for macOS.
