<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="112" alt="喵呜AI 图标" />

  # 喵呜AI · MewuAI

  **把截图、标注、OCR、录屏与 AI 理解留在同一块 Windows 屏幕画布里。**

  **简体中文** · [English](./README.md) · [下载](https://github.com/abnste/mewu_ai/releases/latest) · [反馈问题](https://github.com/abnste/mewu_ai/issues)

  [![Release](https://img.shields.io/badge/版本-v0.0.1-6878F0?style=flat-square)](https://github.com/abnste/mewu_ai/releases/tag/v0.0.1)
  [![Windows](https://img.shields.io/badge/Windows-10%202004%2B-3A8DDE?style=flat-square&logo=windows11&logoColor=white)](#系统要求)
  [![.NET](https://img.shields.io/badge/.NET-10-6C4BC1?style=flat-square&logo=dotnet&logoColor=white)](https://dotnet.microsoft.com/)
  [![WPF](https://img.shields.io/badge/界面-原生%20WPF-E65A8D?style=flat-square)](#技术架构)

  [**下载安装版**](https://github.com/abnste/mewu_ai/releases/download/v0.0.1/MewuAI-Setup-0.0.1-win-x64.exe) · [免安装 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.0.1/MewuAI-Portable-0.0.1-win-x64.zip)
</div>

<br />

<div align="center">
  <img src="./docs/images/mewu-main.png" width="720" alt="喵呜AI 主界面" />
</div>

## 为什么做喵呜AI？

多数截图工具生成图片后就结束了，多数 AI 客户端则会把截图搬进另一个聊天窗口。喵呜AI 把流程留在原地：连续框选多个区域、标注、录制动作、直接提问、查看流式思考，再一键回到 AI 标记的视频时刻，全程不离开冻结桌面的上下文。

> [!IMPORTANT]
> 截图、贴图、标注、复制、保存、离线 OCR 和区域录屏无需 API Key。只有你明确点击 AI 或翻译后，所选内容才会发送给对应模型。

## 核心能力

| | 能力 | 体验 |
|---:|---|---|
| ✂️ | **多区域截图** | 在混合 DPI、多屏和负坐标桌面上连续框选、移动、缩放、编号、引用，并支持撤销/重做。 |
| ✍️ | **原位标注** | 画笔、荧光笔、矩形、椭圆、箭头、文字、本机中文字体、递增序号、任意 RGB；保存时可选带标注或干净原件。 |
| 👁️ | **离线 OCR** | 随软件交付 PP-OCRv6 Small 多语言模型，支持跨行连续选字；仅在初始化或推理异常时降级 Windows OCR。 |
| 🎬 | **区域录屏** | 原位 `3 → 2 → 1` 倒计时、暂停/继续、MP4、按需 GIF、原位预览和视频语义提问。 |
| 🎯 | **视频时间轴标注** | AI 可定位固定帧或播放动作区间并跟踪目标；独立气泡可跳到每一个有效标记。 |
| 🤖 | **多 Provider** | 默认 MiniMax M3，同时支持 OpenAI-compatible、自定义模型/Header、图片/视频、流式正文与思考。 |
| 🐚 | **本机 Hermes** | 自动检测本机 Hermes，并让 Agent/人格、模型、思考程度、持续会话、附件和 TTS 保持同一 Profile。 |
| 🔊 | **语音交互** | Windows 桌面 SAPI 语音输入，以及可选的 Hermes 自动朗读。 |
| 🔐 | **隐私边界** | 产品窗口防捕获、明确发送、DPAPI 凭据、敏感缓冲清零、脱敏日志。 |

## 从截图到回答，始终是一块画布

喵呜AI 不会新开一个割裂的结果窗口。底部对话条只有在内容抵达后才原位展开；截图与录屏对象仍处于最初坐标，AI 标注也会准确映射回产生它的图片或视频。

| 原生设置界面 | 本机 Hermes 接入 |
|:---:|:---:|
| <img src="./docs/images/mewu-settings.png" width="560" alt="喵呜AI 设置" /> | <img src="./docs/images/mewu-hermes.png" width="560" alt="喵呜AI 本机 Hermes" /> |

## 下载与安装

### 安装版（推荐）

下载 [`MewuAI-Setup-0.0.1-win-x64.exe`](https://github.com/abnste/mewu_ai/releases/download/v0.0.1/MewuAI-Setup-0.0.1-win-x64.exe)。安装器按当前用户安装，创建开始菜单快捷方式，可选桌面快捷方式，并提供完整卸载入口。

### 免安装版

下载 [`MewuAI-Portable-0.0.1-win-x64.zip`](https://github.com/abnste/mewu_ai/releases/download/v0.0.1/MewuAI-Portable-0.0.1-win-x64.zip)，解压后运行 `MewuAI.exe`。它是自包含版本，不要求电脑预装 .NET。

首次启动后软件默认驻留系统托盘。按 <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>A</kbd> 唤醒屏幕助手。

### 系统要求

- Windows 10 2004 / build 19041 或更高版本，x64。
- Windows N/KN 版本需要安装 Microsoft Media Feature Pack 才能录制和预览 H.264。
- AI 功能需要兼容 Provider 或本机 Hermes；本地截图能力不需要。

> [!NOTE]
> `0.0.1` 是早期公开版本。安装包暂未进行 Authenticode 代码签名，Windows SmartScreen 可能显示“未知发布者”；请核对 Release 页面公布的 SHA-256。

## 隐私与数据流

```text
冻结桌面 ──► 本地框选 / 标注 / OCR / 录屏
                    │
                    └── 明确发送 / 翻译 ──► 所选 Provider 或本机 Hermes
```

- API Key 与敏感自定义 Header 使用 Windows DPAPI 加密，仅当前 Windows 用户可解密。
- Provider 请求完成、取消或失败后，图片字节缓冲都会被清零。
- 历史上下文有明确上限；未完成回合和迟到的流式增量不会写入历史。
- 日志不会主动记录 API Key、Authorization、Base64 媒体、截图、录屏或完整 Prompt。
- 临时媒体使用引用租约，正在预览、导出、发送或处于剪贴板流程的文件不会被误清理。

## 技术架构

喵呜AI 明确面向 Windows，坚持现代 C#、WPF 与 .NET。所有核心坐标以虚拟桌面物理像素为准并集中换算，Capture、Presentation、AI Provider、本机 Hermes、OCR 与录屏服务彼此隔离。

- **界面：** .NET 10 WPF、PerMonitorV2；合适的窗口使用 DWM 原生圆角与阴影。
- **截图与录屏：** ScreenRecorderLib 调用 Windows Graphics Capture / Desktop Duplication，H.264 使用 Windows Media Foundation。
- **OCR：** RapidOcrNet + 随包 PP-OCRv6 Small ONNX；只在异常时回退 Windows OCR。
- **富文本回答：** Markdig 转原生 FlowDocument，Emoji.Wpf 渲染彩色 emoji。
- **不捆绑 FFmpeg，不引入 GPL/AGPL 依赖。**

## 从源码构建

需要 Windows x64 与 .NET 10 SDK。

```powershell
dotnet build .\mewu_ai_Assistant.csproj -c Release -p:Platform=x64
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64
dotnet publish .\mewu_ai_Assistant.csproj -c Release -p:Platform=x64 -r win-x64 --self-contained true -o .\artifacts\release\win-x64
```

发布目标会拒绝 PDB、非 Windows 原生资产、设置、凭据、日志、截图、录屏和 QA 文件；每份发行包都附带第三方声明、.NET 运行时和模型许可证。

## 项目状态

当前版本优先打磨稳定的 Windows“截图—标注—理解”闭环。欢迎通过 [GitHub Issues](https://github.com/abnste/mewu_ai/issues) 提交可复现问题和产品建议。

---

<div align="center">
  为 Windows 上快速、私密、原位的屏幕理解而生。
</div>
