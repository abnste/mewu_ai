<div align="center">
  <img src="./Assets/MewuAI.Icon.png" width="88" alt="MewuAI 喵呜AI" />
  <h1>MewuAI · 喵呜AI</h1>
  <p><strong>LLM 驱动的开源桌面 Agent 终端</strong></p>
  <p>个人 Agent · AI 创意画布 · 视觉标注 · 桌面工具与自动化</p>
  <p>
    <a href="https://github.com/abnste/mewu_ai/releases/latest"><img src="https://img.shields.io/github/v/release/abnste/mewu_ai?style=flat-square&color=7C6CF0" alt="最新正式版" /></a>
    <img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat-square" alt="Windows x64" />
    <img src="https://img.shields.io/badge/macOS-Apple_Silicon-555555?style=flat-square" alt="macOS Apple Silicon" />
    <img src="https://img.shields.io/badge/Linux-x64-E95420?style=flat-square" alt="Linux x64" />
    <a href="./LICENSE"><img src="https://img.shields.io/badge/License-MPL--2.0-blue?style=flat-square" alt="MPL-2.0" /></a>
  </p>
  <p>
    <a href="https://www.mewu.ai/"><strong>官网与功能演示</strong></a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu_ai/releases/latest"><strong>下载 MewuAI</strong></a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu-plugins">插件市场</a>
    &nbsp;·&nbsp; <a href="https://github.com/abnste/mewu_ai/issues">问题与建议</a>
  </p>
  <p><strong>简体中文</strong> · <a href="./README.md">English</a></p>
</div>

**MewuAI 是一款跨 Windows、macOS 和 Linux 的个人 Agent 应用。** 你可以与 AI 对话，把截图、视频、文件和手绘标注交给它理解，也可以让它生成可交互的 HTML 工具，在画布中使用结果，或通过插件操作授权区域、检索记忆和执行自动化任务。

**Mewu with you**：让 Mewu 陪你工作、与你协作。圈出一个细节、画出一条关系、引用几个画面，让 AI 理解你具体在说什么；生成的内容继续留在同一个工作空间里修改和使用。

