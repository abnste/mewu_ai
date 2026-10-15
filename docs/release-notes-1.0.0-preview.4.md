# MewuAI 1.0.0-preview.4

- 新增 API 2 可执行插件：独立 JavaScript worker、版本化 SDK、权限与生命周期管理，第三方可注册 Agent、长期记忆、工具和交互界面。
- 插件市场接入 [GitHub 社区目录](https://github.com/abnste/mewu-plugins)，安装真实 Release ZIP，核验来源、版本、大小和 SHA-256。保留原声明插件的管理与停用功能。
- Agent 引擎与记忆提供方可独立选择。独立 Agent 不强制配置宿主模型；调用宿主模型时仍使用当前会话的连接。记忆按插件和 Agent 隔离持久保存。
- 插件可向现有截图工具条及左下角空间控件注册操作，生成可交互的 HTML/SVG/Canvas 悬浮对象，保留 Mewu 原有截图和绘制交互。
- 修复插件绑定刷新时 Agent 设置折叠、跳页，以及外部记忆与本地记忆设置混杂的问题。
- 修复区域任务结束时截图层迟到显示、关闭/冻结竞争及 Escape 取消问题，保护独立置顶图片的生命周期。

区域自动操作示例的原生输入适配目前仅支持 Windows，macOS 与 Linux 会明确报告不支持；这不影响两端原有截图、绘制和录屏。API 1 声明插件继续兼容。SDK 与插件包在社区仓库独立发行，不附加到软件安装包列表中。

Windows 提供 x64 安装版与便携版，macOS 提供 ARM64 DMG，Linux 提供 x64 AppImage。已有预览版使用固定预览更新通道；数据目录与原有连接保留。0.7.x 正式版不会自动切换到预览版。

Wayland 截图与录屏仍需要桌面授权。GNOME Wayland 下录屏计时条暂不能保证始终置顶，框选包含计时条时可能录入画面。

This preview adds executable API 2 extensions, a public GitHub community catalog and a development SDK. Independent plugins can provide Agent engines, persistent memory, tools and interactive space objects. Agent and memory providers can be selected separately; host credentials are required only when a plugin invokes the host model. Existing screenshot interactions and API 1 packages remain available.

The region automation input adapter currently supports Windows only. macOS and Linux capture, drawing and recording remain separate from that adapter. This preview also fixes Agent settings resetting during provider refresh and screenshot overlay lifecycle races during region operations. Windows x64, macOS ARM64 and Linux x64 retain the fixed preview update channel.
