<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI">
  <h1>MewuAI</h1>
  <p><strong>An LLM-powered personal Agent and AI creative canvas</strong></p>
  <p>Mewu with you — see, draw, create, and get things done together.</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/latest"><img src="https://img.shields.io/github/v/release/abnste/mewu_ai?style=flat-square&color=7055e7" alt="Latest release"></a>
    <img src="https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-18181b?style=flat-square" alt="Windows, macOS, Linux">
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-7055e7?style=flat-square" alt="MPL-2.0"></a>
  </p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong> · <a href="https://www.mewu.ai/?lang=en">Website &amp; interactive demos</a> · <a href="https://github.com/abnste/mewu_ai/releases/latest">Latest release</a></p>
  <p><a href="https://www.mewu.ai/download.php?platform=windows"><strong>Download for Windows</strong></a>　·　<a href="https://www.mewu.ai/download.php?platform=macos"><strong>Download for macOS</strong></a>　·　<a href="https://www.mewu.ai/download.php?platform=linux"><strong>Download for Linux</strong></a></p>
</div>

MewuAI is an open-source desktop Agent app. Select content on your screen, point out details with text, arrows, and freehand marks, then discuss it, generate interactive tools, and organize materials in one space. CLI, MCP, and plugins connect that work to actions. Capture, recording, and annotation give you ways to express what you mean; your chosen model, Agent, and memory work with that context.

![Select multiple references, ask the personal Agent for a plan, and use the generated checklist on the canvas](./docs/media/readme/agent-overview.gif)

**Share the context and work with the result.** Selections become references automatically. Generated HTML stays on the canvas, ready to click, type into, drag, and refine through conversation.

