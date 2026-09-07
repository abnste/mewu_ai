本次更新改善标注编辑与录屏导出，修复 Hermes 安装后启动、历史复制和高 DPI 菜单问题。

- 取色提示可见时按 C 复制当前像素的 RGB 色值，文字输入和文本复制保持原有操作。
- 移除独立移动工具。使用画笔、高亮或形状工具时，可直接点击已有对象并拖动；矩形、椭圆等支持四角缩放，箭头支持分别调整起点和终点，文字双击重新编辑。调整支持撤销、重做，编辑控制点不会进入导出图片。
- 区域录屏默认录制电脑声音，麦克风可单独开启；默认导出 MP4，也可选择 MP3 音频或 GIF 动图。GIF 格式本身不包含声音。
- 修复安装或更新后 Hermes 后台可能报“代码 1”的启动环境继承问题；修复更新关闭设置窗口时可能出现的窗口状态异常。
- 历史记录支持拖选、Ctrl+C 和右键复制完整内容；修复鼠标仍在对话条内时，因下方截图工具条重叠而误收纳的问题。
- 复用历史控件与取色缓存，限制后台框选探测并发，减少重复布局和鼠标移动时的多余工作。
- 修复高分辨率和高 DPI 下托盘右键菜单文字偏上；主页未配置可用 AI 时显示红色状态点，设置入口直接进入 AI 标签。
- 更新器优先使用 GitHub 安装包资产自带的 SHA-256。此迁移版本仍提供旧客户端自动升级所需的校验兼容文件；演示仅保留 README 内嵌 GIF，不再上传演示 MP4。
- 整理仓库与 Release 展示素材：移除源演示 MP4，保留 README 内嵌 GIF；补齐社区行为准则、贡献指南、安全策略和 Issue/PR 模板。

---

This update improves annotation editing and recording exports, and fixes Hermes startup after installation, history copying, and high-DPI tray menus.

- Press C while the color inspector is visible to copy the current pixel as RGB. Text entry and text copying retain their existing behavior.
- Remove the separate move tool. Click and drag existing annotations while using the pen, highlighter, or shape tools. Resize rectangles, ellipses, and other objects using corner handles, adjust either arrow endpoint, and double-click text to edit it. Adjustments support undo and redo; editing handles stay out of exported images.
- Region recordings capture computer audio by default, with an optional microphone. MP4 is the default export; MP3 audio and GIF animation are also available. GIF does not carry audio.
- Fix an inherited process environment that could make Hermes fail with code 1 after installation or an update. Fix a window-state exception when updates close Settings.
- Select and copy history text with Ctrl+C, or copy the full content from its context menu. Fix the conversation bar hiding while the pointer remains inside it above an overlapping capture toolbar.
- Reuse history controls and color-sampling caches, and bound background snapping probes to reduce repeated layout and unnecessary pointer-movement work.
- Correct vertically misaligned tray-menu text at high DPI. Show a red home-page status indicator when AI is unavailable, and open the AI tab directly from its settings entry.
- Prefer GitHub's native SHA-256 digest for installer verification. This migration release retains the checksum compatibility file needed by older clients for automatic upgrades. Demos remain embedded GIFs in the README; no demo MP4 is attached.
- Clean up repository and release presentation assets: remove the source demo MP4, keep the README GIF, and add community conduct, contribution, security, and issue/PR guidance.
