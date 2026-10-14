# MewuAI 1.0.0-preview.3

- Agent 与电子黑板作为官方可选插件，可启用、停用和卸载。全新安装未配置模型时仅显示截图；左下角不再提供新建对话按钮。
- 对话和设置界面按需加载。停用 Agent 会撤销正在运行的模型与工具任务，保留历史和记忆数据。
- 修复 Linux Wayland 冷启动快捷键恢复、绘制完成后的对话条聚焦、录屏到期保存和视频末尾重播。
- 保留截图绘制、系统声音录屏、裁剪、视频批注及导出等内置能力。
- 修复音频比视频稍长时整段批注导出失败；通用设置按平台显示可用选项。
- Windows 连续截图复用同屏全屏窗口，等待原图解码后呈现；高 DPI 下原始截图与标注共享像素比例。

Windows 提供安装版和便携版；macOS 提供 ARM64 DMG；Linux 提供 x64 AppImage。软件更新仍使用固定预览通道，数据目录和已有连接保留。0.7.3 用户应先升级到 0.7.4；旧版不会自动安装预览版。

Wayland 截图与录屏需要通过桌面授权对话框。GNOME Wayland 下录屏计时条暂不能保证始终置顶，框选包含计时条时可能录入画面。

Agent and blackboard are optional official plugins. A fresh installation starts with capture only until a model is configured. This preview adds Windows, macOS ARM64 and Linux x64 packages, retains the preview update channel, and fixes Wayland capture startup and recording/playback lifecycle issues. Full-length annotated exports retain the final video frame through the valid audio tail. General settings show only options available on the current platform. Repeated Windows captures reuse the fullscreen window on the same monitor and wait for the original image to decode before showing it. High-DPI screenshots and annotations share the same pixel scale.
