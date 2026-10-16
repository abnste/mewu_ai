# MewuAI 1.0.0

重制版首个正式发行，支持 Windows x64、macOS ARM64 和 Linux x64。使用 Rust、Tauri 2 和系统 WebView。

- 快捷键唤起截图空间，保留框选、绘制、复制、保存、置顶和内置录屏；未配置 AI 时也可使用。
- 在原对话条引用截图、视频和文件，支持多会话、Markdown、公式、可收纳的思考过程及可交互的 HTML/SVG/Canvas 对象。
- Agent 与电子黑板按需启用；API 2 可执行插件通过独立运行时扩展 Agent、长期记忆、工具和界面。[GitHub 社区市场](https://github.com/abnste/mewu-plugins)提供源码仓库、效果截图及真实插件发行包。
- 视频定位与裁切、上下工具条避让、对话条自动收纳、最近三色和像素/对象橡皮。

## 升级与数据

已有预览版可在「设置 → 关于」升级到 1.0.0，无需卸载重装。正式发行同时推进预览与正式更新入口；安装后的正式版继续使用正式通道。更新会验证安装包及其对应版本，数据目录保持原位置。

Windows 0.7.4 可在原关于页发现正式版，确认下载并安装；如未自动启动，请从原快捷方式打开。0.7.3 及更早版本应先安装 [0.7.4](https://github.com/abnste/mewu_ai/releases/tag/v0.7.4)。Windows 便携版的自动更新会运行安装器，切换到安装版位置。

支持的旧 Chat Completions、OpenAI Responses 和 Anthropic Messages API 连接会一次导入，包含对应的模型与本机加密密钥。自定义请求头、非系统代理及部分路由配置不自动迁移。**旧文件保留；旧历史对话尚未转换到新版历史界面，偏好设置与集成也并非全部迁移。** 更新前请备份重要数据。

## 当前范围

- 原生区域自动操作插件目前仅支持 Windows。macOS 和 Linux 可使用截图、绘制和录屏。
- Hermes、ChatGPT Work / Codex、WorkBuddy、MiniMax Code 桌面客户端适配、专用图片/视频生成接口和跨设备同步尚未包含。
- Linux Wayland 需要桌面截图与录屏授权。录屏授权后需重新框选区域；请将控制条拖到录制范围外，GNOME 暂不保证它始终置顶。
- 下载提供 Windows 安装版和便携 ZIP、macOS ARM64 DMG、Linux x64 AppImage。协议保持 MPL-2.0，安装包保留第三方许可。

## English

MewuAI's first stable remake release supports Windows x64, macOS ARM64 and Linux x64. It includes the capture workflow, executable API 2 plugins, video seek/trim controls, toolbar placement, conversation-bar hiding, three recent colors and pixel/object erasers. Agent and memory providers are independently extensible through the [GitHub community market](https://github.com/abnste/mewu-plugins).

Existing previews can upgrade in **Settings → About**. The stable release advances both update feeds; installed stable builds then follow the stable feed. Windows 0.7.4 can discover 1.0.0 in its original About page; earlier versions should first install 0.7.4. Portable updates run the installer and move to the installed application.

Supported Chat Completions, Responses and Messages API profiles, including their encrypted local credentials, are imported once. Custom headers, non-system proxy settings and some routing configurations need manual setup. **Legacy files remain, but old conversations are not converted into the new history UI; preferences and integrations are not fully migrated.** Back up important data before upgrading.

Native region automation remains Windows-only. Proprietary desktop-client adapters, dedicated image/video-generation APIs and cross-device sync are not included. Wayland capture/recording requires portal consent and a fresh region selection after recording approval. Move controls outside the recorded region; GNOME may not keep them on top. The project remains MPL-2.0 and includes third-party license notices.
