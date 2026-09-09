<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI" />
  <h1>MewuAI</h1>
  <p>A Windows screenshot tool with screen recording, pinned images, text recognition, and AI annotations.</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.4.0"><img src="https://img.shields.io/badge/Public_Beta-v0.4.0-7C6CF0?style=flat-square" alt="v0.4.0 public beta" /></a>
    <img src="https://img.shields.io/badge/Windows-10_2004%2B-0078D4?style=flat-square" alt="Windows 10 2004 or later, x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="MPL-2.0" /></a>
  </p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.4.0/MewuAI-Setup-0.4.0-win-x64.exe"><strong>Download installer</strong></a>
    &nbsp;·&nbsp;
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.4.0/MewuAI-Portable-0.4.0-win-x64.zip">Portable ZIP</a>
  </p>
  <p><a href="./README.zh-CN.md">简体中文</a> · <strong>English</strong> · <a href="./CHANGELOG.md">Changelog</a> · <a href="https://github.com/abnste/mewu_ai/issues">Report an issue</a></p>
</div>

Select something on your screen and ask AI to translate it, explain it, or highlight the important parts. Answers and annotations appear alongside the content, and you can save them with your screenshot.

<p align="center">
  <a href="./docs/images/web-annotations.jpg"><img src="./docs/images/web-annotations.jpg" width="100%" alt="AI circles buttons in a webpage screenshot and adds explanations beside them" /></a>
</p>

## Features

- **Papers and assignments:** Import selected PDF pages or collect screenshots across capture sessions. Review per-page grading, compare shared errors across submissions, and edit/export separate practice questions and answers. Teacher confirmation remains required. [Workflow guide](./docs/teaching-workflow.md).

- **Screenshots and scrolling capture:** Select a region or window, capture across monitors, and scroll up or down to capture long pages.
- **Annotations and pinned images:** Add pen strokes, highlights, arrows, shapes, text, numbered markers, and pixelation. Drag annotations to adjust them, or pin a screenshot on your desktop for reference.
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
<h3>Translate screenshots</h3>
<p>Read translations where the original text appears, then select text to copy it.</p>
<a href="./docs/images/in-place-translation.jpg"><img src="./docs/images/in-place-translation.jpg" width="100%" alt="Translated text appears in the original positions within a screenshot" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>Understand code</h3>
<p>Select a piece of code and ask for an explanation tied to the lines you are reading.</p>
<a href="./docs/images/code-explanation.jpg"><img src="./docs/images/code-explanation.jpg" width="100%" alt="Explanations beside a code screenshot point to the relevant lines" /></a>
</td>
<td width="50%" valign="top">
<h3>Draw and annotate</h3>
<p>Ask AI to add a diagram, or add your own notes with pens, shapes, and text.</p>
<a href="./docs/images/ai-drawing.jpg"><img src="./docs/images/ai-drawing.jpg" width="100%" alt="AI drawings and annotations on a screenshot" /></a>
</td>
</tr>
</table>

### Paper grading

See correct, incorrect and uncertain answers marked on the original page. Review the reading, expected answer and verdict, or adjust an answer box. Collect several submissions to build practice from their reviewed shared errors. [Explore the teaching workflow](./docs/teaching-workflow.md).

<p align="center">
  <a href="./docs/images/teaching-review-en.png"><img src="./docs/images/teaching-review-en.png" width="100%" alt="Official 2025 TSA exam excerpt with recorded MiniMax annotations and the question-by-question review panel" /></a>
</p>

*Source: [HKEAA 2025 TSA Secondary 3 Mathematics 9MC2, page 11](https://www.bca.hkeaa.edu.hk/web/Common/res/2025secPaper/S3Math/TSA2025_9MC2_Q.pdf#page=11). The original exam layout is preserved. Blue answers are simulated; annotations and verdicts are from the recorded MiniMax evaluation and still require teacher review. Exam copyright belongs to its original owner.*

### Video analysis

Record a short demonstration or attach an existing video, then ask a question. Time buttons in the answer take you to the relevant scene. Annotations can follow the subject while a marked segment plays.

<p align="center">
  <img src="./docs/media/MewuAI-video-annotations.gif" width="880" alt="Jumping to a scene from an answer and playing a video with tracking annotations" />
</p>

## Get started

1. **Install and open.** Download the installer above, or extract the portable ZIP and run MewuAI.exe. Requires Windows 10 2004 or later, x64. No separate .NET installation is needed.
2. **Capture a region.** Press <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd>, drag to select an area, and use the toolbar to copy, save, annotate, extract text, or record.
3. **Connect AI.** Set up and save a connection in **Settings → AI**. Use the capture toolbar's reference button to add a region to your question. You can also upload attachments or ask a text-only question.

**Settings → General** lets you change the capture shortcut and switch between English and Simplified Chinese. Press Delete in the shortcut field to disable it. Restart the app after changing its language.

## AI connections

| Connection | What you need |
| --- | --- |
| API | Your provider's API key and a model. Supports OpenAI-compatible services, MiniMax, and Volcengine, with multiple saved endpoints. |
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

Created by **Abner Stephen** & **Yandi**.

Project-owned source is licensed under [MPL-2.0](./LICENSE). Commercial use is permitted under the license. When distributing covered software, provide the covered source and retain copyright and license notices as required. See [license and source information](./SOURCE.md) and the separate [third-party notices](./THIRD-PARTY-NOTICES.md).
