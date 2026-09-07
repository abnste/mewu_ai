<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI" />
  <h1>MewuAI · 喵呜AI</h1>
  <p><strong>Capture your screen. Let AI annotate, translate, and explain it in place.</strong></p>
  <p>A Windows screenshot tool with on-screen AI assistance</p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong> · <a href="#in-action">In action</a> · <a href="https://github.com/abnste/mewu_ai/issues">Feedback</a></p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.3.0"><img src="https://img.shields.io/badge/Public_Beta-v0.3.0-7C6CF0?style=flat-square" alt="v0.3.0 Public beta" /></a>
    <img src="https://img.shields.io/badge/Windows-10_2004%2B-0078D4?style=flat-square" alt="Windows 10 2004 or later" />
    <img src="https://img.shields.io/badge/Architecture-x64-64748B?style=flat-square" alt="x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="License: MPL-2.0" /></a>
  </p>
  <p><a href="https://github.com/abnste/mewu_ai/releases/download/v0.3.0/MewuAI-Setup-0.3.0-win-x64.exe"><strong>Download installer</strong></a> &nbsp; · &nbsp; <a href="https://github.com/abnste/mewu_ai/releases/download/v0.3.0/MewuAI-Portable-0.3.0-win-x64.zip">Portable ZIP</a></p>
  <p><a href="./docs/release-notes-v0.3.0.md">What's new in 0.3.0</a> · <a href="./CHANGELOG.md">Full release history</a></p>
</div>

<p align="center">
  <a href="./docs/images/web-annotations.jpg"><img src="./docs/images/web-annotations.jpg" width="100%" alt="AI marks webpage controls and connects them to explanations" /></a>
  <br /><sub>Answers connected to the content—not just text in a chat.</sub>
</p>

## New in 0.3.0

Connect **ChatGPT Work / Codex, WorkBuddy, or MiniMax Code desktop** alongside API and Hermes. Save multiple API endpoints, then click the round model button immediately after Upload to switch directly from one list. MewuAI remembers your last selection and preserves each channel's configuration and history scope.

This release also fixes visual instructions appearing in text-only conversations, raw JSON answers, and lingering WorkBuddy thinking. WorkBuddy connection checks now use a bounded protocol handshake without an inference message. Project-owned source is now under MPL-2.0. See [the full changes](./docs/release-notes-v0.3.0.md) and [older versions](./CHANGELOG.md).

## In action

<table>
<tr>
<td width="50%" valign="top">
<h3>Mark what matters</h3>
<p>Ask AI to highlight details and add notes. Keep annotations as you ask follow-up questions.</p>
<a href="./docs/images/ai-checkmarks.jpg"><img src="./docs/images/ai-checkmarks.jpg" width="100%" alt="Mark what matters" /></a>
</td>
<td width="50%" valign="top">
<h3>Translate where you read</h3>
<p>Read and copy translations in place, or extract screenshot text with offline OCR.</p>
<a href="./docs/images/in-place-translation.jpg"><img src="./docs/images/in-place-translation.jpg" width="100%" alt="Translate where you read" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Explain the details</h3>
<p>Connect explanations to code, diagrams, or controls. Reference screenshots and attachments with @.</p>
<a href="./docs/images/code-explanation.jpg"><img src="./docs/images/code-explanation.jpg" width="100%" alt="Explain the details" /></a>
</td>
<td width="50%" valign="top">
<h3>Sketch and annotate</h3>
<p>AI sketches alongside manual pens, shapes, text, numbered markers, highlights, and pixelation.</p>
<a href="./docs/images/ai-drawing.jpg"><img src="./docs/images/ai-drawing.jpg" width="100%" alt="Sketch and annotate" /></a>
</td>
</tr>
</table>

### Video annotations that follow the action

Record a region or attach a video. Jump to an AI-marked moment, or play an interval with tracking annotations.

<p align="center">
  <img src="./docs/media/MewuAI-video-annotations.gif" width="880" alt="Looping demo of video seeking and tracking annotations" />
  <br /><sub>Looping GIF preview</sub>
</p>

*Examples supplied by the author. AI answers, placement, and timing can be wrong; review important results.*

To demonstrate selections and annotations in Tencent Meeting or classroom software, enable **Settings → Capture → Teaching mode**, save, and start a new capture. Share the entire screen. This also makes new pinned images and videos visible; settings remain protected. Use your meeting app to record the lesson; MewuAI region recording and scrolling capture require teaching mode to be off.

## Everyday capture tools

- **Capture:** Multi-monitor selection, window and supported-control snapping, and scrolling capture in either direction.
- **Pin and edit:** Pin, zoom, rotate, move annotations, and undo edits. Save the original or annotated image.
- **Text and tables:** Offline OCR and AI table recognition. Paste tables into Excel or copy them as Markdown or images.
- **Record and inspect:** Record computer audio with an optional microphone. Export MP4 by default, MP3 audio, or GIF animation; press C in the color inspector to copy RGB.

> Capture, pinning, manual annotation, OCR, and recording work without an AI account. Chat, translation, and table recognition require a configured backend with the relevant capabilities.

