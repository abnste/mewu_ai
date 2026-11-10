# MewuAI 1.2.2

- 修复开机自启动开关未及时保存，重启后恢复关闭的问题。
- 改善复制图片、识别文字、表格和视频时的剪贴板占用处理，避免旧请求覆盖新结果。
- 修复图片移动、缩放并继续绘制后，保存笔迹、贴图或收起会话可能受阻的问题。
- 优化中文、日文识别结果中的多余空格，保留英文词间距和表格列间距。
- 减少绘制历史处理和橡皮命中检测的重复计算。
- 长截图预览改为独立处理并限制排队数量，改善滚动时的响应和取消速度。
- 支持外部 Agent 返回明确的登录、额度及模型错误，失败状态不会被当成正常回答或自动重试。
- 改善 Linux 图形桌面切换时的截图、录屏授权恢复与会话接手，修复恢复媒体后截图界面持续加载的问题。
- 修复 Linux 重启后设备编号变化导致原数据目录被误判、软件无法启动的问题。

保留现有配置、会话与插件数据。

## Downloads

- Windows x64: installer or portable ZIP.
- macOS Apple Silicon: DMG; the app archive is used by automatic updates.
- Linux x64: AppImage.

## Changes

- Persist startup preferences immediately so they survive restarting the computer.
- Improve clipboard contention handling for images, recognized text, tables and videos, with stale request protection.
- Fix drawing saves, pinning and session minimization after repeated image transformations and annotations.
- Remove unnecessary CJK recognition spaces while retaining English word spacing and table column separation.
- Reduce repeated work in drawing history and eraser hit testing.
- Process scrolling capture previews separately with a bounded pending queue.
- Preserve classified external Agent failures, including login, quota and unavailable models, without treating them as answers or replaying requests.
- Improve Linux desktop-session handoff, capture and recording authorization recovery, and media restoration.
- Keep Linux data directories accessible when a reboot changes the filesystem device number.

Existing settings, conversations and plugin data are retained.
