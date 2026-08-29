# 喵呜AI

喵呜AI 是一款 Windows 常驻型屏幕工具。即使没有网络、API Key 或 AI 模型，截图、调整选区、贴图、标注、文字识别、复制、保存和区域录屏仍可独立使用；只有用户明确点击发送或翻译后，内容才会交给所选模型。

## 当前功能

- 单实例托盘应用；默认全局快捷键 `Ctrl + Shift + A`。
- 虚拟桌面暗化框选，支持任意方向拖动、八向缩放、整体移动和方向键微调。
- PNG/JPEG 保存、剪贴板复制、多贴图窗口及自由画笔标注（颜色、粗细、撤销、重做、清空）。
- Windows 本地 OCR，可按行选择和复制识别文字。
- OCR 坐标驱动的原位翻译，可切换原文/译文。
- OpenAI-compatible 与 MiniMax Provider，DPAPI 加密 API Key，图片能力检查、取消请求和结构化屏幕批注。
- Windows 原生语音识别输入。
- 区域 MP4 录制（Media Foundation H.264）及 GIF 导出；录制 UI 排除在捕获之外。
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

## 数据与隐私

普通设置位于 `%LOCALAPPDATA%\MewuAI`。API Key 经 Windows DPAPI 加密，仅当前 Windows 用户可解密。临时录像与 GIF 帧位于 `%LOCALAPPDATA%\MewuAI\Temp`，旧文件会自动清理。日志不会主动记录 API Key、Authorization、图片、视频、Base64 或完整 Prompt。

## 第三方组件

- `ScreenRecorderLib`：MIT，使用 Windows Graphics Capture/Desktop Duplication 与 Microsoft Media Foundation 录制 H.264。
- `Microsoft.Windows.SDK.NET.Ref`：Windows SDK targeting pack，用于 Windows OCR 与语音 API。
- xUnit：测试框架。

项目不引入 GPL/AGPL 组件，也不捆绑 FFmpeg。
