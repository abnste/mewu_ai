# 喵呜AI 0.6.2 / MewuAI 0.6.2

本版本基于 v0.6.1，集中更新截图编辑、原位录屏剪辑、翻译与英文界面，并包含此前累计的截图响应、标注布局和稳定性优化。

This release builds on v0.6.1 with screenshot editing, in-place video trimming, translation and English-interface improvements, together with accumulated capture responsiveness, annotation layout and reliability fixes.

## 中文更新说明

### 截图与手工编辑

- **更清晰的两行工具栏。** 工具与属性、撤回、重做、删除和确定分成两行，左侧对齐；橡皮擦紧接画笔，马赛克、无痕提取、涂抹消除与重点高亮放在同一组。
- **绘制和选择分开。** 使用绘制工具时不会因为经过已有图层而意外选中它；通过选择工具移动、缩放和编辑已有标注。修复文字输入、橡皮擦、删除层序和撤销重做的多处交互问题。
- **保留手工内容。** 移动或调整截图选区时，手工标注继续跟随原桌面内容；临时移出选区的内容只被裁切显示，不会被删除，撤销与恢复也保留对应关系。
- **背景高亮。** 荧光笔与“重点高亮”根据底图保护文字和细节，改善高亮后的可读性；重点高亮可以移动、调整范围和修改颜色。
- **无痕提取与涂抹消除。** 框选文字或图案后，自动修补原位置，并将提取内容变成可移动、缩放和删除的透明图层；另有可调笔刷大小的涂抹消除，每一笔支持撤回与重做。两者使用本地背景修补，适合截图文字和简单背景；复杂照片或纹理可能需要再次调整。
- **可手动设置序号。** 新增“下个序号”输入与加减按钮，删除后可以调回需要的编号继续标注。序号图标和标出的数字按实际轮廓居中。
- **紧凑像素放大镜。** 无外框方形镜面配中央十字线，下方相连底框分别显示 RGB 和坐标；屏幕边缘自动避让，并修正高缩放下可能差一个像素的取样。

### 原位录屏剪辑

- 录制完成后，在原位预览下方拖动进度条定位，直接调整起点和终点，也可恢复完整片段。
- 保存、复制、贴视频及发送给 AI 参考均使用选定片段，音频和时间轴标注同步裁切。
- 修复调整裁切范围时的短暂空白、闪回开头及末尾定位问题；重做录屏倒计时样式并修正数字居中。

### 翻译与语言

- 原位翻译保持 OCR 原文的逐行结构，较长译文在原行空间内适配字号，不再因为下方有空白而额外换行；显示与复制采用一致的行内处理。
- 补齐 MiniMax Code、Codex、WorkBuddy 和部分 MCP 设置页的英文文案。英文状态下，附件引用显示为 `@Image1`、`@Video1`、`@File1` 和 `@CurrentScreen`，相关提示同步切换。

### 响应与稳定性

- 点击框选不再同步等待外部界面辅助信息；窗口吸附结果核对当前指针、截图帧和目标窗口，减少陈旧结果造成的误选。冻结会话或替换后的截图，不再用当前桌面的辅助信息误调标注位置。
- 改善多屏负坐标、屏幕边缘、物理像素步进和缩放贴图的边缘裁切；截图启动失败时恢复相关窗口。
- 减少拖动期间的重复处理，复用高亮与取样缓存，并限制派生图像的内存开销。
- 视频标注去重比较完整运动轨迹，避免误删中途分离的标注；关闭或状态变化后，过期匹配结果不能覆盖较新的编辑。导出标注时，仅实际显示的说明卡参与重复框判断，未显示的说明卡不再误隐藏本应显示的区域框。

v0.6.1 的 MCP 服务、网页抓取、屏幕实体操作和 WorkBuddy 更新继续保留。

## English release notes

### Capture and manual editing

- **Two aligned toolbar rows.** Tools occupy the first row; properties, undo, redo, delete and done occupy the second. The eraser follows the pen, while mosaic, seamless lift, healing brush and emphasis highlighting share a group.
- **Separate drawing and selection.** Drawing over existing layers no longer unexpectedly selects them. Use the selection tool to move, resize and edit annotations. Fixes cover text input, erasing, deletion order, undo and redo.
- **Preserved manual content.** Moving or resizing a desktop capture keeps annotations anchored to the original content. Content outside the current selection is clipped rather than deleted, including through undo and restoration.
- **Background highlighting.** Freehand and region highlights protect text and image detail to improve readability. Emphasis highlights can be moved, resized and recolored.
- **Seamless lift and healing brush.** Select text or artwork to repair its original background and lift it onto a movable, resizable, removable transparent layer. A separate brush repairs painted areas with adjustable size and per-stroke undo/redo. These local repairs suit screenshot text and simple backgrounds; complex photos or textures may need further adjustment.
- **Adjustable numbering.** A next-number field and step buttons let you restart from a chosen number after deletion. Number-tool icons and labels are centered using their visible glyphs.
- **Compact pixel magnifier.** A borderless square view with a central crosshair connects directly to two RGB and coordinate rows. It stays within the current screen and fixes possible one-pixel sampling errors at higher display scaling.

### In-place video trimming

- Seek from the timeline beneath a completed recording, adjust its start and end, or restore the full clip.
- Save, copy, pin and AI references all use the selected segment, with audio and timed annotations trimmed consistently.
- Fix blank flashes, jumps to the beginning and end-positioning problems while adjusting the range. The recording countdown now has a matching compact design and centered digits.

### Translation and language

- In-place translations preserve each original OCR line, fitting longer text within that line instead of wrapping into empty space below. Displayed and copied text use consistent line handling.
- Complete English text in MiniMax Code, Codex, WorkBuddy and selected MCP settings pages. English references use `@Image1`, `@Video1`, `@File1` and `@CurrentScreen`, with matching hints.

### Responsiveness and reliability

- Pointer-down no longer waits synchronously for external accessibility queries. Window snapping validates the pointer, captured frame and target window before accepting a result. Frozen conversation captures and replacement images are no longer realigned using accessibility information from the current desktop.
- Improve negative monitor coordinates, screen edges, physical-pixel stepping and scaled pinned-image clipping. Failed capture startup restores affected windows.
- Reduce repeated work while dragging, reuse highlight and sampling caches, and bound derived-image memory use.
- Video annotation deduplication compares complete motion trajectories so annotations that later separate are not incorrectly removed. Stale matching results cannot overwrite newer edits after closing or changing state. During annotated export, only rendered callouts suppress duplicate target boxes, so hidden callouts no longer remove boxes that should remain visible.

The MCP integrations, web crawling, screen-entity actions and WorkBuddy improvements from v0.6.1 remain included.

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.2/MewuAI-Setup-0.6.2-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.2/MewuAI-Portable-0.6.2-win-x64.zip)

支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看。

Requires Windows 10 2004 or later, x64. No separate .NET installation is needed. SHA-256 digests are available in GitHub asset details.

[完整更新历史 / Full changelog](https://github.com/abnste/mewu_ai/blob/v0.6.2/CHANGELOG.md)
