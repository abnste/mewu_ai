# 喵呜AI / MewuAI 0.7.1

## 中文更新说明

本次版本修复 0.7.0 中高亮文字边缘，以及高亮与透明提取、局部修补组合编辑的问题。

- **消除高亮文字周围的浅色描边。** 荧光笔和重点高亮让文字的抗锯齿边缘平滑融入着色背景，同时保留实心字芯和细线；改善叠加高亮时的边缘颜色。
- **修复提取、修补区域盖住高亮。** 透明提取和涂抹消除的背景修补不再形成截断高亮的白色矩形。先高亮再提取、先提取再高亮，以及移动、删除、撤销重做使用一致的图层顺序。
- **修复拖动中的残影和文字受染。** 移动提取内容时使用透明预览，原处的高亮保持连续，目标位置的文字不被高亮染色。连续移动复用文字保护数据，手势结束后恢复图层顺序并清理临时预览。
- **统一现场、选择与导出。** 马赛克、背景修补和提取内容保持栅格层内部的先后顺序；上层可编辑文字和序号优先响应选择。导出与编辑画面使用相同层序，包括提取拖动期间。

0.7.0 的其他功能继续保留，完整介绍见[截图标注指南](https://github.com/abnste/mewu_ai/blob/v0.7.1/docs/ai-screenshot-annotation.zh-CN.md)。

## English release notes

This maintenance release fixes highlighted text edges and combined editing with highlights, transparent content lift and local repair in 0.7.0.

- **Remove pale outlines around highlighted text.** Freehand and region highlights blend antialiased text edges into the tinted background while preserving solid text cores and fine lines, including improved edge colors when highlights overlap.
- **Keep highlights continuous beneath lifted and repaired content.** Background repairs from seamless lift and the healing brush no longer leave white rectangles that interrupt highlights. Highlighting before or after lifting, moving, deleting, undoing and redoing use consistent layer order.
- **Prevent text ghosts and tinted text during dragging.** A transparent preview keeps the original highlighted area clear while protecting text at its destination. Consecutive drag updates reuse highlight protection data; completing the gesture restores layer order and removes the temporary preview.
- **Match presentation, selection and export.** Mosaic, background repair and lifted content retain their order within the raster layer. Editable text and numbered markers above them take selection priority. Exports use the same layer order as the editor, including while lifted content is being dragged.

Other 0.7.0 features remain available. See the [screenshot annotation guide](https://github.com/abnste/mewu_ai/blob/v0.7.1/docs/ai-screenshot-annotation.md) for the full workflow.

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.7.1/MewuAI-Setup-0.7.1-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.7.1/MewuAI-Portable-0.7.1-win-x64.zip)

支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看。

Requires Windows 10 2004 or later, x64. No separate .NET installation is needed. SHA-256 digests are available in GitHub asset details.

[完整更新历史 / Full changelog](https://github.com/abnste/mewu_ai/blob/v0.7.1/CHANGELOG.md)
