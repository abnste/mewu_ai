<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="喵呜AI" />
  <h1>喵呜AI</h1>
  <p>Windows 截图工具，支持录屏、贴图、文字识别和 AI 标注。</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/tag/v0.3.0"><img src="https://img.shields.io/badge/公测版-v0.3.0-7C6CF0?style=flat-square" alt="v0.3.0 公测版" /></a>
    <img src="https://img.shields.io/badge/Windows-10_2004%2B-0078D4?style=flat-square" alt="Windows 10 2004 及以上，x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="MPL-2.0" /></a>
  </p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.3.0/MewuAI-Setup-0.3.0-win-x64.exe"><strong>下载安装版</strong></a>
    &nbsp;·&nbsp;
    <a href="https://github.com/abnste/mewu_ai/releases/download/v0.3.0/MewuAI-Portable-0.3.0-win-x64.zip">免安装版</a>
  </p>
  <p><strong>简体中文</strong> · <a href="./README.md">English</a> · <a href="./CHANGELOG.md">更新日志</a> · <a href="https://github.com/abnste/mewu_ai/issues">问题反馈</a></p>
</div>

框选屏幕上的内容，让 AI 帮你翻译、讲解或标出重点。回答和标注直接显示在画面上，也可以连同截图一起保存。

<p align="center">
  <a href="./docs/images/web-annotations.jpg"><img src="./docs/images/web-annotations.jpg" width="100%" alt="AI 在网页截图上圈出按钮，并在旁边添加说明" /></a>
</p>

## 功能

- **截图与长截图**：框选区域、选择窗口、多屏截图，支持上下滚动截取长页面。
- **标注与贴图**：画笔、高亮、箭头、形状、文字、序号和马赛克；拖动调整已有标注，把截图置顶在桌面上参考。
- **文字与表格**：识别并复制图片中的文字，翻译截图；用 AI 提取表格，粘贴到 Excel。
- **录屏与导出**：录制指定区域，支持电脑声音和麦克风；保存为 MP4 视频、MP3 音频或 GIF 动图。
- **看图、看视频提问**：引用多张截图或附件继续追问，查看 AI 批注，点击回答中的时间跳到对应视频片段。
- **选择你常用的 AI**：支持 API、Hermes、ChatGPT Work / Codex、WorkBuddy 和 MiniMax Code，可随时切换，记住上次选择。

截图、手工标注、贴图、文字识别和录屏无需配置 AI。翻译、表格提取及 AI 问答需要连接相应服务。

## 功能演示

<table>
<tr>
<td width="50%" valign="top">
<h3>圈出重点</h3>
<p>让 AI 在截图上圈选、打勾或添加说明，也可以继续追问。</p>
<a href="./docs/images/ai-checkmarks.jpg"><img src="./docs/images/ai-checkmarks.jpg" width="100%" alt="AI 在截图中圈出重点并添加勾选标记" /></a>
</td>
<td width="50%" valign="top">
<h3>截图翻译</h3>
<p>在原文的位置阅读译文，选中文字即可复制。</p>
<a href="./docs/images/in-place-translation.jpg"><img src="./docs/images/in-place-translation.jpg" width="100%" alt="译文显示在截图原文的位置" /></a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>代码讲解</h3>
<p>框选看不懂的代码，让 AI 对着具体位置解释。</p>
<a href="./docs/images/code-explanation.jpg"><img src="./docs/images/code-explanation.jpg" width="100%" alt="代码截图旁的解释与对应代码位置相连" /></a>
</td>
<td width="50%" valign="top">
<h3>绘图与批注</h3>
<p>让 AI 添加图示，或自己用画笔、形状和文字补充说明。</p>
<a href="./docs/images/ai-drawing.jpg"><img src="./docs/images/ai-drawing.jpg" width="100%" alt="截图上的 AI 绘图与批注" /></a>
</td>
</tr>
</table>

### 视频分析

录一段操作过程，或上传已有视频，直接提问。回答中的时间按钮可以跳到相关画面；播放标记的片段时，批注会跟随画面变化。

<p align="center">
  <img src="./docs/media/MewuAI-video-annotations.gif" width="880" alt="点击回答中的时间定位视频，播放时标注跟随目标移动" />
</p>

## 开始使用

