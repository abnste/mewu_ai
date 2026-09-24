# 喵呜AI 0.6.0 / MewuAI 0.6.0

本版本基于 v0.5.10，**只汇总 0.5.10 之后合入的公开更新**。/ This release is based on v0.5.10 and **only covers public changes merged after v0.5.10**.

## 更新内容 / Changes after v0.5.10

- **混合显卡与虚拟显示器录屏启动修复。** 显示器采集显式使用 Windows Graphics Capture，避免 Desktop Duplication 在部分环境中返回不支持错误，导致录制约一秒后停止；失败原因写入隐私日志，并新增录制进入就绪状态且持续超过 3 秒的回归测试。/ **Hybrid-GPU and virtual-display recording startup.** Use Windows Graphics Capture explicitly to avoid unsupported Desktop Duplication errors that stopped recordings after about one second on some systems; record failures in the privacy log and add a regression that confirms readiness and more than three seconds of recording.
- **NVIDIA 即时重放与屏幕共享兼容。** 教学/屏幕共享刷新桌面时临时隐藏覆盖层，不再切换会中断第三方桌面录制的防捕获标记；弹窗跟随所属窗口的捕获策略，公开许可证窗口可被捕获，关闭共享模式后截图和贴图仍保持保护。/ **NVIDIA Instant Replay and shared-desktop compatibility.** Temporarily cloak the overlay during teaching/shared-desktop refresh instead of changing capture-protection flags that interrupt third-party desktop recording; dialogs follow their owner's capture policy, public license notices remain capturable, and screenshots and pinned windows stay protected when sharing is off.
- **外部录屏验证工具更新。** 增加 shared-only replay 验证路径，覆盖 NVIDIA 即时重放场景下的桌面共享行为。/ **External recorder validation.** Add a shared-only replay path covering desktop sharing behavior with NVIDIA Instant Replay.
- **公开文档署名同步。** 更新中英文 README 的作者头像、署名和许可说明。/ **Public documentation attribution.** Synchronize author avatars, attribution, and licensing information in both README files.

## 验证 / Validation

- Release 构建 0 警告、0 错误；全量测试 1550 项通过、1 项既有现场音频测试默认跳过。/ Release builds completed with 0 warnings and 0 errors; all 1550 tests passed with 1 pre-existing interactive-audio test skipped by default.
- 新增混合显卡/虚拟显示器录制启动回归，确认录制进入就绪状态并持续超过 3 秒。/ Added a hybrid-GPU/virtual-display startup regression that confirms readiness and more than three seconds of recording.

## 下载 / Downloads

[安装版 EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Setup-0.6.0-win-x64.exe) 或 [便携版 ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Portable-0.6.0-win-x64.zip)。支持 Windows 10 2004 及以上 x64 系统，无需另装 .NET。SHA-256 可在 GitHub 下载资产详情中查看，不附带单独校验文件。/ [Installer EXE](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Setup-0.6.0-win-x64.exe) or [portable ZIP](https://github.com/abnste/mewu_ai/releases/download/v0.6.0/MewuAI-Portable-0.6.0-win-x64.zip). Requires Windows 10 2004 or later, x64; no separate .NET installation is needed. SHA-256 digests are available in GitHub asset details; no separate checksum file is included.

完整历史仍保留在 [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.0/CHANGELOG.md)。/ The full history remains in [CHANGELOG.md](https://github.com/abnste/mewu_ai/blob/v0.6.0/CHANGELOG.md).
