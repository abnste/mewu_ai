<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI" />
  <h1>MewuAI — AI Screenshot Annotation for Windows</h1>
  <p>Open-source screen capture with in-place AI annotations, offline OCR, screenshot translation, and screen recording.</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.5.9"><img src="https://img.shields.io/badge/Public_Beta-v0.5.9-7C6CF0?style=flat-square" alt="v0.5.9 public beta" /></a>
    <img src="https://img.shields.io/badge/Windows-10_2004%2B-0078D4?style=flat-square" alt="Windows 10 2004 or later, x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="MPL-2.0" /></a>
  </p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Setup-0.5.9-win-x64.exe"><strong>Download installer</strong></a>
    &nbsp;·&nbsp;
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.5.9/MewuAI-Portable-0.5.9-win-x64.zip">Portable ZIP</a>
  </p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong> · <a href="./CHANGELOG.md">Changelog</a> · <a href="https://github.com/abnste/mewu_ai/issues">Report an issue</a></p>
</div>

**MewuAI (喵呜AI)** is an open-source **AI screenshot annotation tool for Windows**. Select a screen region, reference it in your question, and ask a vision-capable AI model to explain the content or mark important details. Answers, arrows, highlights, and other annotations appear at the original screen positions; you can save the annotated screenshot.

**喵呜AI（MewuAI）是一款 Windows 开源 AI 截图标注软件**，支持 AI 圈选重点、截图问答、离线 OCR、原位翻译、长截图和录屏。中文介绍见[中文首页](./README.zh-CN.md)和[AI 截图标注使用指南](./docs/ai-screenshot-annotation.zh-CN.md)。