1. **安装并打开。** 下载上方安装版，或解压免安装版后运行 MewuAI.exe。支持 Windows 10 2004 及以上的 x64 系统，无需另装 .NET。
2. **截取画面。** 按 <kbd>Shift</kbd> + <kbd>Alt</kbd> + <kbd>S</kbd>，拖动选择区域，再从工具条中选择复制、保存、标注、文字识别或录屏。
3. **连接 AI。** 在 **设置 → AI** 中配置并保存。截图后点击工具条中的引用按钮，把区域加入对话；也可以上传附件或直接输入文字提问。

在 **设置 → 常规** 中可以修改截图快捷键、切换简体中文或 English。在快捷键输入框按 Delete 可清空快捷键；更换界面语言后重启生效。

## 支持的 AI

| 接入方式 | 如何使用 |
| --- | --- |
| API | 填写服务商的 API Key 并选择模型。支持 OpenAI 兼容服务、MiniMax 和火山引擎，可保存多个接入点。 |
| Hermes | 连接电脑上已经配置好的 Hermes，选择人格和模型。 |
| ChatGPT Work / Codex | 使用电脑上已登录的 ChatGPT Work / Codex，在设置中选择模型和思考程度。 |
| WorkBuddy | 安装并登录 WorkBuddy 桌面版后，在设置中连接并选择模型。 |
| MiniMax Code | 使用 MiniMax Code 桌面版的登录状态；可从设置中打开客户端，无需另装命令行工具。 |

配置好多个服务后，点击对话条中 **上传按钮后面的模型按钮**，直接选择要使用的模型。各项配置会保留，下次打开仍使用上次的选择。

图片和视频支持取决于所选模型。MiniMax M3 可通过 API 或 Hermes 使用；Codex、WorkBuddy 分析视频时可能需要电脑上已有的视频处理工具。AI 回答和标注请结合原内容核对。

## 常见问题

<details>
<summary>需要付费吗？</summary>

截图、标注、贴图、文字识别和录屏可直接使用。AI 功能使用你配置的账号或 API，费用与额度由对应服务商决定。

</details>

<details>
<summary>为什么会议共享里看不到截图和标注？</summary>

打开 **设置 → 捕获 → 教学演示模式**，保存后重新截图，并在腾讯会议、极域等软件中共享整个屏幕。

开启后，框选、标注以及新建的贴图、贴视频可被共享。录制课程请使用会议软件的录制功能；使用喵呜AI自带录屏或长截图前，需要关闭教学演示模式。

</details>

<details>
<summary>截图和聊天内容会自动上传吗？</summary>

截图、手工标注和文字识别在电脑上完成。使用 AI 分析、翻译等功能时，相关内容会发送给你选择的服务；纯文字聊天不会自动附带桌面截图。

API Key 在电脑上加密保存。连接已安装的 AI 客户端也可能使用云端服务，请按自己的需求选择。

</details>

<details>
<summary>如何更新？旧版自动更新失败怎么办？</summary>

打开设置会自动检查更新，也可以在 **设置 → 关于** 中手动检查。

使用 v0.2.5 及更早版本时，请从本页下载安装器手动升级。已发布的版本和每次更新内容都可以在[更新日志](./CHANGELOG.md)中查看。

</details>

<details>
<summary>安装或录屏遇到问题怎么办？</summary>

安装包暂未代码签名，Windows 可能提示未知发布者。请从本仓库 [Releases](https://github.com/abnste/mewu_ai/releases) 下载，文件详情中可查看 SHA-256。

Windows N / KN 版本需要安装 Media Feature Pack 才能录制和播放视频。其他问题请在 [Issues](https://github.com/abnste/mewu_ai/issues) 中反馈，附上软件版本、Windows 版本和复现步骤。

</details>

## 参与项目

欢迎反馈问题、提出建议、改进代码或文档。开发环境和构建方法见[贡献指南](./CONTRIBUTING.md)，参与讨论请遵守[社区行为准则](./CODE_OF_CONDUCT.md)，安全问题见[安全策略](./SECURITY.md)。

## 开源协议

作者：**Abner Stephen**。

项目自有源码采用 [MPL-2.0](./LICENSE)，允许遵守协议的商业使用。对外分发时，请按协议提供受覆盖的源代码并保留版权和许可声明。详见[许可与源码说明](./SOURCE.md)；第三方依赖另见[第三方声明](./THIRD-PARTY-NOTICES.md)。
