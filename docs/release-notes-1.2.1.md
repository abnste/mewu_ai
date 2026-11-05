# MewuAI 1.2.1

- 修复 Windows 长按 Win 唤起截图时，系统 AI 或开始菜单可能同时出现的问题；保留短按 Win 和普通 Win 组合键。
- 优化截图放大镜的鼠标取样，减少高频移动时的重复计算；复制颜色时仍使用最新位置。
- 修复更新检查读取失败后的恢复，以及并发请求导致更新状态错乱的问题。
- 修复 Linux 首次截图时，尚未创建悬浮会话窗口导致截图被错误拒绝的问题。

本次为维护更新，保留现有配置、会话与插件数据。Linux 界面回归使用 Ubuntu WSL 的 X11 测试桌面，不代表所有 Wayland 桌面环境均已重新验收。

## Downloads

- Windows x64: installer or portable ZIP.
- macOS Apple Silicon: DMG. The separate app archive is used by automatic updates.
- Linux x64: AppImage.

## Changes

- Fix long-held Windows-key capture activation leaking into the system AI or Start menu, while preserving short presses and regular Windows-key shortcuts.
- Coalesce magnifier sampling during pointer movement without using stale coordinates when copying a color.
- Recover update checks after read failures and prevent overlapping requests from overwriting newer state.
- Fix the first Linux capture being rejected when the floating session window has not yet been created.

Existing settings, conversations and plugin data are retained. Linux GUI regression used an Ubuntu WSL X11 test desktop; this does not certify every Wayland desktop environment.
