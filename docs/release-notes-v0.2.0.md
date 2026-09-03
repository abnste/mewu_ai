0.2.0 公测版集中完善原位标注、长截图、内容引用与国际化体验。 / The v0.2.0 public beta strengthens in-place annotation, scrolling capture, content references, and internationalization.

测试模型 / Tested model：MiniMax M3（MiniMax-M3）。

- 重构图片与视频标注管线：OCR 与无障碍控件校准目标，气泡全局避让，视频批注按真实呈现帧保持时间轴同步。 / Reworked image and video annotations with OCR/accessibility alignment, global callout placement, and presentation-frame timeline sync.
- 已有标注可继续追问、追加或明确重标；贴图、贴视频、复制与导出均可保留当前标注。 / Existing annotations now survive follow-ups and can be appended, replaced, pinned, copied, or exported.
- 长截图改为原位滚轮采集与有限重叠拼接，不再以置顶窗口冒充结果。 / Scrolling capture now uses bounded, in-place wheel capture and overlap stitching.
- 手工标注支持选择、移动、重新编辑与单对象删除，工具栏按上下文收纳。 / Manual annotations can be selected, moved, re-edited, and deleted individually, with a contextual toolbar.
- 新增 AI 表格识别及一次复制 Excel、Markdown、CSV 与图片格式。 / Added AI table recognition with one-step Excel, Markdown, CSV, and image clipboard formats.
- 智能框选、按钮吸附、全屏对话条收纳和保存时机经过稳定性修复。 / Improved smart selection, control snapping, full-screen composer hiding, and capture-safe saving.
- 关于页支持 GitHub 检查更新、校验下载并静默安装；贴图层级、缩放与双击关闭同步修复。 / About now checks GitHub updates, verifies downloads, and installs silently; pinned-image layering, zoom, and double-click close were also fixed.
- 应用与安装程序自动跟随 Windows UI 语言，原生支持简体中文与自然英语。 / The app and installer now follow the Windows UI language with native Simplified Chinese and natural English.

本版本支持 Windows 10 2004（build 19041）或更高版本，x64。安装版和免安装 ZIP 均为自包含发布包。 / Requires Windows 10 version 2004 (build 19041) or newer on x64. Both installer and portable ZIP are self-contained.
