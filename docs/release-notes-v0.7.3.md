# 喵呜AI / MewuAI 0.7.3

## 简体中文

- 圈选后自动识别二维码和条码，直接显示复制内容、打开链接的提示。
- 多个码共用一张紧凑卡片，左右切换后操作当前码；长地址省略显示，提示避开工具条，点击时不会误拖截图。
- 新增本机记忆填充：在设置中保存关键词和值，再右键截图工具条的“文字”按钮选择扫描填充。记忆值使用 Windows DPAPI 加密，填充前核对目标窗口，支持逐项确认。
- 截图尺寸移到放大镜 XY 坐标下方的一行，和底框连在一起；取消独立黑色浮窗，调整选区时即时更新，删除选区后不留空行。感谢 [Issue #18](https://github.com/abnste/mewu_ai/issues/18) 的反馈。

感谢 [shuziyuxingxing-stack](https://github.com/shuziyuxingxing-stack) 在 [PR #19](https://github.com/abnste/mewu_ai/pull/19) 中贡献二维码识别和记忆填充功能。

0.7.2 的 OCR 内存修复及此前功能继续保留，使用方法见[截图标注指南](https://github.com/abnste/mewu_ai/blob/v0.7.3/docs/ai-screenshot-annotation.zh-CN.md)。

## English

- Automatically recognize QR codes and barcodes in a selected region, with inline actions to copy content or open a link.
- Browse multiple codes in one compact card. Actions apply to the current code; long addresses are shortened, the card stays clear of the toolbar, and clicking it no longer drags the capture.
- Add local memory-based form filling: save keyword/value mappings in settings, then right-click the capture toolbar's Text button and choose Scan and fill. Values are encrypted with Windows DPAPI, with target-window checks and per-field confirmation.
- Show capture dimensions on a line beneath the magnifier's XY coordinates within the same footer. Remove the separate dark badge, update dimensions as the selection changes, and remove the row when the selection is deleted. Thanks for the report in [Issue #18](https://github.com/abnste/mewu_ai/issues/18).

Thanks to [shuziyuxingxing-stack](https://github.com/shuziyuxingxing-stack) for contributing barcode recognition and memory fill in [PR #19](https://github.com/abnste/mewu_ai/pull/19).

The OCR memory fix from 0.7.2 and earlier features remain available. See the [screenshot annotation guide](https://github.com/abnste/mewu_ai/blob/v0.7.3/docs/ai-screenshot-annotation.md).

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.7.3/MewuAI-Setup-0.7.3-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.7.3/MewuAI-Portable-0.7.3-win-x64.zip)

安装包和便携包使用 GitHub 资产自带的 SHA-256 校验值，不额外附带校验文件。 / The installer and portable package use GitHub's built-in SHA-256 asset digests; no separate checksum file is attached.

[完整更新历史 / Full changelog](https://github.com/abnste/mewu_ai/blob/v0.7.3/CHANGELOG.md)