## AI connections

| Channel | Connection and configuration |
| --- | --- |
| API | Save multiple named endpoints, each with its own model and credentials. OpenAI compatible, MiniMax, MiniMax (CN), and Volcengine are supported. |
| Hermes | Select an existing local Profile and its model/reasoning settings. |
| ChatGPT Work / Codex | Reuse the official local login through `codex app-server`; choose a real model and reasoning option. Work only; account usage rules apply. |
| WorkBuddy | Reuse the desktop client's bundled ACP runtime and login. Read model/reasoning options and check the protocol connection without sending a conversation. |
| MiniMax Code desktop | Reuse the installed desktop session; open the client, refresh status, or test a model without installing a separate CLI. |

Settings tabs organize configuration. All configured, usable channels can appear in the conversation picker; selecting a different tab does not turn the others off. The picker lists saved API endpoint/model combinations, not every model advertised by a provider. Unimplemented OpenClaw / Claude Code tabs have been removed; Doubao Work desktop is not included.

**Verification scope:** MiniMax M3 has been tested through API and Hermes. Codex and WorkBuddy have local text, synthetic-image, and synthetic-video verification records ([Codex](./docs/codex-work-integration.md), [WorkBuddy](./docs/workbuddy-integration.md)); this does not verify every model or PC. Their Agents inspect video using tools already installed on the PC. MiniMax Code uses desktop-session compatibility rather than the same Agent interface, and client updates may affect it.

## Get started

1. **Install or unzip.** Use the installer above, or extract the ZIP and run `MewuAI.exe`. No separate .NET installation needed.
2. **Select an area.** Press <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd> to capture, copy, save, pin, annotate, or extract text.
3. **Connect AI if you want it.** Open **Settings → AI**, configure your channels, and save. In the conversation bar, use the model button after Upload to select one, then ask in text or reference content with `@`.

The app and installer follow your Windows language. **Settings → General** lets you choose English or Simplified Chinese (restart required) and change the shortcut; press Delete in the shortcut field to disable it. Opening Settings automatically checks for updates and prompts when one is available; **Settings → About** also provides a manual check.

**Upgrading from v0.2.5 or earlier:** use the installer link above. Those older updaters require a checksum attachment that new releases no longer provide. v0.2.6 and later verify GitHub's native SHA-256 digest.

<details>
<summary>Requirements and installation notes</summary>

Windows 10 version 2004 (build 19041) or later, x64. Windows N/KN needs the Media Feature Pack for H.264 recording and playback. The installer is not code-signed yet; SmartScreen may show an unknown-publisher warning. Download from this repository's Releases and verify the SHA-256 shown by GitHub for the asset.

</details>

<details>
<summary>AI connections and privacy</summary>

- Image and video support depends on the model. A local Hermes connection may still use cloud services.
- Add API endpoints in the API list. Only OpenAI compatible requires a URL; the other presets use fixed endpoints. Enter credentials and choose a model for each entry. Optional request parameters are under Advanced settings.
- Codex and WorkBuddy use official local protocol interfaces. MiniMax Code's compatibility adapter reads the desktop session locally to authenticate requests. Local clients can still send content to cloud services; model capabilities, login, and quota affect availability.
- Content is sent when you invoke the relevant AI action. Text-only questions do not automatically attach your desktop. Your provider's data policies apply to sent content.
- API keys and sensitive authentication headers are encrypted locally with Windows DPAPI. Without a usable chat backend, AI chat and reference controls stay hidden; offline tools remain available.

</details>

<details>
<summary>Build and test on Windows x64</summary>

Install the .NET 10 SDK, then run from the repository root:

```powershell
dotnet restore .\mewu_ai_Assistant.slnx --locked-mode
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64 --no-restore
dotnet restore .\mewu_ai_Assistant.csproj --locked-mode -r win-x64 -p:Configuration=Release -p:Platform=x64 -p:SelfContained=true
dotnet publish .\mewu_ai_Assistant.csproj -c Release -p:Platform=x64 -r win-x64 --self-contained true --no-restore -o .\artifacts\release\win-x64
```

The [release workflow](./.github/workflows/release.yml) also builds the smoke-test project, audits the publish output, and packages the installer and portable ZIP.

</details>

---

<p align="center">
  Created by <strong>Abner Stephen</strong><br />
  Licensed under <a href="./LICENSE">MPL-2.0</a> · Commercial use permitted under the license<br />
  <a href="./SOURCE.md">License scope and source code availability</a><br />
  <a href="./THIRD-PARTY-NOTICES.md">Third-party notices</a> · <a href="./CODE_OF_CONDUCT.md">Community standards</a> · <a href="./CONTRIBUTING.md">Contributing</a> · <a href="./SECURITY.md">Security</a><br />
  <a href="https://github.com/abnste/mewu_ai/issues">Feedback</a> · <a href="./CHANGELOG.md">Release history</a> · <a href="https://github.com/abnste/mewu_ai/releases">Releases</a>
</p>
