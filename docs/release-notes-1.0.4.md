# MewuAI 1.0.4

- 插件可提供自己的隔离界面、拖放区域和持久对象库。工具包的格子、文件夹与垃圾桶，便笺的正文与引用列表，均由独立插件实现。
- 插件入口并入截图空间左下角的原工具条，使用各自图标。插件市场、已安装和原生功能分成三个标签；社区插件使用 GitHub 仓库与 Release 安装包。
- 增加通用浮动插件对象，支持移动、调整大小、置顶与关闭。便笺铺满对象，正文自动保存；Esc 先保存再关闭，保存失败保留编辑内容。
- 工具包图片收藏显示真实缩略图，拖动时显示跟随鼠标的预览。插件停用或卸载后撤销界面操作，保留插件数据供重新安装恢复。
- 统一插件 API，移除旧声明包的兼容入口。SDK 0.2.0 增加客户端通信、关闭前保存、对象事务与缩略图接口。
- 改善终端 Agent 对话排版，保留小猫；回答中 Ctrl+C 停止当前任务，等待输入时 Ctrl+C 退出终端对话。

更新继续使用 GitHub 正式发行，保留现有模型连接、API 配置和用户数据。插件的实际扩展范围及尚未接通的能力见[插件边界](./plugin-boundaries.md)；不支持的插件平台会明确显示。

## English

- Adds isolated plugin interfaces, bounded drag-and-drop, and persistent object libraries. Toolbox and notes behavior resides in independent plugin packages.
- Integrates plugin icons into the existing screenshot toolbar and separates the GitHub marketplace, installed community plugins, and native features.
- Adds movable, resizable, pinnable plugin objects. Notes fill their object and save before closing; failed saves retain the draft.
- Adds image thumbnails and visible drag previews to the toolbox. Disabling or removing a plugin revokes its interfaces while retaining its saved data.
- Unifies the plugin API and removes legacy declarative-package compatibility. SDK 0.2.0 adds client communication, before-close handlers, object transactions, and thumbnail access.
- Improves terminal conversation layout and Ctrl+C behavior while retaining the cat.

Updates continue through stable GitHub Releases and preserve existing connections, credentials, and user data. See [plugin boundaries](./plugin-boundaries.md) for supported extension points and remaining limitations.
