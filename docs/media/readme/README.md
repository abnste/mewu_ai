# README 演示素材 / Demo assets

这些素材包含 MewuAI 官网组件演示、新录制的代码讲解，以及原项目截图；具体来源见下表与说明。截图、选区、对话、黑板和对象界面使用 Mewu 前端组件，示例提示词、回复和工具结果由演示环境预置。它们用于解释操作方式，不作为真实模型准确率、原生系统操作或任意模型成功执行的证明。CLI 片段展示固定操作与画布的对应关系。

These assets include MewuAI website component demos, a new code-explanation recording, and original project screenshots; see the individual sources below. They use Mewu frontend components with prepared example prompts, responses, and tool results. They illustrate workflows, not model-accuracy benchmarks or evidence that every operation ran against a live model or native desktop. The CLI clip illustrates the relationship between fixed operations and the canvas.

## 文件 / Files

README 首图使用重新制作的[中文封面](../../images/mewuai-cover-current-zh.png)与[英文封面](../../images/mewuai-cover-current-en.png)。它们沿用原封面的笔记本构图，以 1.0.11 正式版界面为参考生成，属于宣传示意图。界面来源、制作方式与提示词见[封面制作记录](./cover-generation.md)。原先的 `agent-overview.gif` 仍保留在素材目录。

The README hero uses regenerated [Chinese](../../images/mewuai-cover-current-zh.png) and [English](../../images/mewuai-cover-current-en.png) promotional covers, retaining the original laptop composition with references from the released 1.0.11 UI. See the [generation record](./cover-generation.md) for provenance and prompts. The previous `agent-overview.gif` remains in the media directory.

| 文件 | 内容 | 尺寸 | 时长 |
| --- | --- | --- | --- |
| [code-explanation.gif](./code-explanation.gif) | 代码讲解与原位批注 / Code explanation and inline annotation | 1280 × 720 | 11.00 s |
| [exam-annotation-latest.jpg](../../images/exam-annotation-latest.jpg) | 原 README 试卷批注 / Original README exam annotation | 1638 × 766 | — |
| [agent-overview.gif](./agent-overview.gif) | 个人 Agent 与生成清单 / Personal Agent and checklist | 1120 × 630 | 8.54 s |
| [interactive-tools.gif](./interactive-tools.gif) | HTML 粒子实验 / Interactive HTML experiment | 960 × 540 | 8.17 s |
| [blackboard.gif](./blackboard.gif) | 手绘黑板 / Freehand blackboard | 960 × 540 | 8.50 s |
| [multi-region.gif](./multi-region.gif) | 多选区自动引用 / Automatic references from multiple selections | 960 × 540 | 7.27 s |
| [video-annotations.gif](./video-annotations.gif) | 视频时间段标注 / Timed video annotations | 896 × 504 | 12.30 s |
| [cli-screen-control.gif](./cli-screen-control.gif) | CLI 截图与绘制 / CLI capture and drawing | 960 × 540 | 31.75 s |
| [creative-canvas.gif](./creative-canvas.gif) | 素材画布 / Creative canvas | 960 × 540 | 5.34 s |
| [ocr.png](./ocr.png) | 文字识别 / OCR | 1280 × 720 | 0.00 s |
| [translate.png](./translate.png) | 文字翻译 / Translation | 1280 × 720 | 0.00 s |
| [table.png](./table.png) | 表格提取 / Table extraction | 1280 × 720 | 0.00 s |

完整画面保留原始宽高比，转换时没有裁切；GIF 调整帧率、尺寸与调色板以减小体积，不加速播放。文件散列、官网源文件地址与源文件散列见 [media-manifest.json](./media-manifest.json)。本文档记录本次 README 素材快照；官网后续可能更新其源文件。

The conversion preserves the complete frame and aspect ratio without cropping or speeding up playback. GIF frame rate, dimensions, and palettes are reduced for file size. [media-manifest.json](./media-manifest.json) records file hashes, original website URLs, and source hashes. Website files may change after this snapshot.

## 视频内容授权 / Video credits

`video-annotations.gif` 中的狸花猫片段：**Cat Walking on Top of Fence**，作者 **Ileana Bondor**，来自 [Pexels](https://www.pexels.com/video/cat-walking-on-top-of-fence-10296775/)，适用 [Pexels License](https://www.pexels.com/license/)。演示使用无声短片段，并添加 Mewu 界面与分时段标注。连续标注位置为预置示例数据，不代表软件已提供自动目标追踪。

The tabby-cat footage in `video-annotations.gif` is **Cat Walking on Top of Fence** by **Ileana Bondor**, from [Pexels](https://www.pexels.com/video/cat-walking-on-top-of-fence-10296775/), used under the [Pexels License](https://www.pexels.com/license/). The demo uses a short silent excerpt with Mewu UI and timed annotations. Successive annotation positions are prepared example data, not a claim of automatic object tracking.

其余示例素材来自项目官网现有演示。Mewu 自有界面及演示代码按项目许可提供；第三方媒体适用各自许可。

Other sample materials come from the existing project website demos. Mewu-owned interface and demo code follow the project license; third-party media retain their respective licenses.

## 外部 Agent 原生截图 / External Agent native screenshot

[external-agents.jpg](./external-agents.jpg) 原样取自 [外部 Agent 插件](https://github.com/abnste/mewu-plugin-agent-connectors/blob/e3023f9426b3e97c70d077c5818af93d941f9d95/screenshots/settings-windows.jpg) 的公开截图，822 × 652，未重绘或裁切，适用该项目 MPL-2.0 许可。原项目记录为 Windows 原生 MewuAI 1.0.4 候选程序与插件 0.1.2；连接检测画面不代表所有客户端均通过完整模型对话测试。

This unmodified image comes from the External Agents plugin at the fixed source commit above, under MPL-2.0. It records native Windows MewuAI 1.0.4 candidate UI with plugin 0.1.2. Connection detection does not establish successful model execution for every client.

## 代码讲解录制与试卷案例 / Code recording and exam example

`code-explanation.gif` 于 2026-10-27 从源码提交 `1a8a037e519e0af0a6453b7d205d3c2af1c3f55d` 的 Mewu 前端录制。保留原始 App、Composer、DrawingLayer 与样式，浏览器演示桥接仅提供案例背景、示例连接、预置回复和批注数据；画面中的框选自动引用、输入、发送、代码展开均通过真实界面操作完成。示例展示 `reduce` 未设置初始值导致空数组报错，再在原图标出问题和修正。未调用真实模型或原生系统接口，不作为这些能力的实测证明。[原始录屏](./code-explanation.webm) 为 1600 × 900，转换后保持完整画面、原速与宽高比。

The code demo was recorded from the unchanged App, Composer, DrawingLayer, and styles at the source commit above. A browser-only bridge supplies the example background, connection, prepared reply, and annotation data. Selection with automatic referencing, typing, sending, and expanding the code block are actual UI interactions. It explains why `reduce` without an initial value fails on an empty array, then marks the issue and correction on the original. No live model or native operating-system call is claimed. The original recording preserves the complete 1600 × 900 frame; conversion keeps its aspect ratio and playback speed.

试卷批注按要求直接复用原 README 的 `docs/images/exam-annotation-latest.jpg`，文件字节不变，不将它标为本次重录。AI 的识读、解答与批注需要结合原题核对。

The exam example reuses the original README image byte for byte, as requested; it is not a new recording. Check AI interpretation, reasoning, and annotations against the original question.
