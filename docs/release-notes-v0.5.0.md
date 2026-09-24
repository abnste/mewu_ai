# 喵呜AI 0.5.0 / MewuAI 0.5.0

- 对话条支持拖动：从顶部空白边距拖出一定距离后可固定摆放，不再自动隐藏；拖回原位虚线框附近松手，以弹性动画吸附并恢复自动隐藏。没有额外拖动横条或空白占位行。
- 改善 [Issue #4](https://github.com/abnste/mewu_ai/issues/4) 相关截图交互：新截图位于旧贴图上方，只有本轮新贴图保持在截图上方；修复框选后鼠标进入选区或工具条时对话条不收纳的问题。
- 修复文字标注移动、缩放和暂时失焦后的连续编辑问题，保留文字内容，并支持直接修改当前文字的颜色、荧光、字体与字号。
- 采纳并补修 [PR #10](https://github.com/abnste/mewu_ai/pull/10)，感谢 **shuziyuxingxing-stack**：支持手动选择 Codex / WorkBuddy 可执行文件并保存路径；设置窗口支持最小化；Codex 可沿用本机 ChatGPT、API Key 或 CCSwitch 登录配置。
- 基于该贡献者的 JSON 防护，补齐 API 和本机代理的响应字段、类型与终态检查，改善畸形响应的错误处理。
- 修复 GitHub 更新接口限流后误请求不存在的校验文件而报 404。无法取得可信校验值时提示稍后重试，不跳过安全校验；旧版遇到此问题可手动下载安装器升级。

下载：[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.0/MewuAI-Setup-0.5.0-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.0/MewuAI-Portable-0.5.0-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看，不附带单独校验文件。

使用 v0.2.5 及更早版本时，请下载安装器手动升级。此前改动见[更新日志](https://github.com/abnste/mewu_ai/blob/v0.5.0/CHANGELOG.md)。

---

- Drag the composer from its existing top padding to detach it and keep it visible. Release near the dashed original docking target to spring back and restore auto-hide. No visible grip or extra empty row is added.
- Improve capture interactions related to [Issue #4](https://github.com/abnste/mewu_ai/issues/4): new captures stay above older pins, while pins created during the current capture remain above it. Restore composer auto-hide when the pointer moves into a selection or its toolbar after selecting.
- Preserve text annotations through moves, resizing and temporary focus changes; edit the selected text's color, highlight, font and size directly.
- Adopt and complete [PR #10](https://github.com/abnste/mewu_ai/pull/10), with thanks to **shuziyuxingxing-stack**: manually select and save Codex / WorkBuddy executable paths, minimize Settings, and reuse local ChatGPT, API-key or CCSwitch authentication configuration in Codex.
- Integrate the contributor's JSON guards into API and local-agent responses, validating fields, types and completion states and handling malformed responses safely.
- Stop requesting a nonexistent checksum file after GitHub rate limits, eliminating that erroneous 404. When trusted verification data is unavailable, retry later rather than bypassing verification. Older clients encountering this problem can upgrade with the installer below.

Downloads: [installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.5.0/MewuAI-Setup-0.5.0-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.5.0/MewuAI-Portable-0.5.0-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in GitHub asset details; no separate checksum file is included.

If you use v0.2.5 or earlier, download the installer to upgrade manually. See the [changelog](https://github.com/abnste/mewu_ai/blob/v0.5.0/CHANGELOG.md) for earlier changes.
