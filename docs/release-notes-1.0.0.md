# MewuAI 1.0.0

重制版首个正式发行，支持 Windows x64、macOS ARM64 和 Linux x64。使用 Rust、Tauri 2 和系统 WebView。

- 快捷键唤起截图空间，保留框选、绘制、复制、保存、置顶和内置录屏；未配置 AI 时也可使用。
- 在原对话条引用截图、视频和文件，支持多会话、Markdown、公式、可收纳的思考过程及可交互的 HTML/SVG/Canvas 对象。
- Agent 与电子黑板按需启用；API 2 可执行插件通过独立运行时扩展 Agent、长期记忆、工具和界面。[GitHub 社区市场](https://github.com/abnste/mewu-plugins)提供源码仓库、效果截图及真实插件发行包。
- 视频定位与裁切、上下工具条避让、对话条自动收纳、最近三色和像素/对象橡皮。
- 导入图片先成为空间对象；图片、视频和黑板预览可手动置顶。画面支持滚轮等比缩放，视频操作工具条左对齐，裁切条独立在下方，不再显示尺寸按钮。置顶视频保留真实播放、裁切范围和时段批注。
- 修复流式回复重复；视频批注参数错误会反馈给 AI 在原有轮次限制内修正，执行记录显示实际未执行原因。
- 修复对象缩放尚未保存时退出受阻的问题；退出时取消置顶操作，保存失败时保留待保存的几何。

## 升级与数据

本次修订保持版本号 1.0.0，正式版直接读取 GitHub 最新正式发行的 latest.json；Mac 更新归档也随版本发行，新的正式版与常规发布流程不再依赖 update-channel。原公钥、版本签名、平台与来源校验保持。

已有预览版可从原入口升级到修订后的 1.0.0。已经安装早期 1.0.0 的用户因版本号相同，需要从本发行页下载安装包重新安装一次，之后使用新的正式更新入口。无需先卸载，数据目录保持原位置。

Windows 0.7.4 可在原关于页发现正式版，确认下载并安装；如未自动启动，请从原快捷方式打开。0.7.3 及更早版本应先安装 [0.7.4](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4)。Windows 便携版的自动更新会运行安装器，切换到安装版位置。

支持的旧 Chat Completions、OpenAI Responses 和 Anthropic Messages API 连接会一次导入，包含对应的模型与本机加密密钥。自定义请求头、非系统代理及部分路由配置不自动迁移。**旧文件保留；旧历史对话尚未转换到新版历史界面，偏好设置与集成也并非全部迁移。** 更新前请备份重要数据。

## 当前范围

- 原生区域自动操作插件目前仅支持 Windows。macOS 和 Linux 可使用截图、绘制和录屏。
- Hermes、ChatGPT Work / Codex、WorkBuddy、MiniMax Code 桌面客户端适配、专用图片/视频生成接口和跨设备同步尚未包含。
- Linux Wayland 需要桌面截图与录屏授权。录屏授权后需重新框选区域；请将控制条拖到录制范围外，GNOME 暂不保证它始终置顶。
- 下载提供 Windows 安装版和便携 ZIP、macOS ARM64 DMG、Linux x64 AppImage。协议保持 MPL-2.0，安装包保留第三方许可。

## English

MewuAI's first stable remake release supports Windows x64, macOS ARM64 and Linux x64. It includes the capture workflow, executable API 2 plugins, video seek/trim controls, toolbar placement, conversation-bar hiding, three recent colors and pixel/object erasers. Agent and memory providers are independently extensible through the [GitHub community market](https://github.com/abnste/mewu-plugins).

Imported images first become space objects. Images, videos and board previews can be explicitly pinned, with proportional wheel zoom and no size button. Video actions align to the left of the picture and trimming remains below it. Pinned videos retain playback, the selected range and timed annotations. This revision also fixes repeated streamed replies and returns invalid annotation arguments to the AI for correction within the existing tool budget.

Pending media placement can finish saving during exit preparation. Canceling pin operations no longer blocks exit, and failed geometry saves remain available for retry.

The revised 1.0.0 reads latest.json from the latest stable GitHub release, and its macOS updater archive belongs to that versioned release. New builds and normal publishing do not depend on update-channel. Earlier previews can upgrade through their existing endpoint. Earlier 1.0.0 installations need one manual reinstall because this repair keeps the same version number. Windows 0.7.4 can discover 1.0.0 in its original About page; earlier versions should first install 0.7.4. Portable updates run the installer and move to the installed application.

Supported Chat Completions, Responses and Messages API profiles, including their encrypted local credentials, are imported once. Custom headers, non-system proxy settings and some routing configurations need manual setup. **Legacy files remain, but old conversations are not converted into the new history UI; preferences and integrations are not fully migrated.** Back up important data before upgrading.

Native region automation remains Windows-only. Proprietary desktop-client adapters, dedicated image/video-generation APIs and cross-device sync are not included. Wayland capture/recording requires portal consent and a fresh region selection after recording approval. Move controls outside the recorded region; GNOME may not keep them on top. The project remains MPL-2.0 and includes third-party license notices.
