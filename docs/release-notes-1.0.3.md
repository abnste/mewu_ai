# MewuAI 1.0.3

- 新增默认启用的官方 CLI 插件，让其他本机 Agent 通过 `mewu` 操作现有桌面空间。支持截图、框选、绘制、撤销与重做、原图和带标注图导出、黑板、对话、录屏、长截图及已开放的插件能力。
- 使用 `mewu schema` 查看实际支持的方法、参数与结果。写操作提供独立完成回执；未知结果不自动重放。CLI 与对外调用工具的 MCP 配置独立，停用 CLI 插件不影响正常截图。
- 修复 Windows 本地管道返回较大内容时超时，保留请求大小限制、同用户身份核验、取消与退出排空。
- 修复 CLI 将黑板绘制模式误判为持续忙碌的问题；正在绘制、编辑文字、拖动素材和保存时仍保护用户操作。
- 改善 Agent 工具错误反馈、记忆来源核验与参数纠正，避免把无效参数当作成功执行。错误会回到当前工具循环，由模型决定下一步。
- 改善黑板对象移动与缩放后的显示稳定性，标注默认使用对象橡皮；文字对象支持调整宽高与自动换行，取色面板保留最近五种颜色。
- 修复生产构建误依赖开发服务器而使设置页空白的问题。

## 使用 CLI

安装后打开新终端，运行 `mewu help`、`mewu status` 或 `mewu schema`。已有终端和 Agent 进程可能需要重新启动才能获取新的 PATH。便携版可使用包内 `mewu.exe`；macOS 和 Linux 的入口由已安装软件进行用户级配置。

完整用法、平台条件与当前接口范围见 [CLI 文档](./cli.md)。目录中未列出的视频剪辑、导出等界面功能尚未开放为 CLI 方法。

继续通过「设置 → 关于」检查 GitHub 正式发行并校验更新，无额外公开更新清单或签名文件。保留现有数据目录和 API 配置。

## English

- Adds the default-enabled official CLI plugin so local agents can operate the existing desktop space, including capture, regions, drawing, undo/redo, image export, blackboards, conversations, recording and exposed plugin capabilities.
- Provides a machine-readable command catalog and completion receipts. Unknown outcomes are never automatically replayed. Local peer identity checks and request limits remain enforced.
- Fixes large Windows named-pipe responses and distinguishes an idle drawing mode from active human editing.
- Improves Agent tool-error feedback, memory-source validation and parameter correction, blackboard object stability, text resizing and recent colors.
- Prevents production builds from depending on the development server.

Open a new terminal after installation and run `mewu help`. See the [CLI documentation](./cli.md) for actual commands, platform requirements and limitations. Updates still use the latest stable GitHub Release with package and version verification, preserving existing data and API configuration.
