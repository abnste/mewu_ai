本次更新修复视频置顶闪退，新增可调色的 AI 思考光效，并统一回复复制菜单、补齐开源许可查看入口。

### 视频置顶

- 修复带标注视频置顶后可能导致程序闪退的问题，改善多个视频预览同时播放、缩放和关闭时的稳定性。
- 保留视频原件、音频和已有标注，不改变录制内容。

### AI 思考光效

- AI 请求进行时，屏幕底部显示浅色呼吸光效；完成、取消或失败后自动停止。
- 可在“设置 → 常规”中开关光效、自定义 RGB 颜色并即时预览。
- 光效延伸到完整屏幕底边，包含任务栏区域；对话输入条仍避开任务栏。
- 光效不抢输入焦点，不影响文字选择；系统关闭动画时显示静态浅色反馈。

### 回复复制与开源许可

- 历史回复与最新回复统一使用浅色圆角右键菜单，提供“复制所选文字”和“复制完整内容”。
- 修复菜单阴影被裁切的问题，打开复制菜单时保持对话条显示。
- 历史全文复制不受预览长度限制，最新回复复制包含已经收到的内容和表情。
- “设置 → 关于”新增“开源许可与第三方声明”，可离线查看第三方组件、版本、来源、完整许可原文和源码获取说明。

### 下载与升级

- 提供 Windows x64 安装 EXE 与便携 ZIP；安装包包含所需运行时与第三方许可证。使用 GitHub 下载文件详情中的 SHA-256 校验值，不再单独附带校验文件。
- 可在设置中检查更新；v0.2.5 及更早版本建议从发行页手动下载安装器升级。

[全部提交变化](https://github.com/abnste/mewu_ai/compare/v0.3.2...v0.3.3) · [完整版本记录](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)

---

This update fixes crashes when pinning videos, adds a customizable glow while AI is working, unifies reply copy menus, and makes bundled open-source notices accessible from About.

### Pinned videos

- Fixed a crash that could occur after pinning annotated videos, improving stability when multiple previews play, resize, or close.
- Original recordings, audio, and existing annotations are preserved.

### AI activity glow

- A soft breathing glow appears along the bottom of the screen during an AI request and stops on completion, cancellation, or failure.
- Enable or disable it, choose an RGB color, and preview the result in Settings → General.
- The glow reaches the full display edge, including the taskbar area, while the conversation input stays above the taskbar.
- The effect preserves input focus and text selection. A static glow is used when Windows animations are disabled.

### Reply copying and license notices

- Historical and latest replies now share the same rounded context menu with “Copy selected text” and “Copy full text.”
- Fixed clipped menu shadows and kept the conversation bar visible while its copy menu is open.
- Full-history copying includes text beyond the preview limit. Latest-reply copying includes received text and emoji.
- Settings → About now offers “Open-source licenses and third-party notices,” with offline access to component versions, sources, original license texts, and source-code information.

### Download and update

- Windows x64 installer EXE and portable ZIP are provided, including required runtimes and third-party licenses. Use the SHA-256 digest in each GitHub asset's details; no separate checksum file is included.
- Check for updates in Settings. Users on v0.2.5 or earlier should download the installer manually from the release page.

[Full comparison](https://github.com/abnste/mewu_ai/compare/v0.3.2...v0.3.3) · [Release history](https://github.com/abnste/mewu_ai/blob/master/CHANGELOG.md)
