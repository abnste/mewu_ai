# 喵呜AI / MewuAI 0.7.2

## 简体中文

本次修复反复使用离线 OCR 后内存占用偏高的问题。感谢 [Issue #17](https://github.com/abnste/mewu_ai/issues/17) 的反馈。

- 识别完成后释放推理临时内存，减少连续截图识别后的内存积累。
- 排队等待识别的请求不再提前分配整张截图的缓冲，减少同时发起识别时的额外占用。
- 保留 OCR 模型复用，后续识别无需反复加载模型。

0.7.1 及之前的功能继续保留，完整介绍见[截图标注指南](https://github.com/abnste/mewu_ai/blob/v0.7.2/docs/ai-screenshot-annotation.zh-CN.md)。

## English

This update reduces memory retained after repeated offline OCR. Thanks for the report in [Issue #17](https://github.com/abnste/mewu_ai/issues/17).

- Release temporary inference memory after recognition to reduce accumulation during repeated captures.
- Avoid allocating full screenshot buffers while recognition requests wait in the queue.
- Keep OCR models cached so subsequent requests do not reload them.

Features from 0.7.1 and earlier remain available. See the [screenshot annotation guide](https://github.com/abnste/mewu_ai/blob/v0.7.2/docs/ai-screenshot-annotation.md) for the full workflow.

## 下载 / Downloads

[安装版 EXE / Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.7.2/MewuAI-Setup-0.7.2-win-x64.exe) · [便携版 ZIP / Portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.7.2/MewuAI-Portable-0.7.2-win-x64.zip)

安装包和便携包使用 GitHub 资产自带的 SHA-256 校验值，不额外附带校验文件。 / The installer and portable package use GitHub's built-in SHA-256 asset digests; no separate checksum file is attached.

[完整更新历史 / Full changelog](https://github.com/abnste/mewu_ai/blob/v0.7.2/CHANGELOG.md)