[功能](#功能) · [使用场景](#使用场景) · [开始使用](#开始使用) · [AI 接入](#ai-接入与个人-agent) · [CLI](#cli与其他-agent-共用桌面空间) · [Hooks](#hooks事件触发器) · [插件](#插件市场) · [开发](#开发与参与)

## 功能

### 个人 Agent 与对话

为不同任务创建 Agent，设置名称、灵魂设定、默认模型与记忆提供方。在对话条中选择 Agent 和模型，引用区域、图片、视频或文件后继续交流。支持多会话、历史记录、流式回复、Markdown、公式和工具执行记录。

Agent、模型连接和记忆可以分别配置：使用 Mewu 的 Agent，也可以通过插件接入你已有的 Agent 运行时。MCP 与插件工具可以参与同一轮任务。

### AI 创意画布与交互式工具

通过自然语言生成 **HTML / SVG / Canvas 交互对象**，例如小游戏、计算器、可调参数的图表和演示控件。结果可以放在空间中移动、调整大小并直接交互，继续描述需求来修改内容。

截图、图片、视频、黑板与生成对象可以放在同一空间中。电子黑板提供自由绘制、文字、形状与撤销重做；配合工具包插件，还可以收藏素材和工具，在其他会话中拖出复用。

### 框选、标注与视觉交流

- **局部与多选区引用**：连续框选多个区域，完成框选后自动加入对话引用；非选区压暗，便于集中表达任务范围。
- **手工标注**：画笔、箭头、形状、文字、序号、高亮和马赛克，配合选择、橡皮及撤销重做，指出位置、修改意见或画面之间的关系。
- **AI 原位作答**：让 AI 在截图或图片上圈出重点、写说明、画图示，或结合多个引用在对话中回答。批注与原始图片分开保存。
- **基础截图工具**：复制、保存、贴图与长截图；自动应用快照依赖 Windows UI Automation 及目标应用支持。

### 视频标注与录屏

导入视频或录制屏幕区域，在同一空间播放、裁切、引用和提问。AI 可以为对应时间段添加矩形与文字标注，点击回答中的时间片段定位到相关画面。

录屏支持电脑声音、麦克风与暂停继续；裁切保留原片，可导出 **MP4、MP3 和 GIF**。视频理解与 AI 标注取决于所选模型的视觉能力。

### 文字、表格与图表

- **离线 OCR**：识别截图中的文字，拖选并复制需要的部分。
- **原位翻译**：在原文所在位置显示译文，便于对照阅读。
- **表格提取**：把截图中的表格整理为可复制的行列，继续粘贴到电子表格。
- **图表理解**：引用图表，询问数值、趋势与差异，或请 AI 标出需要关注的部分。
- **二维码与条码**：识别选区中的码，提供复制、打开等操作。

截图、手工绘制、录屏和本机 OCR 不需要模型账号；翻译、AI 表格提取、图表理解及内容生成需要相应模型或 Agent。

### 区域 AI 操作

通过[区域操作插件](https://github.com/abnste/mewu-plugin-region-agent)，在原对话条引用并授权一个区域，描述要完成的操作。Agent 观察当前截图、点击或输入，再根据新的截图继续判断；可以在过程中停止。

当前支持 Windows、macOS，以及安装 [Mewu Desktop Witness](./tools/linux-desktop-witness/README.md) 的 **GNOME 50 / Wayland / x64**。macOS 需要录屏与辅助功能权限，Linux 需要 Portal 授权；其他 Linux 桌面尚未适配区域输入。

## 使用场景

以下是可以围绕实际内容发起的任务示例，回答和生成结果由你选择的模型决定。

| 场景 | 操作与表达 |
| --- | --- |
| 界面评审 | 框选按钮和说明文字，画箭头指出关系：“这两处信息重复，帮我整理，并在图上标注修改建议。” |
| 代码与报错讲解 | 同时引用代码和错误信息：“解释它们的关系，标出需要检查的位置。” |
| 试卷与学习 | 引用题目和解题过程：“找出推导有问题的步骤，在原图说明，再给一道同类型练习。” |
| 视频阅读 | 引用一段有实际内容的视频：“标出关键画面，说明各时间段发生了什么。”从回答跳回视频核对。 |
| 资料整理 | 框选页面文字、表格和图表，提取数据、翻译段落或比较趋势；用便笺保存结论与引用。 |
| 交互工具制作 | “做一个能调节参数的演示工具”“把这个计算过程做成计算器”，生成后直接在画布操作，再继续修改。 |
| 桌面任务 | 授权目标区域，让 Agent 在该区域点击、填写并查看结果；通过截图反馈继续任务。 |
| 重复工作自动化 | 用 Hooks 设置定时任务、关键词规则或快捷键，把提示词、MCP 工具和本机操作串联起来。 |

界面与操作演示见 [mewu.ai](https://www.mewu.ai/)。教学、识读和数据分析的结论请结合原内容核对。

## 开始使用

### 下载安装

安装包与更新说明统一发布在 [GitHub 最新正式发行](https://github.com/abnste/mewu_ai/releases/latest)。

| 平台 | 下载包 | 运行说明 |
| --- | --- | --- |
| [Windows x64](https://github.com/abnste/mewu_ai/releases/latest) | 安装版 `Setup…exe` / 便携版 `Portable…zip` | Windows 10 2004 及以上，使用 WebView2 |
| [macOS Apple Silicon](https://github.com/abnste/mewu_ai/releases/latest) | `mac-arm64.dmg` | 将应用放入“应用程序”，按系统提示授予所需权限 |
| [Linux x64](https://github.com/abnste/mewu_ai/releases/latest) | `linux-x64.AppImage` | 添加可执行权限后启动，使用 WebKitGTK 与桌面 Portal |

1. **打开空间。** 启动 MewuAI，点击托盘图标或使用设置中的截图快捷键。框选后即可标注、复制、保存，也可以打开黑板或导入素材。
2. **连接 AI。** 在 **设置 → Agent 与连接 → 连接** 配置模型，或安装外部 Agent 插件并完成连接与角色绑定。
3. **表达任务。** 选择 Agent，框选需要的内容或添加文件，在对话条输入要求。可以先手绘标注，再发送给 AI。
4. **添加工具。** 在 **设置 → 插件** 安装需要的扩展；在 **Hooks** 中添加事件规则。

Linux 需要系统 `libnghttp2.so.14`；本机 OCR 需要 Tesseract 4.1/5 和英语语言数据，Ubuntu 对应 `libnghttp2-14` 与 `tesseract-ocr-eng`。Wayland 截图、录屏需要桌面授权；录屏授权后，在刷新的预览中重新确认区域。更多说明见 [Linux OCR](./docs/linux-native-ocr.md)、[录屏](./docs/linux-native-recording.md)与[安装包权限](./docs/linux-appimage-permissions.md)。

## AI 接入与个人 Agent

| 接入方式 | 用途 |
| --- | --- |
| 模型 API | Chat Completions、OpenAI Responses、Anthropic Messages；保存多个连接，使用服务商模板或自定义地址，读取模型列表并切换模型。 |
| 外部 Agent | 通过[外部 Agent 插件](https://github.com/abnste/mewu-plugin-agent-connectors)接入 **Codex、Hermes、MiniMax Code、WorkBuddy / CodeBuddy**，在原对话条继续使用。 |
| MCP | 在 **Agent 与连接 → MCP** 配置服务，让当前 Agent 调用其提供的工具。 |
| 长期记忆 | 为 Agent 选择本地 SQLite/FTS5 记忆，或独立的 Hindsight、日志记忆等提供方。 |

外部 Agent 插件使用 Codex App Server 或 ACP，沿用对应客户端的登录和运行配置，需要安装并配置兼容的客户端。普通对话、图片及会话恢复按各适配器支持情况提供；第三方专用视频上传和完整工具审批界面尚未接入。各客户端的具体要求见[插件说明](https://github.com/abnste/mewu-plugin-agent-connectors#平台接口)。

模型、Agent 和记忆提供方分别选择。你可以为不同任务保存不同角色设定，让同一个角色持续使用自己的记忆，再按任务切换模型。

## CLI：与其他 Agent 共用桌面空间

官方 CLI 插件随应用提供，默认启用。先启动 MewuAI，再打开终端：

```sh
mewu
mewu chat "帮我梳理这项任务的步骤"
mewu status
mewu schema
mewu call capture.open --json '{}' --wait
mewu scene list
```

`mewu` 可以直接进入交互式 Agent 对话；CLI 也能让本机其他 Agent **唤起截图、创建选区、设置引用、绘制标注、导出图片、调用 OCR 和控制录屏**。命令与桌面共用会话、模型、记忆及工具。

自动化客户端先读取 `mewu schema`，再使用实际返回的会话、区域和修订信息调用接口。写操作返回执行回执；`--wait` 等待操作完成，不代表整轮 AI 答复已经结束。终端聊天默认只发送文字，需要图像时应显式设置引用。

Windows 安装后打开新终端；macOS/Linux 启动已安装应用后会尝试建立用户级命令入口。Linux AppImage 的本机 CLI 连接需要 Linux 6.5+。完整用法见[双语 CLI 指南](./docs/cli.md)和[接口目录](./apps/desktop/control/catalog.json)。

## Hooks：事件触发器

Hooks 是随应用提供、默认启用的官方原生插件。可以在设置侧栏管理规则，每条规则独立启停、编辑，并查看运行记录。

| 触发方式 | 可以怎样使用 |
| --- | --- |
| 定时 / Cron | 到指定时间向 Agent 发送任务，或运行本机工作流；支持时区。 |
| 提示词 | 根据关键词、完全匹配或正则，为本次请求补充约定的指令。 |
| AI 回复 | 在完整回复结束后匹配内容，执行后续动作。 |
| 全局快捷键 | 用快捷键打开截图、创建黑板或触发自定义流程。 |
| 生命周期 / 自定义事件 | 响应应用、会话、Agent、工具或插件事件，按条件执行动作。 |

动作可组合 **提示词注入、发送消息、MCP、命令与程序、打开网站、键鼠操作、JavaScript 和 Mewu 操作**。截图快捷键也由普通 Hooks 规则管理。规则提供冷却、超时和停止控制；系统授权要求仍由操作系统决定。

具体配置与示例见 [Hooks 使用文档](./docs/hooks.md)。

## 插件市场

在 **设置 → 插件** 浏览市场、管理已安装扩展或启停原生功能。社区插件从各自 GitHub 仓库的 Release 安装，安装时可以查看来源、权限和版本。

| 插件 | 用途 |
| --- | --- |
| [外部 Agent](https://github.com/abnste/mewu-plugin-agent-connectors) | 接入 Codex、Hermes、MiniMax Code、WorkBuddy / CodeBuddy。 |
| [区域 AI 操作](https://github.com/abnste/mewu-plugin-region-agent) | 在授权区域内观察、点击、输入，并通过新截图核验结果；市场名称为“区域操作 Agent”。 |
| [Hindsight](https://github.com/abnste/mewu-plugin-hindsight) | 连接长期记忆服务，选择服务器已有记忆库，保存和检索记忆。 |
| [日志记忆](https://github.com/abnste/mewu-plugin-journal-memory) | 按 Agent 隔离、持久保存和检索记忆。 |
| [工具包](https://github.com/abnste/mewu-plugin-object-toolbox) | 收藏图片、视频、生成对象和黑板等素材，跨会话拖出复用。 |
| [研究便笺](https://github.com/abnste/mewu-plugin-research-notes) | 在浮动便笺中编辑内容，保存正文和拖入的对象引用。 |
| [交互圆角尺](https://github.com/abnste/mewu-plugin-interactive-widget) | 一个可在空间中移动和操作的透明 SVG 工具，也是交互对象插件的示例。 |

[浏览社区市场](https://github.com/abnste/mewu-plugins) · [Mewu API / SDK](./packages/sdk/README.md) · [插件开发指南](./docs/extensions.md) · [独立打包与上架](./tools/extension-market/README.md#from-an-independent-project-to-a-market-pr)

插件可提供工具、Agent、记忆、设置页和空间对象。JavaScript 插件运行在独立 QuickJS 进程中，使用授权的宿主接口，安装插件不需要另装 Node.js；需要启动外部客户端的插件仍要求相应客户端已安装。插件停用会撤销其入口和执行能力，已有插件数据保留。

后续方向包括参考图生图与更多图像/视频生成流程、桌面宠物、即圈即搜，以及更多插件与 Linux 桌面适配。这些是规划方向，具体可用能力以发行说明和市场中的实际插件为准。

## 数据、权限与更新

- **本机数据**：新安装默认保存在 `~/.mewu`；在 **设置 → 数据** 查看占用、迁移目录和清理内容。
- **AI 请求**：截图与手工绘制在本机完成；发送 AI 请求时，引用内容与相关上下文交给你选择的模型或 Agent 服务。插件按授权访问能力。
- **系统差异**：区域输入、录屏、OCR 等依赖系统能力和权限。二维码/条码及链接、邮箱、电话号码的可操作识别目前支持 Windows/macOS；原生图片公式渲染目前支持 Windows，聊天公式使用三端共享渲染。
- **屏幕共享**：可在 **设置 → 通用 → 允许屏幕共享看到框选和标注** 调整，实际效果也取决于系统和共享软件。
- **应用更新**：在 **设置 → 关于** 检查更新，下载来自 GitHub 正式发行。Windows 便携版自动更新会运行安装器并转为安装版；需要继续便携使用时下载新的便携 ZIP。
- **安装帮助**：Windows 安装包尚无 Authenticode 证书；N/KN 系统录屏可能需要 Media Feature Pack。遇到问题请在 [Issues](https://github.com/abnste/mewu_ai/issues) 附上系统、版本和复现步骤。

## 开发与参与

MewuAI 使用 **Rust + Tauri 2 + Solid + TypeScript + SQLite**，基于系统 WebView。当前项目源码位于 `master`。

| 目录 | 内容 |
| --- | --- |
| [apps/desktop](./apps/desktop) | 桌面前端与 Tauri 原生宿主 |
| [crates](./crates) | 核心数据、控制协议、Hooks 与扩展运行时 |
| [tools/mewu-cli](./tools/mewu-cli) | 本机命令行客户端 |
| [packages/sdk](./packages/sdk) | 插件开发 SDK |
| [extensions](./extensions) | 插件源码与示例 |
| [docs](./docs) | 功能、平台与发行文档 |

前端开发使用 `npm ci` 与 `npm run dev`，原生开发使用 `npm run desktop:dev`。浏览器预览不提供系统截图、原生录屏或真实桌面输入；原生应用需要相应平台的 Rust/Tauri 工具链，Linux 构建使用项目的完整 Docker 流程。

[桌面开发指南](./apps/desktop/README.md) · [Linux 构建与测试](./docs/linux-testing.md) · [发行流程](./docs/releasing.md)

欢迎提交问题、使用案例、翻译、文档、代码和插件。感谢[所有项目贡献者](https://github.com/abnste/mewu_ai/graphs/contributors)。

## 许可与致谢

项目采用 [MPL-2.0](./LICENSE)。第三方组件及字体的许可见[第三方许可全文](./apps/desktop/THIRD-PARTY-NOTICES)。

感谢 **林文杰老师**提供学科帮助、**张文彪老师**提供功能开发建议，以及 [**blueling0**](https://github.com/blueling0) 协助测试。

合作项目：[**kangarooking/Ta（拓）**](https://github.com/kangarooking/Ta)，面向 macOS 的 AI 截图工具。
