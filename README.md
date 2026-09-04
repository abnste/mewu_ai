<p align="center">
  <img src="./Assets/MewuAI.Icon.png" width="80" alt="MewuAI" />
</p>

# MewuAI

**Capture your screen. Ask a question. See the answer right on it.**

A Windows screenshot tool with on-screen AI annotations. Highlight details, translate text, explain a diagram, or find a moment in a video—all without leaving the content you're working with.

**English** · [简体中文](./README.zh-CN.md) · [Download installer](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Setup-0.2.2-win-x64.exe) · [Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Portable-0.2.2-win-x64.zip) · [Video demo](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-video-annotations.mp4)

Windows 10 2004+ · x64 · v0.2.2 public beta

![MewuAI marks controls on a webpage and connects them to explanations](./docs/images/web-annotations.jpg)

## Show, don't just tell

### Mark up what you see

Ask AI to circle a detail, add a checkmark or cross, or point something out with a note. Keep asking questions without losing existing annotations. You can also draw by hand, move notes, and save the annotated result.

![AI adds checkmarks and a cross to video thumbnails](./docs/images/ai-checkmarks.jpg)

### Read across languages

Translate text where it appears, then select and copy the translation. Offline OCR makes text in screenshots selectable; AI table recognition turns captured tables into cells you can paste into Excel.

![Translated comments remain in their original positions, with selectable text](./docs/images/in-place-translation.jpg)

### Explain the tricky parts

Connect an explanation to the relevant part of a diagram, code example, or interface. Reference multiple screenshots and uploaded files with `@` to ask about them together.

![Notes explain individual blocks in a Scratch program](./docs/images/code-explanation.jpg)

### Put an idea on the screen

Ask AI to sketch, or use the pen, shapes, arrows, text, and numbered markers yourself. Highlight important details or pixelate a rectangular area before sharing.

![A cat drawn by AI directly inside the selected area](./docs/images/ai-drawing.jpg)

### Find the moment in a video

Record a region or attach a video, then ask about what happens. Click a timestamp to jump to a marked moment, or play an annotated interval with tracking. Export recordings as MP4 or GIF.

[Watch the video annotation demo →](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-video-annotations.mp4)

*Examples supplied by the author. AI answers, placement, and timing can be wrong; review the result before sharing or relying on it.*

## A screenshot tool, even without AI

- **Capture:** select across monitors, snap to windows and supported controls, or stitch a scrolling capture in either direction.
- **Pin:** keep an image on top, zoom or rotate it, and preserve annotations when pinning.
- **Edit:** draw, highlight, add text or mosaic, undo changes, and choose between the original and annotated image when saving.
- **Extract:** select text with offline OCR; inspect the color and screen coordinates under your pointer.

Capture, pinning, manual annotation, OCR, and recording work without an AI account. AI chat, translation, and table recognition require a configured backend with the relevant capabilities.

## Get started

1. Download the [installer](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Setup-0.2.2-win-x64.exe), or extract the [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Portable-0.2.2-win-x64.zip) and run `MewuAI.exe`. Neither needs a separate .NET installation.
2. Press <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd> and select an area. Use its toolbar to copy, save, pin, annotate, or extract text.
3. For AI features, configure a compatible API in Settings or connect your local Hermes installation. Add references with `@`, type your request, and send.

The app and installer follow your Windows language. You can choose English or Simplified Chinese in **Settings → General**; app language changes take effect after a restart. The capture shortcut is customizable there too. Check for new versions in **Settings → About**.

### AI connections and privacy

- Bring your own compatible API, or use an existing local Hermes profile. Image and video support depends on the selected model; a local Hermes connection may still use cloud services.
- Screenshots and attachments are sent to the selected backend only when you invoke the relevant AI action. A text-only question does **not** attach your desktop automatically.
- API keys and sensitive authentication headers are encrypted locally with Windows DPAPI. Your provider's own data policies apply to content you send.
- AI chat and reference controls stay hidden when no usable chat backend is configured. Offline tools remain available.

Tested model for this public beta: **MiniMax M3**. Other model and provider combinations may behave differently.

### Before installing

Requires **Windows 10 version 2004 (build 19041) or later, x64**. Windows N/KN editions need the Media Feature Pack for H.264 recording and playback. The installer is not code-signed yet, so SmartScreen may show an unknown-publisher warning. Download from this repository's [Releases](https://github.com/abnste/mewu_ai/releases) and verify the included `SHA256SUMS.txt`.

## Development

Built with **C# · .NET 10 · WPF**. OCR runs locally; recording uses Windows media components.

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

## Author and use

Created by **Abner Stephen**. Source code is available in this repository; **commercial use requires the author's authorization**. Third-party components retain their own licenses—see [third-party notices](./THIRD-PARTY-NOTICES.md).

[Report a bug or suggest a feature](https://github.com/abnste/mewu_ai/issues) · [Release notes](https://github.com/abnste/mewu_ai/releases/tag/v0.2.2)
