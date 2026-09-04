<p align="center">
  <img src="./Assets/MewuAI.Icon.png" width="80" alt="喵呜AI" />
</p>

# 喵呜AI

**截下画面，问出问题，让答案直接标在眼前。**

一款带 AI 原位标注的 Windows 截图工具。圈重点、翻译文字、讲解图示、定位视频片段，都在当前画面上完成。

**简体中文** · [English](./README.md) · [下载安装版](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Setup-0.2.2-win-x64.exe) · [免安装 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Portable-0.2.2-win-x64.zip) · [视频演示](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-video-annotations.mp4)

Windows 10 2004+ · x64 · v0.2.2 公测版

![AI 在网页上圈出按钮，并连接对应说明](./docs/images/web-annotations.jpg)

## 不只回答，也直接标出来

### 重点在哪，一眼看到

让 AI 圈出细节、打勾打叉，或连上一条说明。继续追问时保留已有标注，也可以自己动手补画、移动气泡，最后保存带标注的图片。

![AI 在视频封面上打勾与打叉](./docs/images/ai-checkmarks.jpg)

### 看懂外语，取出文字

译文显示在原文位置，支持选中复制。离线 OCR 让截图里的文字可以直接选择；AI 表格识别则把截图中的表格变成可粘贴到 Excel 的单元格。

![评论原位翻译，译文支持选择与复制](./docs/images/in-place-translation.jpg)

### 对着画面讲清楚

图示、代码、软件界面，让说明连到对应位置。用 `@` 引用多张截图或上传的文件，一起提问、对照理解。

![AI 为 Scratch 程序的不同积木添加解释](./docs/images/code-explanation.jpg)

### 让想法落到画面上

让 AI 画一幅草图，或自己使用画笔、形状、箭头、文字和序号。分享前，也可以高亮重点或给矩形区域打马赛克。

![AI 在选区中绘制小猫](./docs/images/ai-drawing.jpg)

### 视频里的那一刻，直接跳过去

录制一个区域，或添加视频后提问。点击时间入口跳到标记位置，也可以播放指定区间并跟踪标注。录屏支持导出 MP4 和 GIF。

[观看视频标注演示 →](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-video-annotations.mp4)

*以上实景由作者提供。AI 的回答、标注位置与时间可能有误，分享或使用结果前请核对。*

## 不接 AI，也是一款截图工具

- **截图：** 多屏框选、窗口与受支持控件吸附，以及上下双向滚动长截图。
- **贴图：** 图片置顶、缩放、旋转，带着已有标注一起贴在屏幕上。
- **编辑：** 绘制、高亮、文字、马赛克与撤销；保存时可选择原件或带标注图片。
- **提取：** 离线 OCR 选字复制，查看鼠标位置的颜色值和屏幕坐标。

截图、贴图、手工标注、OCR 和录屏无需 AI 账号。AI 对话、翻译和表格识别需要配置具备相应能力的后端。

## 开始使用

1. 下载[安装版](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Setup-0.2.2-win-x64.exe)，或解压[免安装 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.2.2/MewuAI-Portable-0.2.2-win-x64.zip) 后运行 `MewuAI.exe`，均无需另装 .NET。
2. 按 <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd>，框选所需内容，通过选区工具条复制、保存、置顶、标注或识别文字。
3. 如需 AI，在设置中配置兼容 API，或连接本机 Hermes。用 `@` 添加引用，输入问题后发送。

应用和安装程序默认跟随 Windows 语言。可在 **设置 → 常规** 中选择简体中文或 English，重启应用后生效；截图快捷键也可在此修改。新版本可通过 **设置 → 关于** 检查更新。

### AI 接入与隐私

- 支持自行配置兼容 API，或使用本机已有的 Hermes Profile。图片、视频能力取决于所选模型；连接本机 Hermes 不代表它使用的模型服务也在本地运行。
- 只有主动调用相应 AI 功能时，截图和附件才会发送到所选后端。只发文字问题，**不会自动附带桌面截图**。
- API Key 和敏感认证 Header 使用 Windows DPAPI 在本机加密；已发送内容的处理方式由相应服务商的数据政策决定。
- 未配置可用对话后端时，隐藏 AI 对话条和引用入口，离线工具照常使用。

本次公测使用 **MiniMax M3** 测试，其他模型与服务组合的效果可能不同。

### 安装须知

支持 **Windows 10 2004（build 19041）或更高版本，x64**。Windows N/KN 版本录制和播放 H.264 需要 Media Feature Pack。安装包暂未代码签名，SmartScreen 可能提示“未知发布者”；请从本仓库的 [Releases](https://github.com/abnste/mewu_ai/releases) 下载，并核对附带的 `SHA256SUMS.txt`。

## 开发

基于 **C# · .NET 10 · WPF**，OCR 在本机运行，录屏使用 Windows 媒体组件。

<details>
<summary>在 Windows x64 上构建与测试</summary>

安装 .NET 10 SDK，在仓库根目录运行：

```powershell
dotnet restore .\mewu_ai_Assistant.slnx --locked-mode
dotnet test .\tests\MewuAI.Tests\MewuAI.Tests.csproj -c Release -p:Platform=x64 --no-restore
dotnet restore .\mewu_ai_Assistant.csproj --locked-mode -r win-x64 -p:Configuration=Release -p:Platform=x64 -p:SelfContained=true
dotnet publish .\mewu_ai_Assistant.csproj -c Release -p:Platform=x64 -r win-x64 --self-contained true --no-restore -o .\artifacts\release\win-x64
```

[发布流程](./.github/workflows/release.yml) 还会编译连通性测试项目、审计发布目录，并生成安装 EXE 与免安装 ZIP。

</details>

## 作者与使用说明

作者：**Abner Stephen**。项目源代码公开，**商业使用需取得作者授权**。第三方组件遵循各自许可证，详见[第三方声明](./THIRD-PARTY-NOTICES.md)。

[反馈问题或建议](https://github.com/abnste/mewu_ai/issues) · [版本更新记录](https://github.com/abnste/mewu_ai/releases/tag/v0.2.2)
