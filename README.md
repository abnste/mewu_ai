<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="104" alt="MewuAI icon" />

  # MewuAI · 喵呜AI

  **Capture, annotate, and understand content directly on your Windows screen.**

  [简体中文](./README.zh-CN.md) · **English** · [Download](https://github.com/abnste/mewu_ai/releases/latest) · [Report a bug](https://github.com/abnste/mewu_ai/issues)

  [![Release](https://img.shields.io/badge/release-v0.0.8-6878F0?style=flat-square)](https://github.com/abnste/mewu_ai/releases/tag/v0.0.8)
  [![Windows](https://img.shields.io/badge/Windows-10%202004%2B-3A8DDE?style=flat-square&logo=windows11&logoColor=white)](#system-requirements)
  [![.NET](https://img.shields.io/badge/.NET-10-6C4BC1?style=flat-square&logo=dotnet&logoColor=white)](https://dotnet.microsoft.com/)

  [**Download installer**](https://github.com/abnste/mewu_ai/releases/download/v0.0.8/MewuAI-Setup-0.0.8-win-x64.exe) · [Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.0.8/MewuAI-Portable-0.0.8-win-x64.zip)
</div>

<br />

<div align="center">
  <img src="./docs/images/mewu-capture-annotation.png" width="900" alt="MewuAI in-place capture and AI annotation" />
</div>

## Highlights

- **Capture and pin:** precise multi-monitor capture, smart snapping, scrolling capture, live color/coordinates, and rotatable pinned images.
- **Complete annotation:** pen, highlighter, shapes, text, numbered markers, draggable AI callouts, and rectangle mosaic—all included in undo, copy, and export.
- **OCR and translation:** offline multilingual OCR, cross-line selection, and in-place translation that can be copied, pinned, or exported with annotations.
- **Image and video AI:** exact `@RegionN`, `@ImageN`, and `@VideoN` references with multi-attachment annotations mapped back to the correct source.
- **Recording and timeline:** in-place MP4 recording, on-demand GIF export, and seekable or tracked AI video moments.
- **Connect only what you use:** compatible remote multimodal providers and local Hermes; unavailable AI entry points stay hidden until a working backend is configured.

<div align="center">
  <img src="./docs/images/mewu-features-bilingual.png" width="960" alt="MewuAI bilingual feature overview" />
</div>

> [!IMPORTANT]
> Capture, pinning, annotation, OCR, and recording work offline. Selected content is sent only when you explicitly invoke AI or translation.

## Download

- [Installer 0.0.7](https://github.com/abnste/mewu_ai/releases/download/v0.0.7/MewuAI-Setup-0.0.7-win-x64.exe): per-user setup with Start Menu entry, optional desktop shortcut, and uninstaller.
- [Portable 0.0.7](https://github.com/abnste/mewu_ai/releases/download/v0.0.7/MewuAI-Portable-0.0.7-win-x64.zip): extract and run `MewuAI.exe`; no separate .NET installation is required.

Press <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>A</kbd> to open the screen assistant.

### System requirements

- Windows 10 version 2004 (build 19041) or newer, x64.
- Windows N/KN editions need the Microsoft Media Feature Pack for H.264 recording and preview.
- AI and translation require a working compatible provider. Hermes features require a local enabled Hermes installation.

> [!NOTE]
> The installer is not Authenticode-signed yet, so Windows SmartScreen may show an unknown-publisher notice. Verify the SHA-256 file attached to the release.

## Privacy

- Product windows are capture-protected, and screen content is never uploaded before explicit send.
- API keys and sensitive headers are encrypted with Windows DPAPI.
- Sensitive image buffers are scrubbed after requests; history is bounded and logs exclude screen content and credentials.
- Capture, OCR, annotation, and recording never depend on AI.

## Build from source

Requires Windows x64 and the .NET 10 SDK.

```powershell
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64
dotnet publish .\mewu_ai_Assistant.csproj -c Release -p:Platform=x64 -r win-x64 --self-contained true -o .\artifacts\release\win-x64
```

Stack: C#, .NET 10, native WPF, Windows Media Foundation, ScreenRecorderLib, and RapidOcrNet. No bundled FFmpeg and no GPL/AGPL dependencies.

---

<div align="center">Built for fast, private, in-place screen understanding.</div>
