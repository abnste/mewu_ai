0.2.1 公测版集中完善原位标注、长截图、内容引用与国际化体验。 / The v0.2.1 public beta strengthens in-place annotation, scrolling capture, content references, and internationalization.

测试模型 / Tested model：MiniMax M3（MiniMax-M3）。

修复自包含安装包的运行时还原流程；0.2.0 构建未发布安装包，本次以 0.2.1 提供完整下载。 / Fixed runtime-pack restore for self-contained builds. The 0.2.0 build did not produce a release; 0.2.1 provides the complete downloads.

- 重构图片与视频标注管线：OCR 与无障碍控件校准目标，气泡全局避让，视频批注按真实呈现帧保持时间轴同步。 / Reworked image and video annotations with OCR/accessibility alignment, global callout placement, and presentation-frame timeline sync.
- 已有标注可继续追问、追加或明确重标；贴图、贴视频、复制与导出均可保留当前标注。 / Existing annotations now survive follow-ups and can be appended, replaced, pinned, copied, or exported.
- 长截图支持向上、向下连续拼接，修复滚轮焦点、图像比例及选区边框混入成品的问题。 / Scrolling capture now stitches in both directions, with fixes for wheel focus, aspect ratio, and selection borders leaking into the output.
- 手工标注支持选择、移动、重新编辑与单对象删除，工具栏按上下文收纳。 / Manual annotations can be selected, moved, re-edited, and deleted individually, with a contextual toolbar.
- 新增 AI 表格识别及一次复制 Excel、Markdown、CSV 与图片格式。 / Added AI table recognition with one-step Excel, Markdown, CSV, and image clipboard formats.
- 智能框选、按钮吸附、全屏对话条收纳和保存时机经过稳定性修复。 / Improved smart selection, control snapping, full-screen composer hiding, and capture-safe saving.
- 关于页支持 GitHub 检查更新、校验下载并静默安装；贴图层级、缩放与双击关闭同步修复。 / About now checks GitHub updates, verifies downloads, and installs silently; pinned-image layering, zoom, and double-click close were also fixed.
- 应用默认跟随 Windows UI 语言，也可在“常规”中固定简体中文或 English；安装程序自动适配系统语言。 / The app follows the Windows UI language by default and can be pinned to Simplified Chinese or English in General; the installer adapts automatically.

本版本支持 Windows 10 2004（build 19041）或更高版本，x64。安装版和免安装 ZIP 均为自包含发布包。 / Requires Windows 10 version 2004 (build 19041) or newer on x64. Both installer and portable ZIP are self-contained.

**网页原位讲解 / Explain interface controls in place**

![网页标注 / Web annotations](https://raw.githubusercontent.com/abnste/mewu_ai/v0.2.1/docs/images/web-annotations.jpg)

**圈重点、打勾与打叉 / Highlight, check, and cross out**

![AI 标记 / AI marks](https://raw.githubusercontent.com/abnste/mewu_ai/v0.2.1/docs/images/ai-checkmarks.jpg)

**原位翻译，可选中复制 / Translate in place, then select and copy**

![原位翻译 / In-place translation](https://raw.githubusercontent.com/abnste/mewu_ai/v0.2.1/docs/images/in-place-translation.jpg)

**用一句话让 AI 动笔 / Ask AI to draw**

![AI 绘图 / AI drawing](https://raw.githubusercontent.com/abnste/mewu_ai/v0.2.1/docs/images/ai-drawing.jpg)

**代码与步骤可视化讲解 / Visual explanations of code and steps**

![代码讲解 / Code explanation](https://raw.githubusercontent.com/abnste/mewu_ai/v0.2.1/docs/images/code-explanation.jpg)

[视频定位与跟踪演示 / Watch video annotation and tracking](https://github.com/abnste/mewu_ai/releases/download/v0.2.1/MewuAI-video-annotations.mp4)

示例由作者提供并授权发布；AI 结果请核对。 / Examples supplied for publication by the author; please review AI-generated results.
