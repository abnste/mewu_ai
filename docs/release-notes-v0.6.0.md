# 喵呜AI 0.6.0 / MewuAI 0.6.0

这是一个合并发行版，汇总 0.5.3 至 0.6.0 的公开更新，并包含最新的混合显卡录屏启动和 NVIDIA 即时重放兼容修复。/ This consolidated release includes the public changes from 0.5.3 through 0.6.0, including the latest hybrid-display recording startup and NVIDIA Instant Replay compatibility fixes.

## 更新内容 / Changes

- **0.5.3–0.5.5：框选与编码稳定性。** 框选拖动复用冻结桌面图像，录屏使用更稳定的 H.264 编码路径，并加入 100% 与 175% DPI 的像素一致性回归。/ **0.5.3–0.5.5: selection and encoding stability.** Reuse the frozen desktop image during selection dragging, use a stable H.264 path, and cover pixel consistency at 100% and 175% DPI.
- **0.5.6：历史会话。** 新建对话清空当前聊天列表并保持独立上下文；历史按钮恢复之前的完整会话。/ **0.5.6: conversation history.** New conversations clear the current list and keep an isolated context; the history button restores complete earlier sessions.
- **0.5.7–0.5.9：录制交互与显示。** 区外输入交给底层应用；录制保留实时遮罩和边框；移除全局鼠标钩子与事件重注入；停止流程有界恢复；修复工具条和 AI 标注卡阴影。/ **0.5.7–0.5.9: recording interaction and presentation.** Outside-bar input reaches the underlying app; recording keeps a live dimmer and border; global mouse hooks and event reinjection are removed; stop recovery is bounded; toolbar and AI annotation shadows are fixed.
- **0.5.10：电脑声音录制稳定性。** 修复静音桌面开启电脑声音录制时约 3 秒后画面停留在旧帧的问题，保持共享 WASAPI 时钟连续。/ **0.5.10: computer-audio recording stability.** Fix video remaining on an old frame after roughly three seconds on a silent desktop with computer audio enabled, while keeping the shared WASAPI clock active.
- **0.6.0：显示器录制与第三方录屏兼容。** 混合显卡或虚拟显示环境显式使用 Windows Graphics Capture，避免 Desktop Duplication 不支持导致录制约一秒后停止；失败原因写入隐私日志并及时反馈。教学/屏幕共享刷新桌面时临时隐藏覆盖层，不再切换会中断 NVIDIA Instant Replay 的防捕获标记；弹窗跟随所属窗口的捕获策略，关闭共享模式后截图和贴图仍保持保护。/ **0.6.0: display capture and recorder compatibility.** Use Windows Graphics Capture explicitly on hybrid-GPU and virtual-display systems to avoid recordings stopping after about one second because Desktop Duplication is unsupported; report failures promptly and retain them in the privacy log. Temporarily cloak the overlay during teaching/shared-desktop refresh instead of changing capture-protection flags that interrupt NVIDIA Instant Replay; dialogs follow their owner's capture policy, while screenshots and pinned windows remain protected when sharing is off.

## 验证 / Validation

- Release 构建 0 警告、0 错误；全量测试 1549 项通过、1 项既有现场音频测试默认跳过；启用 `MEWU_AUDIO_LIVE=1` 后声音相关 10 项全部通过。/ Release builds completed with 0 warnings and 0 errors; all 1549 tests passed with 1 pre-existing interactive-audio test skipped by default; with `MEWU_AUDIO_LIVE=1`, all 10 audio tests passed.
- 新增混合显卡/虚拟显示器录制启动回归，确认录制进入就绪状态并持续超过 3 秒。/ Added a hybrid-GPU/virtual-display startup regression that confirms readiness and more than three seconds of recording.

## 下载 / Downloads

[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Setup-0.6.0-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Portable-0.6.0-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看，不附带单独校验文件。/ [Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Setup-0.6.0-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Portable-0.6.0-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in GitHub asset details; no separate checksum file is included.

完整历史仍保留在 [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.0/CHANGELOG.md)。/ The full history remains in [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.0/CHANGELOG.md).
