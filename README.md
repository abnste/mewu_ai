<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI" />
  <h1>MewuAI · 喵呜AI</h1>
  <p><strong>Capture your screen. Let AI annotate, translate, and explain it in place.</strong></p>
  <p>A Windows screenshot tool with on-screen AI assistance</p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong> · <a href="#in-action">In action</a> · <a href="https://github.com/abnste/mewu_ai/issues">Feedback</a></p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.2.3"><img src="https://img.shields.io/badge/Public_Beta-v0.2.3-7C6CF0?style=flat-square" alt="v0.2.3 Public beta" /></a>
    <img src="https://img.shields.io/badge/Windows-10_2004%2B-0078D4?style=flat-square" alt="Windows 10 2004 or later" />
    <img src="https://img.shields.io/badge/Architecture-x64-64748B?style=flat-square" alt="x64" />
  </p>
  <p><a href="https://github.com/abnste/mewu_ai/releases/download/v0.2.3/MewuAI-Setup-0.2.3-win-x64.exe"><strong>Download installer</strong></a> &nbsp; · &nbsp; <a href="https://github.com/abnste/mewu_ai/releases/download/v0.2.3/MewuAI-Portable-0.2.3-win-x64.zip">Portable ZIP</a></p>
</div>

<p align="center">
  <a href="./docs/images/web-annotations.jpg"><img src="./docs/images/web-annotations.jpg" width="100%" alt="AI marks webpage controls and connects them to explanations" /></a>
  <br /><sub>Answers connected to the content—not just text in a chat.</sub>
</p>

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
  <br /><sub>Looping preview · <a href="https://github.com/abnste/mewu_ai/releases/download/v0.2.3/MewuAI-video-annotations.mp4">Watch with audio (MP4)</a></sub>
</p>

*Examples supplied by the author. AI answers, placement, and timing can be wrong; review important results.*

## Everyday capture tools

- **Capture:** Multi-monitor selection, window and supported-control snapping, and scrolling capture in either direction.
- **Pin and edit:** Pin, zoom, rotate, move annotations, and undo edits. Save the original or annotated image.
- **Text and tables:** Offline OCR and AI table recognition. Paste tables into Excel or copy them as Markdown or images.
- **Record and inspect:** Region recording, MP4/GIF export, and color values and coordinates under the pointer.

> Capture, pinning, manual annotation, OCR, and recording work without an AI account. Chat, translation, and table recognition require a configured backend with the relevant capabilities.

**Tested model:** MiniMax M3 has been tested through both Hermes and direct API connections. Other models have not been verified; please test their compatibility yourself.

## Get started

1. **Install or unzip.** Use the installer above, or extract the ZIP and run `MewuAI.exe`. No separate .NET installation needed.
2. **Select an area.** Press <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd> to capture, copy, save, pin, annotate, or extract text.
3. **Connect AI if you want it.** Configure a compatible API or connect local Hermes in Settings, then reference content with `@` and ask.

The app and installer follow your Windows language. **Settings → General** lets you choose English or Simplified Chinese (restart required) and change the shortcut. Check for updates in **Settings → About**.

<details>
<summary>Requirements and installation notes</summary>

Windows 10 version 2004 (build 19041) or later, x64. Windows N/KN needs the Media Feature Pack for H.264 recording and playback. The installer is not code-signed yet; SmartScreen may show an unknown-publisher warning. Download from this repository's Releases and verify the included `SHA256SUMS.txt`.

</details>

<details>
<summary>AI connections and privacy</summary>

- Image and video support depends on the model. A local Hermes connection may still use cloud services.
- Choose OpenAI compatible, MiniMax, MiniMax (CN), or Volcengine. Only OpenAI compatible requires a URL; the others use fixed endpoints. Enter your API key, then choose a model. Optional request parameters are under Advanced settings.
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
  Source available · Commercial use requires the author's authorization<br />
  <a href="./THIRD-PARTY-NOTICES.md">Third-party notices</a> · <a href="https://github.com/abnste/mewu_ai/issues">Feedback</a> · <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.2.3">Release notes</a>
</p>
