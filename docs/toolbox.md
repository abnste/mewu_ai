# 对象工具包

实现位于 `extensions/official/object-toolbox`，包括全部 HTML、样式、拖放、八格布局、
文件夹、分页、重命名和垃圾桶规则。宿主没有 `Toolbox`、`ToolboxTarget` 或专用编辑命令。

宿主只提供 `space-overlay` 客户端、受控的对象拖放来源，以及按插件隔离的对象事务。
对照示例 `extensions/examples/research-notes` 使用同样的接口实现另一种可编辑列表布局。

两者当前是开发包，未自动安装到用户账户或上传市场。发布前需要完成独立包安装、
界面交互、保存重开、停用、卸载与重装验收。SDK 契约见 `packages/sdk/README.md`，
其他插件仍有的核心耦合见 `docs/plugin-boundaries.md`。