[Feature demos](#feature-demos) · [Use cases](#use-cases) · [Plugin marketplace](#plugin-marketplace) · [Get started](#get-started) · [Development](#development-and-contributing) · [Project members](#project-members)

## Feature demos

### Select a region to reference it

Select several areas of your desktop in sequence. Each selection automatically joins the current conversation as a reference, while the surrounding screen dims. Compare passages, images, or content from different windows without sending the whole screen or clicking a separate “Reference” button.

- Add freehand marks, arrows, rectangles, and text to show exactly what needs attention.
- View results in place, or copy, save, and pin content for later work.
- Use single-region capture, multiple selections, scrolling capture, and recording in the same desktop workspace.

![Select three areas in sequence and automatically add them as references](./docs/media/readme/multi-region.gif)

### An AI creative canvas for materials and conversations

Arrange images, videos, text, and generated objects on one canvas. Resize and move them as you build a set of references. Projects and conversations retain their own content, while Agent roles, model connections, and memory can be configured independently. Use the canvas to compare design drafts, organize research, discuss a composition, or develop a creative project.

- **Multimodal conversations:** reference images, videos, and files, retain conversation history, and read Markdown, formulas, and collapsible public reasoning.
- **Roles and memory:** customize an Agent, choose its model and tools, and use persistent local memory or connect an independent memory service through a plugin.

![Arrange storyboard references, images, and a conversation on the creative canvas](./docs/media/readme/creative-canvas.gif)

### Generate HTML tools you can actually use

Describe a calculator, visual experiment, task panel, or small game and ask AI to generate HTML / SVG / Canvas content. The result becomes an interactive object in your workspace: type, click, drag, and ask for changes. This particle experiment demonstrates mode switching and parameter adjustment.

![Generate an interactive particle experiment, switch movement modes, and adjust its parameters](./docs/media/readme/interactive-tools.gif)

### Video annotations and timestamp navigation

Import a video or record your screen, add rectangles and text for specific time ranges, then click a timestamp in the conversation to return to that moment. The example below uses a tabby cat walking along a fence to show annotations across successive segments, playback, and timestamp navigation.

![Tabby cat video with timed target annotations and timestamp navigation](./docs/media/readme/video-annotations.gif)

Annotations appear during their assigned time ranges; successive ranges can use different positions. Recording also supports system audio and microphone input, pause and resume, non-destructive trimming, and MP4, MP3, and GIF export. Audio availability depends on the operating system.

### A blackboard for drawing and explaining

Use freehand strokes, shapes, formulas, and color to explain your reasoning. Work alongside a captured question or ask AI to analyze your drawing. For presentations, a setting lets screen-sharing software see your selections and annotations.

![Draw axes, a function curve, and notes on a full-screen blackboard](./docs/media/readme/blackboard.gif)

### OCR, translation, and table extraction

Everyday capture tools are included: extract text with local OCR, translate a selection, or use an image-capable model to explain a chart and turn a captured table into reusable data. These examples show text extraction, translation beside the source, and table organization.

<table>
  <tr>
    <td width="50%"><a href="./docs/media/readme/ocr.png"><img src="./docs/media/readme/ocr.png" alt="Extract text from a selected screen region"></a><br><strong>OCR</strong> · Extract text from a capture</td>
    <td width="50%"><a href="./docs/media/readme/translate.png"><img src="./docs/media/readme/translate.png" alt="Translate English text in a selected region"></a><br><strong>Translation</strong> · Read the result beside the source</td>
  </tr>
</table>

![Recognize a captured table and organize its data](./docs/media/readme/table.png)

Windows and macOS also offer actionable recognition of links, email addresses, phone numbers, QR codes, and barcodes.

### CLI control for other local Agents

`mewu` provides both terminal chat and fixed operations for screenshots, selections, drawing, OCR, recording, and image export. Another local Agent can inspect the current scene, submit an operation, and check its result while sharing the same desktop space with you.

![CLI capture, selection, and drawing calls update the Mewu interface on the right](./docs/media/readme/cli-screen-control.gif)

Start the desktop app, then open a terminal:

```sh
mewu                         # Start an Agent conversation
mewu status                  # Inspect the app and current scene
mewu schema                  # Discover operations and parameters
mewu call capture.open --json '{}' --wait
```

Plain `mewu chat` sends text by default; screenshots and files need explicit references through the relevant interface. See the [CLI guide](./docs/cli.md) and [operation catalog](./apps/desktop/control/catalog.json) for parameters, shell integration, and Linux requirements.

## Use cases

| What you are working on | How Mewu helps |
| --- | --- |
| Reading and research | Select several sources, ask AI to compare them, and organize the findings on the canvas or in notes. |
| Design and content review | Circle a detail, draw an arrow, and specify what should change and what should stay. |
| Teaching and problem solving | Reference a question, draw the steps on a blackboard, and ask AI to help check the reasoning. |
| Video review | Annotate key time ranges and revisit them through timestamp buttons in the conversation. |
| Small interactive tools | Describe the interaction, generate an HTML tool, and refine it as you use it. |
| Desktop workflows | Combine CLI, region AI operations, or Hooks with capture, tool calls, and local actions. |

## Plugin marketplace

Browse, install, and manage extensions in **Settings → Plugins**. Agent, memory, tool, and interface capabilities can be extended independently. Install from the marketplace or import a release ZIP. Browse the [full plugin marketplace](https://github.com/abnste/mewu-plugins).

### Hooks · the official event trigger plugin

Hooks is included with Mewu and enabled by default. Open **Hooks** in the settings sidebar to combine a trigger with actions. Each rule can be enabled, disabled, edited, and inspected through its run history.

| Trigger | Example follow-up actions |
| --- | --- |
| A schedule (Cron) | Send a task, open an app, or run a command. |
| A matching user prompt | Inject instructions into that request or call a tool. |
| A completed AI reply | Call a configured MCP tool or continue a local workflow. |
| A global shortcut | Open capture, create a blackboard, or execute a sequence of actions. |
| An app, plugin, or custom event | Run conditional Mewu operations, keyboard/mouse actions, or other events. |

Rules support ordered actions, conditions, cooldowns, and run history. See the [Hooks guide](./docs/hooks.md).

### Agent, memory, and tool extensions

| Plugin | What it adds |
| --- | --- |
| [External Agents](https://github.com/abnste/mewu-plugin-agent-connectors) | Connect Codex, MiniMax Code, WorkBuddy / CodeBuddy, or Hermes to the existing conversation bar, using the client's own login and configuration. |
| [Region AI operations](https://github.com/abnste/mewu-plugin-region-agent) | Select a target and describe a task. The model observes the region, acts, then checks a fresh screenshot before continuing. |
| [Hindsight memory](https://github.com/abnste/mewu-plugin-hindsight) | Connect your Hindsight service and memory banks, and assign long-term memory to an Agent. |
| [Journal memory](https://github.com/abnste/mewu-plugin-journal-memory) | Choose an independent journal-based memory provider for an Agent. |
| [Mewu toolbox](https://github.com/abnste/mewu-plugin-object-toolbox) | Save images, videos, HTML, and drawing objects; organize them in folders and drag out copies for reuse. |
| [Research notes](https://github.com/abnste/mewu-plugin-research-notes) | Record, organize, and reuse notes in the workspace. |
| [Interactive ruler](https://github.com/abnste/mewu-plugin-interactive-widget) | Add a movable, adjustable HTML / SVG measuring tool to the canvas. |

External Agents require the corresponding client to be installed and signed in; check each plugin's documentation for its supported scope. Region AI operations support Windows, macOS, and compatible GNOME 50 Wayland x64 setups. Linux also requires Desktop Witness and system Portal authorization, as described in the plugin guide.

**Planned:** reference-guided image generation and dedicated image/video model connections, circle-to-search, and a desktop companion plugin. These are roadmap items, not listed as shipped features.

## Get started

| Platform | Download and installation |
| --- | --- |
| **Windows x64** | [Installer](https://www.mewu.ai/download.php?platform=windows) · [Portable ZIP](https://www.mewu.ai/download.php?platform=portable). Uses the system WebView2 runtime. |
| **macOS Apple Silicon** | [Download the DMG](https://www.mewu.ai/download.php?platform=macos), move the app to Applications, and grant screen-recording permissions when requested. |
| **Linux x64** | [Download the AppImage](https://www.mewu.ai/download.php?platform=linux), make it executable, and launch it. See [AppImage and desktop permissions](./docs/linux-appimage-permissions.md). |

Download links follow the latest stable release. All packages and release notes are available on [GitHub Releases](https://github.com/abnste/mewu_ai/releases/latest).

1. **Try the desktop tools.** Enter the workspace from the tray or a capture shortcut. Select, draw, save, or open a blackboard. Capture shortcuts are configured in Hooks.
2. **Connect AI.** In **Settings → Agents & Connections**, add a model service and choose an Agent, model, and memory. You can also connect an existing client through the External Agents plugin.
3. **Ask with context.** Selections become references automatically. Send a task from the conversation bar, and enable MCP or plugin tools as needed.

Model connections support Chat Completions, OpenAI Responses, and Anthropic Messages APIs, with multiple independent connections. Image, video, and tool-calling capabilities depend on the selected model and service.

### FAQ

<details>
<summary><strong>Do I need an AI account? Is it paid?</strong></summary>

Capture, manual drawing, recording, and local OCR do not require an AI account. AI conversations and generated content use your configured service or external Agent; that provider determines usage limits and charges. MewuAI itself is open source.

</details>

<details>
<summary><strong>Where do my captures and conversations go?</strong></summary>

Capture and manual drawing happen locally. When you use AI, referenced content and relevant context are sent to the service you select. Plugins use the permissions you grant, and external Agents retain their own runtime configuration.

</details>

<details>
<summary><strong>How do I update or report a problem?</strong></summary>

Check for updates in **Settings → About**, or download a package from the [latest release](https://github.com/abnste/mewu_ai/releases/latest). The Windows portable build's built-in update switches to the installed edition; download a new portable ZIP if you want to keep using it portably.

Report problems through [Issues](https://github.com/abnste/mewu_ai/issues), including your operating system, app version, reproduction steps, and relevant screenshots. Capture and recording depend on system permissions and the desktop environment; Linux users can consult the [recording guide](./docs/linux-native-recording.md).

</details>

## Development and contributing

The desktop app uses **Rust + Tauri 2 + SolidJS / TypeScript**, renders through the system WebView, and stores projects and conversations in SQLite. Browser previews support interface development; native capture, recording, and system input require the desktop runtime.

| Guide | Contents |
| --- | --- |
| [Desktop development](./apps/desktop/README.md) | Environment setup, development, and builds. |
| [CLI](./docs/cli.md) · [Operation catalog](./apps/desktop/control/catalog.json) | Control Mewu from scripts and other local Agents. |
| [Hooks](./docs/hooks.md) | Events, conditions, actions, and execution rules. |
| [Extension SDK](./packages/sdk/README.md) · [Extension interfaces](./docs/extensions.md) | Build independent plugins for tools, Agents, memory, and UI services. |
| [Plugin packaging and submissions](./tools/extension-market/README.md) | Publish an independent plugin and submit it to the marketplace. |
| [Release guide](./docs/releasing.md) | Desktop packages and the release process. |

Contributions of code, plugins, documentation, use cases, and bug reports are welcome.

## Project members

<table>
  <tr>
    <td align="center"><a href="https://github.com/abnste"><img src="https://github.com/abnste.png?size=100" width="48" alt="Abner Stephen"><br>Abner Stephen</a></td>
    <td align="center"><a href="https://github.com/pyxyd"><img src="https://github.com/pyxyd.png?size=100" width="48" alt="Yandi"><br>Yandi</a></td>
    <td align="center"><a href="https://github.com/shuziyuxingxing-stack"><img src="https://github.com/shuziyuxingxing-stack.png?size=100" width="48" alt="数字与星星"><br>数字与星星</a></td>
    <td align="center"><a href="https://github.com/AndeDrCobb"><img src="https://github.com/AndeDrCobb.png?size=100" width="48" alt="Cobb"><br>Cobb</a></td>
    <td align="center"><a href="https://github.com/tljsj"><img src="https://github.com/tljsj.png?size=100" width="48" alt="tljsj"><br>tljsj</a></td>
  </tr>
</table>

[View all contributors](https://github.com/abnste/mewu_ai/graphs/contributors)

## License and acknowledgments

MewuAI's own source code is licensed under [MPL-2.0](./LICENSE), which permits commercial use subject to its terms. Distributions must provide covered source and preserve copyright and license notices as required. [Third-party notices](./apps/desktop/THIRD-PARTY-NOTICES) ship with the app.

Thanks to **Lin Wenjie (林文杰)** for subject-matter guidance, **Zhang Wenbiao (张文彪)** for feature-development suggestions, and [**blueling0**](https://github.com/blueling0) for testing. Thank you to everyone who reports issues, shares use cases, or contributes to the project.

Demos use Mewu interface components with prepared sample content to show the workflow. Generated results vary by model and prompt. See [demo assets and credits](./docs/media/readme/README.md).