| At a glance | Details |
| --- | --- |
| Platform | Windows 10 2004 or later, x64; English and Simplified Chinese |
| Core workflow | Capture → reference a region → ask AI → review and save annotations |
| Works offline | Screenshots, manual markup, pinned images, OCR, and recording |
| AI requirements | A connected account or API; image/video support and charges depend on the selected service |
| Source and downloads | [Official repository](https://github.com/abnste/mewu_ai) · [Latest release](https://github.com/abnste/mewu_ai/releases/latest) |
| License | [MPL-2.0](./LICENSE); third-party terms are listed separately |

Read the [AI screenshot annotation guide](./docs/ai-screenshot-annotation.md) for examples, setup, privacy, and the difference between manual markup, OCR, and AI annotations.

<p align="center">
  <a href="./docs/images/mewuai-cover-web-en.png"><img src="./docs/images/mewuai-cover-web-en.png" width="100%" alt="MewuAI concept cover: capture, ask and annotate on a Windows desktop" /></a>
  <br /><sub>AI-generated concept cover. Actual interface examples are shown below.</sub>
</p>

## Features

- **Papers and assignments:** Reference exam screenshots in the ordinary conversation bar for explanations and annotations. Reference multiple screenshots to discuss shared problems and generate practice questions; readings and grading still need human review.

- **Screenshots and scrolling capture:** Select a region or window, capture across monitors, and capture long pages without a fixed segment limit. Window-snapped selections offer app snapshots that capture supported scroll areas, restore their position, then pin and reference the result. Availability depends on the application's rendering and scrolling support.
- **Annotations and pinned images:** Add pen strokes, highlights, lines, arrows, shapes, text, numbered markers, and pixelation. Edit selected objects, choose colors from a full-color palette, and preview pixelation while dragging. Hold Shift to constrain lines, arrows and shapes; click Finish to copy the annotated image automatically.
- **Text and tables:** Copy text from images, translate screenshots, and use AI to extract tables for Excel.
- **Screen recording:** Record a region with computer audio and an optional microphone. Export MP4 video, MP3 audio, or a GIF.
- **Ask about images and videos:** Reference several screenshots or attachments, ask follow-up questions, and click a time in an answer to jump to the relevant video scene.
- **Choose your AI:** Connect API services, Hermes, ChatGPT Work / Codex, WorkBuddy, or MiniMax Code. Switch between them and keep your last selection.

Screenshots, manual annotations, pinned images, text recognition, and recording work without an AI account. Translation, table extraction, and AI questions require a connected service.

## Preview

<table>
<tr>
<td width="50%" valign="top">
<h3>Highlight details</h3>
<p>Ask AI to circle items, add checkmarks, or leave notes on a screenshot. Continue with follow-up questions.</p>
<a href="./docs/images/ai-checkmarks.jpg"><img src="./docs/images/ai-checkmarks.jpg" width="100%" alt="AI highlights details and adds checkmarks to a screenshot" /></a>
</td>
<td width="50%" valign="top">
<h3>Recognize tables</h3>
<p>Turn a table screenshot into a structured answer, then use Copy table to paste it into Excel.</p>
<a href="./docs/images/table-recognition.png"><img src="./docs/images/table-recognition.png" width="100%" alt="A table screenshot recognized as rows and columns with a Copy table button" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Analyze videos</h3>
<p>Click a time in an answer to jump to a scene. Annotations follow the subject while a marked segment plays.</p>
<a href="./docs/media/MewuAI-video-annotations.gif"><img src="./docs/media/MewuAI-video-annotations.gif" width="100%" alt="Jumping to a video scene from an answer and playing it with tracking annotations" /></a>
</td>
<td width="50%" valign="top">
<h3>Annotate data</h3>
<p>Ask AI to identify and mark objects in an image, such as visible vehicles in a traffic scene.</p>
<a href="./docs/images/data-annotations.png"><img src="./docs/images/data-annotations.png" width="100%" alt="AI circles visible vehicles in a traffic image and explains the limits of recognition" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Translate screenshots</h3>
<p>Read translations where the original text appears, then select text to copy it.</p>
<a href="./docs/images/in-place-translation.jpg"><img src="./docs/images/in-place-translation.jpg" width="100%" alt="Translated text appears in the original positions within a screenshot" /></a>
</td>
<td width="50%" valign="top">
<h3>Draw and annotate</h3>
<p>Ask AI to add a diagram, or add your own notes with pens, shapes, and text.</p>
<a href="./docs/images/draw-and-annotate-latest.jpg"><img src="./docs/images/draw-and-annotate-latest.jpg" width="100%" alt="AI drawings and annotations on a screenshot" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Understand code</h3>
<p>Select a piece of code and ask for an explanation tied to the lines you are reading.</p>
<a href="./docs/images/understand-code-20260921.png"><img src="./docs/images/understand-code-20260921.png" width="100%" alt="Explanations beside a code screenshot point to the relevant lines" /></a>
</td>
<td width="50%" valign="top">
<h3>Annotate papers</h3>
<p>Reference exam screenshots for explanations and annotations, or compare several pages to discuss shared errors and create practice.</p>
<a href="./docs/images/exam-annotation-latest.jpg"><img src="./docs/images/exam-annotation-latest.jpg" width="100%" alt="Geometry explanations and in-place annotations on an exam screenshot" /></a>
</td>
</tr>
</table>

<details>
<summary>About the paper demonstration</summary>

The example shows explanations and in-place annotations for a geometry question. Check AI answers and annotations against the original question.

</details>

## Get started

> **Mac users:** MewuAI does not currently support macOS. Explore our partner project [**kangarooking/Ta**](https://github.com/kangarooking/Ta), an AI screenshot tool for macOS with OCR, translation, scrolling capture, and annotations.

1. **Install and open.** Download the installer above, or extract the portable ZIP and run MewuAI.exe. Requires Windows 10 2004 or later, x64. No separate .NET installation is needed.
2. **Capture a region.** Press <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd>, drag to select an area, and use the toolbar to copy, save, annotate, extract text, or record.
3. **Connect AI.** Set up and save a connection in **Settings → AI**. Use the capture toolbar's reference button to add a region to your question. You can also upload attachments or ask a text-only question.

While capturing, right-click or hold and drag on the canvas to click or drag in the application underneath. Hold <kbd>Ctrl</kbd> while right-clicking to pass through a real right-click. Existing captures remain in place when you release.

**Settings → General** lets you change the capture shortcut and switch between English and Simplified Chinese. Press Delete in the shortcut field to disable it. Restart the app after changing its language.

You can maximize Settings with its maximize button or by double-clicking the title bar; repeat to restore the compact window.

## AI connections

| Connection | What you need |
| --- | --- |
| API | Your provider's API key and a model. Search built-in China/global service templates, load their model catalogs, and save multiple independent connections. [Supported services and setup](docs/api-providers.md). |
| Hermes | A configured Hermes installation on your PC. Choose a profile and model in settings. |
| ChatGPT Work / Codex | A signed-in ChatGPT Work / Codex installation on your PC. Choose a model and reasoning level in settings. |
| WorkBuddy | Install and sign in to WorkBuddy desktop, then connect and choose a model in settings. |
| MiniMax Code | Sign in to MiniMax Code desktop. You can open it from settings; no separate command-line installation is required. |

With multiple connections saved, click the **model button after Upload** in the conversation bar to choose one. Your configurations are kept, and the app remembers your last selection.

Image and video support depends on the selected model. MiniMax M3 is available through API or Hermes. Codex and WorkBuddy may need video-processing tools already installed on your PC to inspect videos. Check AI answers and annotations against the original content.

## FAQ

<details>
<summary>Does it cost anything?</summary>

Screenshots, annotations, pinned images, text recognition, and recording are available without an AI account. AI features use the account or API you connect; your provider determines charges and usage limits.

</details>

<details>
<summary>Why are my selections and annotations missing from a screen share?</summary>

Teaching mode is on by default. Share your entire screen in your meeting or classroom app. You can turn it off or back on under **Settings → Capture → Teaching mode**; save and start a new capture to apply the change.

This makes selections, annotations, and newly pinned images and videos visible to viewers. You can also use MewuAI's recording and scrolling capture while teaching mode is on. Controls stay outside the capture area; when there is no room, such as a full-screen capture, they are hidden. Press **F8** to stop recording or finish scrolling capture. During the recording countdown, F8 cancels it.

</details>

<details>
<summary>Are screenshots and conversations uploaded automatically?</summary>

Screenshots, manual annotations, and text recognition are processed on your PC. When you use AI analysis, translation, or another AI feature, the relevant content is sent to your chosen service. Text-only questions do not automatically include your desktop.

API keys are stored encrypted on your PC. A connected desktop AI app may also use cloud services.

</details>

<details>
<summary>How do I update? What if an older version cannot update?</summary>

Opening Settings checks for updates automatically. You can also check manually in **Settings → About**.

If you use v0.2.5 or earlier, download the installer from this page to upgrade manually. See the [changelog](./CHANGELOG.md) for previous versions and their release notes.

The release page shows a SHA-256 digest in each asset's details for verifying the installer and portable ZIP. A separate checksum file is not provided.

</details>

<details>
<summary>Having trouble installing or recording?</summary>

The installer is not code-signed yet, so Windows may show an unknown-publisher prompt. Download from this repository's [Releases](https://github.com/abnste/mewu_ai/releases); the asset details include its SHA-256 checksum.

Windows N / KN editions need the Media Feature Pack to record and play video. For other problems, [open an issue](https://github.com/abnste/mewu_ai/issues) with your app version, Windows version, and steps to reproduce it.

</details>

## Contributing

Bug reports, suggestions, code, and documentation improvements are welcome. See the [contributing guide](./CONTRIBUTING.md) for development setup and build instructions, the [code of conduct](./CODE_OF_CONDUCT.md) for community guidelines, and the [security policy](./SECURITY.md) for vulnerability reports.

## License

Created by [**Abner Stephen**](https://github.com/abnste), [**Yandi**](https://github.com/pyxyd), and [**shuziyuxingxing-stack**](https://github.com/shuziyuxingxing-stack).

Project-owned source is licensed under [MPL-2.0](./LICENSE). Commercial use is permitted under the license. When distributing covered software, provide the covered source and retain copyright and license notices as required. See [license and source information](./SOURCE.md) and the separate [third-party notices](./THIRD-PARTY-NOTICES.md).

## Acknowledgments

Thanks to teachers **Lin Wenjie** for subject-matter guidance and **Zhang Wenbiao** for feature development suggestions, and to [**blueling0**](https://github.com/blueling0) for testing assistance.
