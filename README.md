# 喵呜AI

喵呜AI 是一款 Windows 常驻型屏幕工具。即使没有网络、API Key 或 AI 模型，截图、调整选区、贴图、标注、文字识别、复制、保存和区域录屏仍可独立使用；只有用户明确点击发送或翻译后，内容才会交给所选模型。

## 当前功能

- 单实例托盘应用；默认全局快捷键 `Ctrl + Shift + A`。
- PerMonitorV2 虚拟桌面暗化框选；选区以物理像素统一处理多屏、负坐标和混合 DPI，并支持任意方向拖动、八向缩放、整体移动和方向键微调。
- PNG/JPEG 保存、剪贴板复制、多贴图窗口，以及画笔、高亮、橡皮、矩形、箭头、粗细、撤销、重做和清空。
- 随软件离线交付的 PP-OCRv6 Small 多语言 OCR；初始化或推理异常时才回退 Windows OCR。识别后可跨行选择、复制全部，大图坐标会还原到原选区。
- OCR 坐标驱动的分批原位翻译；支持取消、独立超时和长文档保序合并。
- 多 Provider 管理，支持 OpenAI-compatible 与 MiniMax、自定义 Base URL/Model/Headers、连接测试和默认 Provider；API Key 与敏感自定义 Header 均由 DPAPI 加密。
- 图片/视频/纯文字多轮 AI、兼容接口流式输出、取消请求、重复分析同一区域及结构化屏幕批注。
- Windows 桌面 SAPI 本地语音识别；支持再次点击停止、语言选择和可选自动监听，识别结果不会自动发送。
- 单屏或跨屏区域 MP4 录制（Media Foundation H.264）、暂停/继续、保存时按需 GIF 导出、原位预览和原生视频 AI 理解；录制 UI 排除在捕获之外。
- 隐私安全日志、临时媒体清理和本地设置。

## 环境与运行

- Windows 10 2004（build 19041）或更高版本，x64。
- 开发环境：.NET 10 SDK。
- 录屏使用 Media Foundation；Windows N/KN 版本需要安装 Microsoft Media Feature Pack。

```powershell
& 'C:\Program Files\dotnet\dotnet.exe' build '.\mewu_ai_Assistant.csproj' -p:Platform=x64
& '.\bin\x64\Debug\net10.0-windows10.0.19041.0\MewuAI.exe'
```

测试：

```powershell
& 'C:\Program Files\dotnet\dotnet.exe' test '.\tests\MewuAI.Tests\MewuAI.Tests.csproj' -p:Platform=x64
```

`tests\MewuAI.ProviderSmoke` 可用 `MEWU_SMOKE_VIDEO_PATH` 验证既有 MP4，或用 `MEWU_SMOKE_RECORD_SECONDS`（严格为 2–20 秒）和可选的 `MEWU_SMOKE_RECORD_RECT_JSON` 先录制指定区域；视频验收必须通过 `MEWU_SMOKE_VIDEO_EXPECTED_ANY_JSON` / `MEWU_SMOKE_VIDEO_EXPECTED_ALL_JSON` 提供至少一项非空目标语义，并默认要求首中末帧发生变化（仅静态素材可显式设置 `MEWU_SMOKE_VIDEO_REQUIRE_FRAME_CHANGES=false`）。动态判定会把帧缩成 32 × 18 亮度签名：任意两帧中亮度差至少 12 的网格达到 3% 才通过，同时保存原始像素 SHA-256 证据。该工具必须在主程序关闭时独占运行，会走真实流式视频请求并核对时长与发送前后文件哈希；它仍只验证录制服务与 Provider，发布前还需在 Release 覆盖层完成“区域录屏 → `@视频N` → 发送 → 回答”的产品闭环。

发布自包含版本：

```powershell
& 'C:\Program Files\dotnet\dotnet.exe' publish '.\mewu_ai_Assistant.csproj' -c Release -p:Platform=x64 -r win-x64 --self-contained true -o '.\artifacts\release\win-x64'
```

发布目录中的 `MewuAI.exe` 可直接运行，不要求用户预装 .NET。发布目标会附带 `THIRD-PARTY-NOTICES.md` 与 `Licenses`，清除 PDB、原生开发库和非 Windows 原生资产，并在发现设置、凭据、日志、截图、录屏或 QA 临时文件时终止发布。首次启动默认仅驻留系统托盘；重复启动会激活已有实例。

## 数据与隐私

普通设置位于 `%LOCALAPPDATA%\MewuAI`。API Key 与敏感自定义 Header 经 Windows DPAPI 加密，仅当前 Windows 用户可解密。临时录像和导出中间文件位于 `%LOCALAPPDATA%\MewuAI\Temp`，录制、预览、贴视频、发送或导出期间均由引用租约保护，清理只会删除未使用的旧文件；录制期间不会额外落盘 PNG 帧。复制视频时会先原子暂存到 `%LOCALAPPDATA%\MewuAI\Clipboard`，让退出程序后粘贴仍然有效；暂存副本按用户配置的保留天数清理。日志不会主动记录 API Key、Authorization、图片、视频、Base64 或完整 Prompt。

## 第三方组件

- `ScreenRecorderLib`：MIT，使用 Windows Graphics Capture/Desktop Duplication 与 Microsoft Media Foundation 录制 H.264。
- `RapidOcrNet`、PP-OCRv6 与 ONNX Runtime：Apache-2.0/MIT，提供离线多语言 OCR。
- `SkiaSharp` 与 `Clipper2`：MIT/BSL-1.0，作为 OCR 处理链的传递依赖。
- `System.Speech`：MIT，调用 Windows 桌面 SAPI 完成本地语音识别，不要求 MSIX 包身份。
- `Microsoft.Windows.SDK.NET.Ref`：Windows SDK targeting pack，用于 Windows OCR 等系统 API。
- Windows 自包含 .NET 与 WPF 运行时按 Microsoft .NET Library License 分发；对应许可证和第三方声明随发布目录一并提供。
- xUnit：测试框架。

项目不引入 GPL/AGPL 组件，也不捆绑 FFmpeg。

## 当前能力边界

- MiniMax M3 通过官方 OpenAI-compatible Chat Completions 原生接收图片与视频：图片使用 `image_url`，视频使用 `video_url` 并按 2 FPS 采样；图片单张上限 10 MB、视频单文件上限 50 MB，但 Base64 内联请求体总上限为 64 MiB，因此内联视频应压缩到约 47 MB 或改用 Files API 的 `mm_file://` 引用。所有兼容 Provider 单次最多接收 16 个附件；历史上下文最多保留 20 条消息、24,000 个 UTF-16 字符，并只选取最近的完整问答对。默认 Provider 为 `MiniMax-M3`，不会自动回退到火山方舟。
- 第一版不录制系统音频或麦克风音频；视频编码仍使用 Windows Media Foundation H.264。
